use serde::Serialize;
use tauri::{command, State};

use discord_client::DiscordState;

pub const API_BASE: &str = "https://discord.com/api/v10";

/// Сервер Discord (либа «гильдия»).
#[derive(Debug, Serialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
}

/// Текстовый или голосовой канал сервера.
#[derive(Debug, Serialize)]
pub struct Channel {
    pub id: String,
    pub guild_id: String,
    pub name: String,
    /// `type` из API Discord (0 текст, 2 голос и т.д.); наружу отдаём как `kind`
    pub kind: u8,
}

/// Сообщение чата, которое отдаём во фронт.
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub id: String,
    pub channel_id: String,
    pub author: String,
    pub content: String,
    pub timestamp: String,
}

/// Участник сервера (для колонки участников).
#[derive(Debug, Serialize)]
pub struct Member {
    pub user_id: String,
    /// Отображаемое имя: ник на сервере → global_name → username.
    pub username: String,
    /// Хэш аватарки для CDN Discord (None — показываем букву).
    pub avatar: Option<String>,
    pub bot: bool,
    /// ID ролей участника.
    pub roles: Vec<String>,
}

/// Роль сервера (для группировки участников по ролям).
#[derive(Debug, Serialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    /// Цвет роли числом (0 — без цвета).
    pub color: u64,
    pub position: i64,
    /// Показывать роль отдельной группой в списке участников?
    pub hoist: bool,
}

/// Presence-статусы и голосовые состояния участников сервера (из gateway).
#[derive(Debug, Serialize)]
pub struct GuildActivity {
    /// user_id → online/idle/dnd/offline
    pub presences: std::collections::HashMap<String, String>,
    /// user_id → channel_id (кто в каком голосовом канале)
    pub voice: std::collections::HashMap<String, String>,
}

/// GET с bearer-токеном. Возвращает raw JSON (проверку на ошибку API делает вызывающий).
async fn discord_get(token: &str, url: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    client
        .get(url)
        .header("Authorization", token)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора JSON: {e}"))
}

/// POST JSON с bearer-токеном.
async fn discord_post_json(
    token: &str,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    client
        .post(url)
        .header("Authorization", token)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Ошибка разбора JSON: {e}"))
}

/// Сообщение API об ошибке, если ответ не массив.
fn api_error(resp: &serde_json::Value) -> String {
    format!(
        "API вернул ошибку: {} (code {})",
        resp["message"].as_str().unwrap_or("неизвестно"),
        resp["code"].as_u64().unwrap_or(0)
    )
}

#[command]
pub async fn get_guilds(
    state: State<'_, DiscordState>,
) -> Result<Vec<Guild>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let resp = discord_get(&token, &format!("{API_BASE}/users/@me/guilds")).await?;

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
        return Err(api_error(&resp));
    }
    Ok(guilds)
}

#[command]
pub async fn get_channels(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<Vec<Channel>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let resp = discord_get(
        &token,
        &format!("{API_BASE}/guilds/{guild_id}/channels"),
    )
    .await?;

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
        return Err(api_error(&resp));
    }
    Ok(channels)
}

#[command]
pub async fn get_guild_members(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<Vec<Member>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let resp = discord_get(
        &token,
        &format!("{API_BASE}/guilds/{guild_id}/members?limit=1000"),
    )
    .await?;

    let mut out = Vec::new();
    if let Some(arr) = resp.as_array() {
        for m in arr {
            let u = &m["user"];
            let username = m["nick"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| u["global_name"].as_str())
                .or_else(|| u["username"].as_str())
                .unwrap_or("")
                .to_string();
            out.push(Member {
                user_id: u["id"].as_str().unwrap_or("").to_string(),
                username,
                avatar: u["avatar"].as_str().map(String::from),
                bot: u["bot"].as_bool().unwrap_or(false),
                roles: m["roles"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|r| r.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    } else {
        return Err(api_error(&resp));
    }
    Ok(out)
}

#[command]
pub async fn get_guild_roles(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<Vec<Role>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let resp = discord_get(&token, &format!("{API_BASE}/guilds/{guild_id}/roles")).await?;

    let mut out = Vec::new();
    if let Some(arr) = resp.as_array() {
        for r in arr {
            out.push(Role {
                id: r["id"].as_str().unwrap_or("").to_string(),
                name: r["name"].as_str().unwrap_or("").to_string(),
                color: r["color"].as_u64().unwrap_or(0),
                position: r["position"].as_i64().unwrap_or(0),
                hoist: r["hoist"].as_bool().unwrap_or(false),
            });
        }
    } else {
        return Err(api_error(&resp));
    }
    Ok(out)
}

#[command]
pub async fn get_guild_activity(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<GuildActivity, String> {
    let presences = state
        .presences
        .lock()
        .unwrap()
        .get(&guild_id)
        .cloned()
        .unwrap_or_default();
    let voice = state
        .voice_states
        .lock()
        .unwrap()
        .get(&guild_id)
        .cloned()
        .unwrap_or_default();
    Ok(GuildActivity { presences, voice })
}

#[command]
pub async fn get_messages(
    state: State<'_, DiscordState>,
    channel_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let resp = discord_get(
        &token,
        &format!("{API_BASE}/channels/{channel_id}/messages?limit=50"),
    )
    .await?;

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
        return Err(api_error(&resp));
    }
    // API возвращает новые→старые; реверсим для отображения
    out.reverse();
    Ok(out)
}

#[command]
pub async fn send_message(
    state: State<'_, DiscordState>,
    channel_id: String,
    content: String,
) -> Result<ChatMessage, String> {
    let token = state.token().ok_or("Не залогинен")?;
    let resp = discord_post_json(
        &token,
        &format!("{API_BASE}/channels/{channel_id}/messages"),
        &serde_json::json!({"content": content}),
    )
    .await?;

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
