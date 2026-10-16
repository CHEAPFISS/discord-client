#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use discord_client::{Channel, ChatMessage, DiscordState, GatewaySender, Guild};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tauri::{command, generate_context, generate_handler, AppHandle, Emitter, Manager, State};
use tokio::io::AsyncReadExt;

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
        let msg = msg.map_err(|e| e.to_string())?;
        if let Message::Text(text) = msg {
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if v["op"].as_u64() == Some(10) {
                break v["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
            }
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
                    Message::Ping(_) | Message::Pong(_) => continue,
                    _ => continue,
                };
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let op = v["op"].as_u64().unwrap_or(0);
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
            leave_voice_channel
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
