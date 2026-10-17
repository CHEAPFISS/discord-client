#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use discord_client::{Channel, ChatMessage, DiscordState, GatewaySender, Guild};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tauri::{command, generate_context, generate_handler, AppHandle, Emitter, Manager, State};
use tokio_tungstenite::tungstenite::Message;
use tokio::io::AsyncReadExt;
use tokio::net::UdpSocket;

const API_BASE: &str = "https://discord.com/api/v10";
const REDIRECT_URI: &str = "http://localhost:1436/callback";
const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

// --- Gateway WebSocket ---

/// Защита от повторного запуска: только одно соединение за раз.
static GATEWAY_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[command]
async fn connect_gateway(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
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

/// Бесконечный цикл соединения с ретраями.
async fn gateway_loop(
    app: AppHandle,
    token: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Result<(), String> {
    loop {
        match gateway_once(&app, &token, &mut rx).await {
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

async fn gateway_once(
    app: &AppHandle,
    token: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::Message;

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
    log::info!("Identify отправлен (tok={:.4}…)", &token[..4.min(token.len())]);

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
            if voice.session_id.is_some() && voice.token.is_some() && voice.endpoint.is_some() {
                let payload = json!({
                    "guild_id": voice.guild_id,
                    "channel_id": voice.channel_id,
                    "session_id": voice.session_id,
                    "token": voice.token,
                    "endpoint": voice.endpoint,
                });
                log::info!("Голосовое подключение собрано: {payload}");
                let _ = app.emit("voice_channel_ready", payload);
            }
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
                let payload = json!({
                    "guild_id": voice.guild_id,
                    "channel_id": voice.channel_id,
                    "session_id": voice.session_id,
                    "token": voice.token,
                    "endpoint": voice.endpoint,
                });
                log::info!("Голосовое подключение собрано: {payload}");
                let _ = app.emit("voice_channel_ready", payload);
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
fn logout(state: State<'_, DiscordState>) -> Result<(), String> {
    state.set_session(None)
}

/// Вход по user-токену Discord (полноценный доступ к REST API).
#[command]
async fn login_with_token(
    state: State<'_, DiscordState>,
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
    let guild_id = {
        let mut voice = state.voice.lock().unwrap();
        let guild = voice.as_ref().map(|v| v.guild_id.clone());
        *voice = None;
        guild
    };
    // сбрасываем флаг соединения, чтобы цикл heartbeat в UDP-задаче прервался
    {
        let mut conn = state.voice_conn.lock().unwrap();
        conn.connected = false;
        conn.secret_key = None;
        conn.ssrc = None;
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

/// Запрос на голосовое UDP-подключение (приходит после voice_channel_ready из Gateway).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceUdpConnectRequest {
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
    pub guild_id: String,
    pub channel_id: String,
}

/// Голосовое подключение к серверу Discord:
/// Voice WebSocket (identify → READY с ssrc) → UDP (IP Discovery → Select Protocol → Heartbeat).
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
    log::info!(
        "Voice connect: guild={} channel={} endpoint={}",
        guild_id, channel_id, endpoint
    );

    let app_clone = app.clone();
    let app_err = app.clone();
    let state_clone = DiscordState {
        session: state.session.clone(),
        voice: state.voice.clone(),
        voice_conn: state.voice_conn.clone(),
    };

    let req = VoiceUdpConnectRequest {
        session_id,
        token,
        endpoint,
        guild_id,
        channel_id,
    };

    tokio::spawn(async move {
        if let Err(e) = run_voice_udp(app_clone, state_clone, req).await {
            log::error!("Voice error: {e}");
            let _ = app_err.emit("voice_status", json!({"status": "error", "error": e}));
        }
    });

    Ok(())
}

async fn run_voice_udp(
    app: AppHandle,
    state: DiscordState,
    req: VoiceUdpConnectRequest,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::Message;

    log::info!(
        "Voice connect: guild={} channel={} endpoint={}",
        req.guild_id, req.channel_id, req.endpoint
    );

    let user_id = {
        let guard = state.session.lock().unwrap();
        guard.as_ref().ok_or("Не залогинен")?.user_id.clone()
    };

    // 1) Голосовой WebSocket (протокол v8)
    let voice_ws_url = format!("wss://{}/?v=8", req.endpoint);
    let (ws_stream, _) = tokio_tungstenite::connect_async(voice_ws_url.as_str())
        .await
        .map_err(|e| format!("Voice WebSocket connect: {e}"))?;
    let (mut vw_write, mut vw_read) = ws_stream.split();

    // 2) Hello (op 8) — heartbeat_interval
    let mut hb_ms: u64 = 45000;
    loop {
        let msg = vw_read.next().await.ok_or("Voice WS закрылся до Hello")?;
        if let Message::Text(text) = msg.map_err(|e| e.to_string())? {
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if v["op"].as_u64() == Some(8) {
                hb_ms = v["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
                log::debug!("Voice WS Hello: heartbeat {hb_ms}мс");
                break;
            }
        }
    }

    // 3) Identify (в v8 это op 0!)
    let identify = json!({
        "op": 0,
        "d": {
            "server_id": req.guild_id,
            "user_id": user_id,
            "session_id": req.session_id,
            "token": req.token,
            "max_dave_protocol_version": 0
        }
    });
    vw_write
        .send(Message::Text(identify.to_string()))
        .await
        .map_err(|e| format!("Voice identify send: {e}"))?;
    log::debug!("Voice identify (op 0) отправлен");

    let mut last_seq: i64 = -1;
    let mut hb = tokio::time::interval(std::time::Duration::from_millis(hb_ms));
    hb.tick().await; // первый tick мгновенный

    // Отправка heartbeat с seq_ack (v8)
    macro_rules! voice_hb {
        ($w:expr, $seq:expr) => {{
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let hbv = json!({"op": 3, "d": {"t": t, "seq_ack": $seq}});
            if $w.send(Message::Text(hbv.to_string())).await.is_err() {
                return Err("Voice heartbeat send failed".into());
            }
            log::debug!("Voice heartbeat отправлен");
        }};
    }

    // 4) Ждём Ready (op 2) — ssrc, ip, port, modes. Параллельно heartbeat и Speaking-события.
    let mut ready: Option<serde_json::Value> = None;
    while ready.is_none() {
        tokio::select! {
            _ = hb.tick() => {
                voice_hb!(vw_write, last_seq);
            }
            msg = vw_read.next() => {
                let msg = msg.ok_or("Voice WS закрылся (ожидание Ready)")?;
                match msg.map_err(|e| e.to_string())? {
                    Message::Text(text) => {
                        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                        if let Some(s) = v["seq"].as_u64() { last_seq = s as i64; }
                        let op = v["op"].as_u64().unwrap_or(999);
                        if op == 2 {
                            ready = Some(v["d"].clone());
                        } else if op == 0 && v["t"].as_str() == Some("SPEAKING") {
                            let _ = app.emit("voice_speaking", json!({
                                "user_id": v["d"]["user_id"],
                                "ssrc": v["d"]["ssrc"],
                                "speaking": v["d"]["speaking"],
                            }));
                        } else {
                            log::debug!("Voice recv op={op} t={}", v["t"].as_str().unwrap_or(""));
                        }
                    }
                    Message::Close(c) => return Err(format!("Voice WS закрылся: {c:?}")),
                    _ => {}
                }
            }
        }
    }
    let ready = ready.unwrap();
    let ssrc = ready["ssrc"].as_u64().unwrap_or(0) as u32;
    let v_ip = ready["ip"].as_str().unwrap_or("").to_string();
    let v_port = ready["port"].as_u64().unwrap_or(0) as u16;
    let modes: Vec<String> = ready["modes"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let chosen_mode = if modes.iter().any(|m| m == "aead_aes256_gcm_rtpsize") {
        "aead_aes256_gcm_rtpsize"
    } else {
        "aead_xchacha20_poly1305_rtpsize"
    };
    log::info!(
        "Voice Ready: ssrc={ssrc} udp={v_ip}:{v_port} mode={chosen_mode} modes={modes:?}"
    );

    // 5) UDP: подключаемся к ip:port из Ready (не к endpoint) + IP Discovery
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("UDP bind: {e}"))?;
    socket
        .connect(format!("{v_ip}:{v_port}").as_str())
        .await
        .map_err(|e| format!("UDP connect: {e}"))?;
    let local = socket.local_addr().map_err(|e| e.to_string())?;
    log::info!("Voice UDP bound {local}");

    // IP Discovery: Type(0x0001, 2б) + Length(70, 2б) + SSRC(4б) + Address(64б 0x00) + Port(2б 0x00)
    let mut disco = Vec::with_capacity(74);
    disco.extend_from_slice(&0x0001u16.to_be_bytes());
    disco.extend_from_slice(&70u16.to_be_bytes());
    disco.extend_from_slice(&ssrc.to_be_bytes());
    disco.extend_from_slice(&[0u8; 64]);
    disco.extend_from_slice(&0u16.to_be_bytes());
    socket.send(&disco).await.map_err(|e| e.to_string())?;
    log::debug!("IP Discovery отправлен (74 байта)");

    let mut buf = [0u8; 512];
    let n = socket.recv(&mut buf).await.map_err(|e| e.to_string())?;
    if n < 74 || buf[0] != 0x00 || buf[1] != 0x02 {
        return Err(format!("Неожиданный ответ IP Discovery: {n} байт, type={:02x}{:02x}", buf[0], buf[1]));
    }
    let ext_ssrc = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if ext_ssrc != ssrc {
        log::warn!("IP Discovery ssrc не совпал: {ext_ssrc} != {ssrc}");
    }
    let addr_end = buf[8..72].iter().position(|&b| b == 0).map(|p| 8 + p).unwrap_or(72);
    let ext_ip = String::from_utf8_lossy(&buf[8..addr_end]).to_string();
    let ext_port = u16::from_be_bytes([buf[72], buf[73]]);
    log::info!("IP Discovery: наш внешний адрес {ext_ip}:{ext_port}");

    // 6) Select Protocol (op 1) через WebSocket (не UDP!)
    let select = json!({
        "op": 1,
        "d": {
            "protocol": "udp",
            "data": {
                "address": ext_ip,
                "port": ext_port,
                "mode": chosen_mode
            }
        }
    });
    vw_write
        .send(Message::Text(select.to_string()))
        .await
        .map_err(|e| format!("Select Protocol send: {e}"))?;
    log::debug!("Select Protocol отправлен (mode={chosen_mode})");

    // 7) Session Description (op 4) — secret_key
    let mut secret_key: Option<Vec<u8>> = None;
    let mut dave_version: u64 = 0;
    while secret_key.is_none() {
        tokio::select! {
            _ = hb.tick() => {
                voice_hb!(vw_write, last_seq);
            }
            msg = vw_read.next() => {
                let msg = msg.ok_or("Voice WS закрылся (ожидание Session Description)")?;
                match msg.map_err(|e| e.to_string())? {
                    Message::Text(text) => {
                        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                        if let Some(s) = v["seq"].as_u64() { last_seq = s as i64; }
                        let op = v["op"].as_u64().unwrap_or(999);
                        if op == 4 {
                            let d = &v["d"];
                            secret_key = Some(
                                d["secret_key"]
                                    .as_array()
                                    .map(|a| a.iter().filter_map(|b| b.as_u64().map(|x| x as u8)).collect())
                                    .unwrap_or_default()
                            );
                            dave_version = d["dave_protocol_version"].as_u64().unwrap_or(0);
                            log::info!("Session Description получена (dave={dave_version})");
                        } else if op == 0 && v["t"].as_str() == Some("SPEAKING") {
                            let _ = app.emit("voice_speaking", json!({
                                "user_id": v["d"]["user_id"],
                                "ssrc": v["d"]["ssrc"],
                                "speaking": v["d"]["speaking"],
                            }));
                        } else {
                            log::debug!("Voice recv op={op}");
                        }
                    }
                    Message::Close(c) => return Err(format!("Voice WS закрылся: {c:?}")),
                    _ => {}
                }
            }
        }
    }
    let secret_key = secret_key.unwrap();
    if secret_key.len() != 32 {
        return Err(format!("secret_key неожиданной длины: {}", secret_key.len()));
    }
    log::info!(
        "Select Protocol OK: secret_key={}… ssrc={ssrc}",
        hex::encode(&secret_key[..4])
    );

    {
        let mut conn = state.voice_conn.lock().unwrap();
        conn.secret_key = Some(secret_key);
        conn.ssrc = Some(ssrc);
        conn.mode = Some(chosen_mode.to_string());
        conn.connected = true;
    }

    let _ = app.emit(
        "voice_channel_ready",
        json!({
            "guild_id": req.guild_id,
            "channel_id": req.channel_id,
            "ssrc": ssrc,
            "mode": chosen_mode,
            "udp": format!("{v_ip}:{v_port}"),
            "external": format!("{ext_ip}:{ext_port}"),
            "dave_protocol_version": dave_version,
        }),
    );
    let _ = app.emit("voice_status", json!({"status": "connected"}));

    // 8) Постоянный цикл: heartbeat по WS + Speaking-события + чтение UDP (аудио — Этап 3.2)
    loop {
        tokio::select! {
            _ = hb.tick() => {
                voice_hb!(vw_write, last_seq);
                let conn = state.voice_conn.lock().unwrap();
                if !conn.connected {
                    return Ok(());
                }
            }
            msg = vw_read.next() => {
                let msg = msg.ok_or("Voice WS закрылся (steady)")?;
                match msg.map_err(|e| e.to_string())? {
                    Message::Text(text) => {
                        let v: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if let Some(s) = v["seq"].as_u64() { last_seq = s as i64; }
                        let op = v["op"].as_u64().unwrap_or(999);
                        if op == 0 && v["t"].as_str() == Some("SPEAKING") {
                            let d = &v["d"];
                            let _ = app.emit("voice_speaking", json!({
                                "user_id": d["user_id"],
                                "ssrc": d["ssrc"],
                                "speaking": d["speaking"],
                            }));
                            log::debug!("Speaking: user={} flags={}", d["user_id"], d["speaking"]);
                        } else if op != 6 {
                            log::debug!("Voice recv op={op}");
                        }
                    }
                    Message::Close(c) => return Err(format!("Voice WS закрылся: {c:?}")),
                    _ => {}
                }
            }
            n = socket.recv(&mut buf) => {
                match n {
                    Ok(0) => continue,
                    Ok(_n) => { /* RTP-пакеты других участников — обработаем в Этапе 3.2 */ }
                    Err(e) => log::debug!("UDP recv: {e}"),
                }
            }
        }
    }
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
