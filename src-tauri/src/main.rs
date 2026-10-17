#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use discord_client::{Channel, ChatMessage, DiscordState, GatewaySender, Guild};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tauri::{command, generate_context, generate_handler, AppHandle, Emitter, Manager, State};
use tokio::io::AsyncReadExt;
use tokio_tungstenite::tungstenite::Message;

const API_BASE: &str = "https://discord.com/api/v10";
const REDIRECT_URI: &str = "http://localhost:1436/callback";
const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

// --- Gateway WebSocket ---

/// Защита от повторного запуска: только одно соединение за раз.
static GATEWAY_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Служебный payload для rx-канала gateway: «остановись окончательно» (смена аккаунта).
const GATEWAY_RELOG_MARKER: &str = "__relog__";

#[command]
async fn connect_gateway(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<String, String> {
    start_gateway_inner(app, clone_state(&state), &sender).await
}

async fn start_gateway_inner(
    app: AppHandle,
    state: DiscordState,
    sender: &GatewaySender,
) -> Result<String, String> {
    use std::sync::atomic::Ordering;

    if GATEWAY_RUNNING.swap(true, Ordering::SeqCst) {
        // уже запущен — считаем это успехом
        return Ok("already-running".into());
    }

    let token = state.token().ok_or("Не залогинен")?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    *sender.tx.lock().await = Some(tx);

    tokio::spawn(async move {
        let _ = gateway_loop(app.clone(), token, rx).await;
        GATEWAY_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok("started".into())
}

/// Смена идентичности: гасим голос (oto) и голосовой стейт под старым аккаунтом.
async fn reset_voice_on_identity_change(app: &AppHandle, state: &DiscordState) {
    let conn = {
        let mut slot = state.voice_conn.lock().unwrap();
        let conn = slot.conn.take();
        slot.session_id = None;
        slot.token = None;
        slot.pump_spawned = false;
        conn
    };
    state
        .voice_rejoin
        .store(0, std::sync::atomic::Ordering::SeqCst);
    *state.voice.lock().unwrap() = None;
    if let Some(conn) = conn {
        let _ = conn.shutdown().await;
    }
    let _ = app.emit("voice_status", json!({"status": "left"}));
}

/// Перезапуск gateway-цикла под НОВЫМ токеном: маркер старому loop'у → ожидание → старт.
async fn restart_gateway_for_new_token(
    app: &AppHandle,
    state: &DiscordState,
    sender: &GatewaySender,
) {
    // 1) просим активный loop выйти
    let marker = json!({"internal": GATEWAY_RELOG_MARKER});
    let _ = sender.send(marker).await;
    // 2) ждём его остановки (до ~5с; loop проверяет маркер в rx-ветке select-а)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while GATEWAY_RUNNING.load(std::sync::atomic::Ordering::SeqCst)
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // 3) заводим новый gateway с текущей (новой) сессией
    let app2 = app.clone();
    let state2 = state.clone();
    // sender: GatewaySender не Clone — живём через app.state::<GatewaySender>()
    tokio::spawn(async move {
        let app3 = app2.clone();
        let sender2 = app3.state::<GatewaySender>();
        let _ = start_gateway_inner(app2, state2, sender2.inner()).await;
    });
}

/// Бесконечный цикл соединения с ретраями.
async fn gateway_loop(
    app: AppHandle,
    token: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Result<(), String> {
    loop {
        match gateway_once(&app, &token, &mut rx).await {
            Ok(true) => {
                log::info!("Gateway: окончательная остановка (релогин)");
                return Ok(());
            }
            Ok(_) => {
                log::info!("Gateway: закрыто штатно, переподключение через 3с");
            }
            Err(e) => {
                log::warn!("Gateway: ошибка {e}, переподключение через 5с");
                let _ = app.emit(
                    "gateway_status",
                    json!({"status": "reconnecting", "error": e}),
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// true = остановиться окончательно (маркер релогина), false/Err = обычный reconnect-цикл.
async fn gateway_once(
    app: &AppHandle,
    token: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Result<bool, String> {
    
    let (ws, _) = tokio_tungstenite::connect_async(GATEWAY_URL)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let (mut write, mut read) = ws.split();

    // 1) жду HELLO (op 10) с heartbeat_interval
    let mut last_seq: Option<u64> = None;
    let heartbeat_ms = loop {
        let msg = read.next().await.ok_or("поток закрылся")?;
        let ping = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Ping(pay)) => {
                write.send(Message::Pong(pay)).await.ok();
                continue;
            }
            Ok(other) => {
                log::info!("Gateway до HELLO: {other:?}");
                continue;
            }
            Err(e) => return Err(format!("ws при HELLO: {e}")),
        };
        let v: serde_json::Value = serde_json::from_str(&ping).map_err(|e| e.to_string())?;
        if v["op"].as_u64() == Some(10) {
            break v["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
        }
    };

    // 2) Identify (user-токен, без intents)
    let identify = json!({
        "op": 2,
        "d": {
            "token": token,
            "properties": {"os": "linux", "browser": "discord-client", "device": "discord-client"},
            "compress": false
        }
    });
    write
        .send(Message::Text(identify.to_string()))
        .await
        .map_err(|e| e.to_string())?;

    let _ = app.emit("gateway_status", json!({"status": "connected"}));
    log::info!("Gateway: подключен, heartbeat {heartbeat_ms}мс");
    log::info!("Identify отправлен");

    // 3) heartbeat + приём событий
    let mut hb = tokio::time::interval(std::time::Duration::from_millis(heartbeat_ms));
    hb.tick().await; // первый tick срабатывает сразу

    loop {
        tokio::select! {
            _ = hb.tick() => {
                let hb = json!({"op": 1, "d": last_seq});
                if write.send(Message::Text(hb.to_string())).await.is_err() {
                    return Err("heartbeat send failed".into());
                }
            }
            out = rx.recv() => {
                match out {
                    Some(payload) => {
                        if payload.get("internal").and_then(|v| v.as_str()) == Some(GATEWAY_RELOG_MARKER) {
                            log::info!("Gateway: маркер релогина — останавливаю цикл");
                            return Ok(true);
                        }
                        if write.send(Message::Text(payload.to_string())).await.is_err() {
                            return Err("outgoing send failed".into());
                        }
                    }
                    None => { return Err("командный канал закрылся".into()); }
                }
            }
            msg = read.next() => {
                let msg = msg.ok_or("поток закрылся")?;
                let text = match msg.map_err(|e| e.to_string())? {
                    Message::Text(t) => t,
                    Message::Ping(pay) => {
                        log::debug!("WebSocket Ping, отвечаю Pong");
                        if write.send(Message::Pong(pay)).await.is_err() {
                            return Err("pong send failed".into());
                        }
                        continue;
                    }
                    Message::Pong(_) => continue,
                    Message::Close(c) => {
                        return Err(format!("gateway закрыл соединение: {c:?}"));
                    }
                    other => {
                        log::debug!("Gateway прочее: {other:?}");
                        continue;
                    }
                };
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let op = v["op"].as_u64().unwrap_or(0);
                let t = v["t"].as_str().unwrap_or("");
                log::info!("Gateway recv: op={op} t={t}");
                match op {
                    11 => { /* HEARTBEAT ACK */ }
                    7 | 9 => {
                        return Err(format!("gateway требует реконнект (op {op})"));
                    }
                    _ => {}
                }
                if let Some(s) = v["s"].as_u64() {
                    last_seq = Some(s);
                }
                let t = v["t"].as_str().unwrap_or("");
                match t {
                    "READY" => {
                        let _ = app.emit("gateway_status", json!({"status": "ready"}));
                    }
                    "MESSAGE_CREATE" => {
                        let d = &v["d"];
                        if d["type"].as_u64().unwrap_or(0) == 0 {
                            let payload = json!({
                                "id": d["id"],
                                "channel_id": d["channel_id"],
                                "author": d["author"]["username"],
                                "content": d["content"],
                                "timestamp": d["timestamp"],
                            });
                            let _ = app.emit("message_create", payload);
                        }
                    }
                    "VOICE_STATE_UPDATE" => {
                        handle_voice_state(app, &v["d"]).await;
                    }
                    "VOICE_SERVER_UPDATE" => {
                        handle_voice_server(app, &v["d"]).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Накапливает данные голосового подключения; когда все три есть — готово.
async fn handle_voice_state(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    let mut guard = state.voice.lock().unwrap();
    if let Some(voice) = guard.as_mut() {
        if d["guild_id"].as_str() == Some(voice.guild_id.as_str())
            && d["user_id"].as_str() == Some(voice.user_id.as_str())
        {
            let session_id = d["session_id"].as_str().unwrap_or("").to_string();
            if !session_id.is_empty() {
                voice.session_id = Some(session_id);
            }
            // Подключаемся только когда придёт свежий token (VOICE_SERVER_UPDATE) —
            // иначе используем протухший и получим 4014/4022.
        }
    }
}

async fn handle_voice_server(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    let mut guard = state.voice.lock().unwrap();
    if let Some(voice) = guard.as_mut() {
        if d["guild_id"].as_str() == Some(voice.guild_id.as_str()) {
            if let Some(token) = d["token"].as_str() {
                voice.token = Some(token.to_string());
            }
            if let Some(endpoint) = d["endpoint"].as_str() {
                voice.endpoint = Some(endpoint.trim_start_matches("wss://").to_string());
            }
            if voice.session_id.is_some() && voice.token.is_some() && voice.endpoint.is_some() {
                // Токен в webview не отдаём: фронт подключение не выполняет.
                let payload = json!({
                    "guild_id": voice.guild_id,
                    "channel_id": voice.channel_id,
                    "session_id": voice.session_id,
                    "endpoint": voice.endpoint,
                });
                log::info!(
                    "Голосовое подключение собрано: guild={} channel={} session={} token=<redacted>",
                    voice.guild_id, voice.channel_id, voice.session_id.clone().unwrap_or_default()
                );
                let _ = app.emit("voice_channel_ready", payload);

                // Авто-подключение из бэкенда: свежий токен одноразовый, тратим сразу.
                let app2 = app.clone();
                let discord_state = clone_state(&state);
                let guild_ids = voice.guild_id.clone();
                let channel_ids = voice.channel_id.clone();
                let session_id = voice.session_id.clone().unwrap();
                let token = voice.token.clone().unwrap();
                let endpoint = voice.endpoint.clone().unwrap();
                tokio::spawn(async move {
                    if let Err(e) = connect_voice(
                        app2, discord_state, guild_ids, channel_ids, session_id, token, endpoint,
                    ).await {
                        log::error!("Voice error: {e}");
                    }
                });
            }
        }
    }
}

// --- История и отправка сообщений ---

#[command]
async fn get_messages(
    state: State<'_, DiscordState>,
    channel_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(format!("{API_BASE}/channels/{channel_id}/messages"))
        .query(&[("limit", "50")])
        .header("Authorization", token)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса истории: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора истории: {e}"))?;

    let mut out = Vec::new();
    if let Some(arr) = resp.as_array() {
        for m in arr {
            out.push(ChatMessage {
                id: m["id"].as_str().unwrap_or("").to_string(),
                channel_id: channel_id.clone(),
                author: m["author"]["username"].as_str().unwrap_or("").to_string(),
                content: m["content"].as_str().unwrap_or("").to_string(),
                timestamp: m["timestamp"].as_str().unwrap_or("").to_string(),
            });
        }
    } else {
        return Err(format!(
            "API вернул ошибку: {} (code {})",
            resp["message"].as_str().unwrap_or("неизвестно"),
            resp["code"].as_u64().unwrap_or(0)
        ));
    }
    // API возвращает новые→старые; реверсим для отображения
    out.reverse();
    Ok(out)
}

#[command]
async fn send_message(
    state: State<'_, DiscordState>,
    channel_id: String,
    content: String,
) -> Result<ChatMessage, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!("{API_BASE}/channels/{channel_id}/messages"))
        .header("Authorization", token)
        .json(&json!({"content": content}))
        .send()
        .await
        .map_err(|e| format!("Ошибка отправки: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора ответа отправки: {e}"))?;

    Ok(ChatMessage {
        id: resp["id"].as_str().unwrap_or("").to_string(),
        channel_id,
        author: resp["author"]["username"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        content: resp["content"].as_str().unwrap_or("").to_string(),
        timestamp: resp["timestamp"].as_str().unwrap_or("").to_string(),
    })
}

// --- OAuth ---

#[derive(Debug, Deserialize)]
struct OAuthConfig {
    client_id: String,
    client_secret: String,
}

fn read_oauth_config() -> Result<OAuthConfig, String> {
    let path = discord_client::app_config_dir().join("config.json");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "Не найден конфиг {}. Создай его с содержимым:\n{{\"client_id\": \"ТВОЙ_CLIENT_ID\", \"client_secret\": \"ТВОЙ_CLIENT_SECRET\"}}\n(id/secret из Discord Developer Portal → твоё приложение → OAuth2)",
            path.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|e| format!("Ошибка разбора {}: {e}", path.display()))
}

#[command]
fn get_me(state: State<'_, DiscordState>) -> Result<serde_json::Value, String> {
    let guard = state.session.lock().unwrap();
    let s = guard.as_ref().ok_or("Не залогинен")?;
    Ok(serde_json::json!({
        "user_id": s.user_id,
        "username": s.username
    }))
}

#[command]
async fn logout(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<(), String> {
    let st = clone_state(&state);
    reset_voice_on_identity_change(&app, &st).await;
    // останавливаем gateway-цикл (новый стартует после следующего логина)
    let _ = sender.send(json!({"internal": GATEWAY_RELOG_MARKER})).await;
    state.set_session(None)
}

/// Вход по user-токену Discord (полноценный доступ к REST API).
#[command]
async fn login_with_token(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
    token: String,
) -> Result<serde_json::Value, String> {
    let token = token.trim().trim_matches('"').to_string();
    if token.is_empty() {
        return Err("Пустой токен".into());
    }

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(format!("{API_BASE}/users/@me"))
        .header("Authorization", &token)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора: {e}"))?;

    if resp["code"].is_number() || resp["id"].is_null() {
        return Err(format!(
            "Токен не принят Discord: {} (code {})",
            resp["message"].as_str().unwrap_or("неизвестно"),
            resp["code"].as_u64().unwrap_or(0)
        ));
    }

    let session = discord_client::DiscordSession {
        token,
        user_id: resp["id"].as_str().unwrap_or("").to_string(),
        username: resp["username"].as_str().unwrap_or("").to_string(),
    };
    state.set_session(Some(session.clone()))?;

    // Смена идентичности: гасим голос и перезапускаем gateway под новым токеном.
    let st = clone_state(&state);
    reset_voice_on_identity_change(&app, &st).await;
    restart_gateway_for_new_token(&app, &st, &sender).await;

    Ok(serde_json::json!({
        "user_id": session.user_id,
        "username": session.username
    }))
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[command]
async fn start_login(state: State<'_, DiscordState>) -> Result<serde_json::Value, String> {
    let cfg = read_oauth_config()?;

    // 1. Открываем браузер для авторизации
    let auth_url = format!(
        "https://discord.com/oauth2/authorize?client_id={}&response_type=code&redirect_uri={}&scope=identify%20guilds",
        cfg.client_id,
        REDIRECT_URI.replace(':', "%3A").replace('/', "%2F")
    );

    let listener: tokio::net::TcpListener = tokio::net::TcpListener::bind("127.0.0.1:1436")
        .await
        .map_err(|e| format!("Порт 1436 занят: {e}"))?;

    std::process::Command::new("xdg-open")
        .arg(&auth_url)
        .spawn()
        .map_err(|e| format!("Не удалось открыть браузер: {e}"))?;

    let (code, _stream) = wait_for_code_on(listener).await?;

    // 2. Обмен кода на токен
    let client = reqwest::Client::new();
    let params = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", REDIRECT_URI),
    ];

    let token_resp: TokenResponse = client
        .post(format!("{API_BASE}/oauth2/token"))
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Ошибка HTTP при обмене кода: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Не удалось разобрать токен-ответ: {e}"))?;

    // 3. Информация о пользователе
    #[derive(Debug, Deserialize)]
    struct UserInfo {
        id: String,
        username: String,
    }

    let user: UserInfo = client
        .get(format!("{API_BASE}/users/@me"))
        .bearer_auth(&token_resp.access_token)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса пользователя: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Не удалось разобрать пользователя: {e}"))?;

    // 4. Сохраняем сессию
    let session = discord_client::DiscordSession {
        token: token_resp.access_token,
        user_id: user.id,
        username: user.username,
    };
    state.set_session(Some(session.clone()))?;

    Ok(serde_json::json!({
        "user_id": session.user_id,
        "username": session.username,
    }))
}

async fn wait_for_code_on(listener: tokio::net::TcpListener) -> Result<(String, ()), String> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let first_line = req.lines().next().unwrap_or("");

        use tokio::io::AsyncWriteExt;
        let response_body = if first_line.contains("code=") {
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body style='font-family:sans-serif;background:#36393f;color:#fff;text-align:center;padding-top:80px'><h2>Вход выполнен!</h2><p>Вернитесь в приложение.</p></body></html>"
        } else {
            "HTTP/1.1 404 Not Found\r\n\r\n<!doctype html><html><body>listening for callback...</body></html>\r\n"
        };
        let _ = stream.write_all(response_body.as_bytes()).await;

        if let Some(pos) = first_line.find("code=") {
            let rest = &first_line[pos + 5..];
            let end = rest.find(['&', ' ']).unwrap_or(rest.len());
            return Ok((rest[..end].to_string(), ()));
        }
    }
}

// --- REST Discord ---

#[command]
async fn get_guilds(state: State<'_, DiscordState>) -> Result<Vec<Guild>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(format!("{API_BASE}/users/@me/guilds"))
        .header("Authorization", token)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса серверов: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора списка серверов: {e}"))?;

    let mut guilds = Vec::new();
    if let Some(arr) = resp.as_array() {
        for g in arr {
            guilds.push(Guild {
                id: g["id"].as_str().unwrap_or("").to_string(),
                name: g["name"].as_str().unwrap_or("").to_string(),
                icon: g["icon"].as_str().map(String::from),
            });
        }
    } else {
        return Err(format!(
            "API вернул ошибку: {} (code {})",
            resp["message"].as_str().unwrap_or("неизвестно"),
            resp["code"].as_u64().unwrap_or(0)
        ));
    }
    Ok(guilds)
}

#[command]
async fn get_channels(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<Vec<Channel>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(format!("{API_BASE}/guilds/{guild_id}/channels"))
        .header("Authorization", token)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса каналов: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора списка каналов: {e}"))?;

    let mut channels = Vec::new();
    if let Some(arr) = resp.as_array() {
        for c in arr {
            channels.push(Channel {
                id: c["id"].as_str().unwrap_or("").to_string(),
                guild_id: guild_id.clone(),
                name: c["name"].as_str().unwrap_or("").to_string(),
                kind: c["type"].as_u64().unwrap_or(0) as u8,
            });
        }
    } else {
        return Err(format!(
            "API вернул ошибку: {} (code {})",
            resp["message"].as_str().unwrap_or("неизвестно"),
            resp["code"].as_u64().unwrap_or(0)
        ));
    }
    Ok(channels)
}

#[command]
async fn join_voice_channel(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
    channel_id: String,
    guild_id: String,
) -> Result<(), String> {
    // user_id берём из сессии
    let user_id = {
        let guard = state.session.lock().unwrap();
        guard.as_ref().ok_or("Не залогинен")?.user_id.clone()
    };

    // фиксируем цель голосового подключения
    state.voice_rejoin.store(0, std::sync::atomic::Ordering::SeqCst);
    {
        let mut voice = state.voice.lock().unwrap();
        *voice = Some(discord_client::VoiceData {
            guild_id: guild_id.clone(),
            channel_id: channel_id.clone(),
            user_id: user_id.clone(),
            session_id: None,
            token: None,
            endpoint: None,
        });
    }

    // op 4 — обновить голосовое состояние (Discord пришлёт VOICE_SERVER_UPDATE/STATE)
    let op4 = json!({
        "op": 4,
        "d": {
            "guild_id": guild_id,
            "channel_id": channel_id,
            "self_mute": false,
            "self_deaf": false
        }
    });
    sender.send(op4).await?;

    // уведомление UI
    let _ = app.emit("voice_status", json!({"status": "joining"}));
    Ok(())
}

#[command]
async fn leave_voice_channel(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<(), String> {
    // Сериализация с in-flight connect: иначе коннект успеет "воскресить" голос после выхода.
    let _guard = state.voice_connect_lock.lock().await;
    let guild_id = {
        let mut voice = state.voice.lock().unwrap();
        let guild = voice.as_ref().map(|v| v.guild_id.clone());
        *voice = None;
        guild
    };
    // Единая секция: conn + пара + флаг насоса согласованно, без окна между локами.
    let active_conn = {
        let mut slot = state.voice_conn.lock().unwrap();
        let conn = slot.conn.take();
        slot.session_id = None;
        slot.token = None;
        slot.pump_spawned = false;
        conn
    };
    state.voice_rejoin.store(0, std::sync::atomic::Ordering::SeqCst);
    if let Some(conn) = active_conn {
        let _ = conn.shutdown().await;
    }
    if let Some(guild_id) = guild_id {
        let op4 = json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": null,
                "self_mute": false,
                "self_deaf": false
            }
        });
        sender.send(op4).await.ok();
    }
    let _ = app.emit("voice_status", json!({"status": "left"}));
    Ok(())
}

/// Голосовое подключение к серверу Discord:
/// Voice WebSocket (identify → READY с ssrc) → UDP (IP Discovery → Select Protocol → Heartbeat).
/// Повторный op 4 join текущего канала: Discord выдаст свежую пару (session, token).
/// Нужно после 4014/4022 NeedsFreshVoiceInfo — токен одноразовый и мог быть потрачен.
const VOICE_REJOIN_LIMIT: u64 = 3;

fn request_voice_rejoin(app: &AppHandle, state: &DiscordState) {
    use std::sync::atomic::Ordering;
    let attempts = state.voice_rejoin.fetch_add(1, Ordering::SeqCst) + 1;
    if attempts > VOICE_REJOIN_LIMIT {
        log::warn!("Voice: re-join не запрошен — исчерпан лимит ({VOICE_REJOIN_LIMIT})");
        let _ = app.emit(
            "voice_status",
            json!({"status": "error", "error": "не получил свежую voice-пару после 3 попыток"}),
        );
        return;
    }
    let target = state
        .voice
        .lock()
        .unwrap()
        .as_ref()
        .map(|v| (v.guild_id.clone(), v.channel_id.clone()));
    let Some((guild_id, channel_id)) = target else { return };
    log::info!("Voice: запрашиваю свежую voice-пару (re-join #{attempts})");
    let app = app.clone();
    tokio::spawn(async move {
        let sender = app.state::<GatewaySender>();
        let op4 = json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": channel_id,
                "self_mute": false,
                "self_deaf": false
            }
        });
        if let Err(e) = sender.send(op4).await {
            log::warn!("Voice re-join send: {e}");
        }
    });
}

/// Клоны Arc-полей состояния для spawned задач.
fn clone_state(state: &State<'_, DiscordState>) -> DiscordState {
    DiscordState {
        session: state.session.clone(),
        voice: state.voice.clone(),
        voice_conn: state.voice_conn.clone(),
        voice_connect_lock: state.voice_connect_lock.clone(),
        voice_rejoin: state.voice_rejoin.clone(),
    }
}

/// Команда для совместимости (старый фронт) — реальная работа в connect_voice.
#[command]
async fn voice_udp_connect(
    app: AppHandle,
    state: State<'_, DiscordState>,
    session_id: String,
    token: String,
    endpoint: String,
    guild_id: String,
    channel_id: String,
) -> Result<(), String> {
    let st = clone_state(&state);
    connect_voice(app, st, guild_id, channel_id, session_id, token, endpoint).await
}

/// Голосовое подключение Discord через oto: Voice Gateway v8 + DAVE (E2EE) + RTP.
///
/// Правила управления соединением:
/// • Токен Discord одноразовый → сериализуем подключение локом.
/// • Пара (session, token) идентифицирует попытку → дедуп по ней.
/// • Смена канала в том же guild НЕ рвёт WS: живому соединению отдаём свежую пару
///   через replace_voice_info (иначе Discord шлёт каскад 4022/4014 за рструкция).
async fn connect_voice(
    app: AppHandle,
    state: DiscordState,
    guild_ids: String,
    channel_ids: String,
    session_id: String,
    token: String,
    endpoint_in: String,
) -> Result<(), String> {
    let user_id = {
        let guard = state.session.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Не залогинен")?
            .user_id
            .clone()
            .parse::<u64>()
            .map_err(|e| format!("user_id: {e}"))?
    };
    let guild = guild_ids.parse::<u64>().map_err(|e| format!("guild_id: {e}"))?;
    let channel = channel_ids.parse::<u64>().map_err(|e| format!("channel_id: {e}"))?;
    let endpoint = endpoint_in.trim().trim_start_matches("wss://").to_string();

    // Цель могла смениться/сняться, пока задача стояла в очереди (leave, новый join,
    // смена аккаунта): неактуальная попытка не тратит одноразовый токен и не воскрешает голос.
    {
        let voice = state.voice.lock().unwrap();
        match voice.as_ref() {
            None => {
                log::info!("Voice connect отменён: цель снята (leave/смена аккаунта)");
                return Ok(());
            }
            Some(v) if v.channel_id != channel_ids => {
                log::info!("Voice connect отменён: цель сменилась на канал {}", v.channel_id);
                return Ok(());
            }
            _ => {}
        }
    }

    let _guard = state.voice_connect_lock.lock().await;

    // Дедуп: пара (session, token) уже применена к не-терминальному соединению.
    let dup = {
        let slot = state.voice_conn.lock().unwrap();
        match slot.conn.as_ref() {
            Some(c) => {
                let ph = c.state().phase();
                let terminal =
                    matches!(ph, oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed);
                !terminal
                    && slot.session_id.as_deref() == Some(session_id.as_str())
                    && slot.token.as_deref() == Some(token.as_str())
            }
            None => false,
        }
    };
    if dup {
        log::debug!("Voice: пара (session, token) уже применена, дедуп-пропуск");
        return Ok(());
    }

    log::info!(
        "Voice connect (oto): guild={guild} channel={channel} session={} endpoint={endpoint}",
        &session_id[..session_id.len().min(8)]
    );

    let info = oto::VoiceConnectInfo::new(
        guild,
        user_id,
        channel,
        session_id.clone(),
        endpoint,
        oto::VoiceToken::new(token.clone()),
    );

    // Запоминаем пару оптимистично (эхо-вызовы не будут повторять в meanwhile).
    let (prev, prev_alive) = {
        let mut slot = state.voice_conn.lock().unwrap();
        slot.session_id = Some(session_id.clone());
        slot.token = Some(token.clone());
        let prev = slot.conn.take();
        let alive = prev
            .as_ref()
            .is_some_and(|p| {
                !matches!(
                    p.state().phase(),
                    oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed
                )
            });
        if !alive {
            slot.pump_spawned = false; // мёртвое/новое соединение — насос заводим заново
        }
        (prev, alive)
    };

    let conn = match prev {
        Some(p) if prev_alive => {
            // Живое соединение: не рвём WS, отдаём свежую пару на месте.
            let ph = p.state().phase();
            match p.replace_voice_info(info).await {
                Ok(generation) => {
                    log::info!(
                        "Voice (oto) replace_voice_info OK (phase={ph:?}, gen={})",
                        generation.get()
                    );
                    p
                }
                Err(e) => {
                    state.voice_conn.lock().unwrap().session_id = None;
                    state.voice_conn.lock().unwrap().token = None;
                    let _ = p.shutdown().await;
                    if matches!(e.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&app, &state);
                    }
                    return Err(format!("oto replace_voice_info: {e}"));
                }
            }
        }
        Some(p) => {
            // Мёртвое соединение — закрываем и подключаемся начисто.
            let _ = p.shutdown().await;
            let oto = oto::Oto::builder().build().map_err(|e| format!("oto init: {e}"))?;
            match oto.connect(info).await {
                Ok(c) => c,
                Err(e) => {
                    state.voice_conn.lock().unwrap().session_id = None;
                    state.voice_conn.lock().unwrap().token = None;
                    if matches!(e.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&app, &state);
                    }
                    return Err(format!("oto connect: {e}"));
                }
            }
        }
        None => {
            let oto = oto::Oto::builder().build().map_err(|e| format!("oto init: {e}"))?;
            match oto.connect(info).await {
                Ok(c) => c,
                Err(e) => {
                    state.voice_conn.lock().unwrap().session_id = None;
                    state.voice_conn.lock().unwrap().token = None;
                    if matches!(e.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&app, &state);
                    }
                    return Err(format!("oto connect: {e}"));
                }
            }
        }
    };

    state.voice_rejoin.store(0, std::sync::atomic::Ordering::SeqCst);

    let phase = conn.state().phase();
    log::info!("Voice (oto) готово: phase={phase:?} rtt={:?}", conn.state().gateway_rtt());

    state.voice_conn.lock().unwrap().conn = Some(conn.clone());
    let _ = app.emit("voice_status", json!({"status": "connected"}));

    // Насос событий: ОДИН на соединение. Повторные replace_voice_info обслуживает
    // уже живущий насос — иначе плодим подписчиков (лимит oto: 8) и дублируем события.
    let need_pump = {
        let mut slot = state.voice_conn.lock().unwrap();
        let need = !slot.pump_spawned;
        slot.pump_spawned = true;
        need
    };
    if need_pump {
    let pump_app = app.clone();
    let pump_state = state.clone();
    let slot_ref = state.voice_conn.clone();
    let mut sub = conn
        .subscribe_events()
        .map_err(|e| format!("oto subscribe: {e}"))?;
    tokio::spawn(async move {
        loop {
            let event = match sub.recv().await {
                Ok(ev) => ev,
                Err(oto::EventReceiveError::Lagged { skipped }) => {
                    // Переполнение кольца — не смертельно, продолжаем с новыми.
                    log::warn!("Voice (oto) насос отстал: пропущено {skipped} событий");
                    continue;
                }
                Err(_) => break,
            };
            // Актуальное состояние слота на момент прихода события.
            let (cur_gen, cur_phase) = {
                let slot = slot_ref.lock().unwrap();
                match slot.conn.as_ref() {
                    Some(c) => {
                        let s = c.state();
                        (Some(s.generation()), Some(s.phase()))
                    }
                    None => (None, None),
                }
            };
            let ev_gen = match &event {
                oto::ConnectionEvent::StateChanged { generation, .. } => Some(*generation),
                oto::ConnectionEvent::Failure(f) => Some(f.generation()),
                _ => None,
            };
            if let Some(g) = ev_gen {
                if Some(g) != cur_gen {
                    continue; // поколение уже заменено — событие устарело
                }
            } else if matches!(event, oto::ConnectionEvent::Closed(_))
                && cur_phase != Some(oto::ConnectionPhase::Closed)
            {
                continue; // Closed старого соединения — в слоте уже другое/живое
            }
            match event {
                oto::ConnectionEvent::StateChanged { phase, .. } => {
                    log::debug!("Voice (oto) phase: {phase:?}");
                    let (status, err) = match phase {
                        oto::ConnectionPhase::Connected => ("connected", None),
                        oto::ConnectionPhase::Resuming
                        | oto::ConnectionPhase::Reconnecting => ("reconnecting", None),
                        oto::ConnectionPhase::NeedsFreshVoiceInfo => ("needs_fresh", None),
                        oto::ConnectionPhase::Closed => ("left", None),
                        oto::ConnectionPhase::Failed => ("error", Some("voice connection failed".to_string())),
                        _ => ("connecting", None),
                    };
                    let payload = match err {
                        Some(e) => json!({"status": "error", "error": e}),
                        None => json!({"status": status}),
                    };
                    let _ = pump_app.emit("voice_status", payload);
                }
                oto::ConnectionEvent::Failure(f) => {
                    log::warn!(
                        "Voice (oto) failure: op={:?} kind={:?} code={:?} retry={:?}",
                        f.operation(),
                        f.kind(),
                        f.safe_code(),
                        f.retry_disposition()
                    );
                    // NeedsFreshVoiceInfo mid-session: соединение ждёт свежую пару —
                    // сами запрашиваем re-join (op 4), иначе голос висит навсегда.
                    if matches!(f.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&pump_app, &pump_state);
                        let _ = pump_app.emit("voice_status", json!({"status": "joining"}));
                    } else {
                        let _ = pump_app.emit(
                            "voice_status",
                            json!({"status": "error", "error": format!("{:?}/{:?}", f.operation(), f.kind())}),
                        );
                    }
                }
                oto::ConnectionEvent::Closed(_) => {
                    let _ = pump_app.emit("voice_status", json!({"status": "left"}));
                    break;
                }
                _ => {}
            }
        }
        });
    } // need_pump

    Ok(())
}

// --- Точка входа ---

fn main() {
    env_logger::init();

    // Явно фиксируем криптопровайдер rustls: иначе первое WS-подключение паникует,
    // если в дереве зависимостей включены и ring, и aws-lc-rs (неоднозначность).
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    tauri::Builder::default()
        .manage(DiscordState::load_or_default())
        .manage(GatewaySender::new())
        .invoke_handler(generate_handler![
            start_login,
            login_with_token,
            get_me,
            logout,
            get_guilds,
            get_channels,
            get_messages,
            send_message,
            connect_gateway,
            join_voice_channel,
            leave_voice_channel,
            voice_udp_connect
        ])
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            Ok(())
        })
        .run(generate_context!())
        .expect("error while running tauri application");
}
