#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use discord_client::{Channel, DiscordState, Guild};
use serde::Deserialize;
use tauri::{command, generate_context, generate_handler, AppHandle, Manager, State};
use tokio::io::AsyncReadExt;

const API_BASE: &str = "https://discord.com/api/v10";
const REDIRECT_URI: &str = "http://localhost:1436/callback";

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

async fn wait_for_code_on(
    listener: tokio::net::TcpListener,
) -> Result<(String, ()), String> {
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
        .bearer_auth(token)
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
        .bearer_auth(token)
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
    }
    Ok(channels)
}

#[command]
async fn join_voice_channel(
    _app: AppHandle,
    _state: State<'_, DiscordState>,
    channel_id: String,
    guild_id: String,
) -> Result<(), String> {
    // TODO: подключение к голосовому каналу через WebRTC мост
    let _ = (channel_id, guild_id);
    Err("Голос ещё не реализован".into())
}

// --- Точка входа ---

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .manage(DiscordState::load_or_default())
        .invoke_handler(generate_handler![
            start_login,
            get_me,
            get_guilds,
            get_channels,
            join_voice_channel
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
