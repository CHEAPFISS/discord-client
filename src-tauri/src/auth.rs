use serde::Deserialize;
use tauri::{command, AppHandle, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use discord_client::{DiscordState, GatewaySender};
use crate::gateway::{
    clone_state, request_gateway_stop, reset_voice_on_identity_change,
    restart_gateway_for_new_token,
};

/// Адрес callback-сервера OAuth2.
pub(crate) const REDIRECT_URI: &str = "http://localhost:1436/callback";

/// Ответ обмена кода на токен.
#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: String,
}

/// Вход по user-токену Discord (полноценный доступ к REST API).
#[command]
pub(crate) async fn login_with_token(
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
        .get(format!("{}/users/@me", crate::api::API_BASE))
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

/// Случайный nonce для OAuth state (16 байт энтропии).
pub(crate) fn random_oauth_state() -> String {
    use std::io::Read;
    let mut buf = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        let mut acc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
            ^ (std::process::id() as u64);
        for b in buf.iter_mut() {
            acc = acc
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (acc >> 33) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Ждём callback с code= и state=; чужой/без state — запрос игнорируется (жмём дальше).
pub(crate) async fn wait_for_code_on(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, String> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let first_line = req.lines().next().unwrap_or("");

        // Разбор query: GET /callback?code=...&state=...
        let query = first_line
            .split(' ')
            .nth(1)
            .unwrap_or_default()
            .split('?')
            .nth(1)
            .unwrap_or_default();
        let qparam = |name: &str| -> Option<String> {
            query.split('&').find_map(|kv| {
                let mut it = kv.splitn(2, '=');
                (it.next()? == name).then(|| it.next().unwrap_or_default().to_string())
            })
        };
        let got_code = qparam("code");
        let got_state = qparam("state");

        let response_body = if got_code.is_some() {
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body style='font-family:sans-serif;background:#36393f;color:#fff;text-align:center;padding-top:80px'><h2>Вход выполнен!</h2><p>Вернитесь в приложение.</p></body></html>"
        } else {
            "HTTP/1.1 404 Not Found\r\n\r\n<!doctype html><html><body>listening for callback...</body></html>\r\n"
        };
        let _ = stream.write_all(response_body.as_bytes()).await;

        match (got_code, got_state) {
            (Some(code), Some(state)) if state == expected_state => return Ok(code),
            (Some(_), _) => {
                log::warn!("OAuth: callback без state/с неверным state — игнорирую");
                continue;
            }
            _ => continue,
        }
    }
}

#[command]
pub(crate) async fn start_login(state: State<'_, DiscordState>) -> Result<serde_json::Value, String> {
    let cfg = discord_client::read_oauth_config()?;

    // 1. Открываем браузер для авторизации (state — защита callback от подмены)
    let state_token = random_oauth_state();
    let auth_url = format!(
        "https://discord.com/oauth2/authorize?client_id={}&response_type=code&redirect_uri={}&scope=identify%20guilds&state={}",
        cfg.client_id,
        REDIRECT_URI.replace(':', "%3A").replace('/', "%2F"),
        state_token
    );

    let listener: TcpListener = TcpListener::bind("127.0.0.1:1436")
        .await
        .map_err(|e| format!("Порт 1436 занят: {e}"))?;

    std::process::Command::new("xdg-open")
        .arg(&auth_url)
        .spawn()
        .map_err(|e| format!("Не удалось открыть браузер: {e}"))?;

    let code = wait_for_code_on(listener, &state_token).await?;

    // 2. Обмен кода на токен
    let client = reqwest::Client::new();
    let params = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", crate::auth::REDIRECT_URI),
    ];

    let token_resp: TokenResponse = client
        .post(format!("{}/oauth2/token", crate::api::API_BASE))
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
        .get(format!("{}/users/@me", crate::api::API_BASE))
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

#[command]
pub(crate) async fn logout(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<(), String> {
    let st = clone_state(&state);
    reset_voice_on_identity_change(&app, &st).await;
    // останавливаем gateway-цикл (новый стартует после следующего логина)
    request_gateway_stop(&sender, std::time::Duration::from_secs(5)).await;
    state.set_session(None)
}

#[command]
pub(crate) fn get_me(state: State<'_, DiscordState>) -> Result<serde_json::Value, String> {
    let guard = state.session.lock().unwrap();
    let s = guard.as_ref().ok_or("Не залогинен")?;
    Ok(serde_json::json!({
        "user_id": s.user_id,
        "username": s.username
    }))
}
