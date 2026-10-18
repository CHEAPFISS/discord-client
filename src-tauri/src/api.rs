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

/// Вложение сообщения (картинка, GIF, видео или файл).
#[derive(Debug, Clone, Serialize)]
pub struct Attachment {
    pub url: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
}

/// Поле embed-карточки.
#[derive(Debug, Clone, Serialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

/// Embed-карточка сообщения (превью ссылки, тенор-GIF и т.п.) — нужное подмножество полей Discord.
#[derive(Debug, Clone, Serialize)]
pub struct Embed {
    /// Тип эмбеда Discord: rich | image | gifv | video | link …
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub color: Option<u64>,
    pub image: Option<String>,
    /// gifv-эмбеды (гифки Klipy/Giphy/Tenor): Discord отдаёт mp4-версию в video.
    pub video: Option<String>,
    pub thumbnail: Option<String>,
    pub author: Option<String>,
    pub footer: Option<String>,
    pub fields: Vec<EmbedField>,
}

/// Сообщение чата, которое отдаём во фронт.
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub id: String,
    pub channel_id: String,
    pub author: String,
    pub author_id: String,
    pub author_avatar: Option<String>,
    pub content: String,
    pub timestamp: String,
    pub attachments: Vec<Attachment>,
    pub embeds: Vec<Embed>,
}

/// Разбирает сообщение Discord (единый формат у REST-истории и gateway MESSAGE_CREATE).
pub fn parse_message(d: &serde_json::Value) -> ChatMessage {
    let attachments = d["attachments"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(Attachment {
                        url: a["url"].as_str()?.to_string(),
                        filename: a["filename"].as_str().unwrap_or("файл").to_string(),
                        content_type: a["content_type"].as_str().map(str::to_string),
                        width: a["width"].as_u64(),
                        height: a["height"].as_u64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let embeds = d["embeds"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|e| Embed {
                    kind: e["type"].as_str().unwrap_or("rich").to_string(),
                    title: e["title"].as_str().map(str::to_string),
                    description: e["description"].as_str().map(str::to_string),
                    url: e["url"].as_str().map(str::to_string),
                    color: e["color"].as_u64(),
                    image: e["image"]["url"].as_str().map(str::to_string),
                    video: e["video"]["proxy_url"]
                        .as_str()
                        .or(e["video"]["url"].as_str())
                        .map(str::to_string),
                    thumbnail: e["thumbnail"]["url"].as_str().map(str::to_string),
                    author: e["author"]["name"].as_str().map(str::to_string),
                    footer: e["footer"]["text"].as_str().map(str::to_string),
                    fields: e["fields"]
                        .as_array()
                        .map(|f| {
                            f.iter()
                                .map(|x| EmbedField {
                                    name: x["name"].as_str().unwrap_or("").to_string(),
                                    value: x["value"].as_str().unwrap_or("").to_string(),
                                    inline: x["inline"].as_bool().unwrap_or(false),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    ChatMessage {
        id: d["id"].as_str().unwrap_or("").to_string(),
        channel_id: d["channel_id"].as_str().unwrap_or("").to_string(),
        author: d["author"]["username"].as_str().unwrap_or("").to_string(),
        author_id: d["author"]["id"].as_str().unwrap_or("").to_string(),
        author_avatar: d["author"]["avatar"].as_str().map(str::to_string),
        content: d["content"].as_str().unwrap_or("").to_string(),
        timestamp: d["timestamp"].as_str().unwrap_or("").to_string(),
        attachments,
        embeds,
    }
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
    /// user_id → PresenceInfo (статус + активности)
    pub presences: std::collections::HashMap<String, discord_client::PresenceInfo>,
    /// user_id → голосовое состояние (канал + флаги демонстрации экрана/камеры)
    pub voice: std::collections::HashMap<String, discord_client::VoiceStateInfo>,
}

/// Общий HTTP-клиент: пул соединений (раньше новый Client на каждый запрос —
/// заново TLS-хэндшейк) + таймаут, чтобы зависший запрос не висел вечно.
pub(crate) fn http() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client")
    })
}

/// GET с bearer-токеном. Возвращает raw JSON (проверку на ошибку API делает вызывающий).
async fn discord_get(token: &str, url: &str) -> Result<serde_json::Value, String> {
    http()
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
    http()
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
            let mut msg = parse_message(m);
            if msg.channel_id.is_empty() {
                msg.channel_id = channel_id.clone();
            }
            out.push(msg);
        }
    } else {
        return Err(api_error(&resp));
    }
    // API возвращает новые→старые; реверсим для отображения
    out.reverse();
    log::debug!("get_messages: channel={channel_id} отдано {} шт.", out.len());
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

    let mut msg = parse_message(&resp);
    if msg.channel_id.is_empty() {
        msg.channel_id = channel_id;
    }
    Ok(msg)
}

/// Открыть ссылку (картинка/файл/ссылка из чата) в системном браузере.
#[command]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Разрешены только http(s)-ссылки".into());
    }
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Не удалось открыть: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_message_basic() {
        let d = json!({
            "id": "123",
            "channel_id": "456",
            "content": "привет",
            "timestamp": "2026-10-18T00:00:00Z",
            "author": {"id": "789", "username": "cheapfiss", "avatar": "abc"}
        });
        let m = parse_message(&d);
        assert_eq!(m.id, "123");
        assert_eq!(m.channel_id, "456");
        assert_eq!(m.content, "привет");
        assert_eq!(m.author, "cheapfiss");
        assert_eq!(m.author_id, "789");
        assert_eq!(m.author_avatar.as_deref(), Some("abc"));
        assert!(m.attachments.is_empty());
        assert!(m.embeds.is_empty());
    }

    #[test]
    fn parse_message_missing_fields_are_safe() {
        let m = parse_message(&json!({}));
        assert_eq!(m.id, "");
        assert_eq!(m.content, "");
        assert!(m.author_avatar.is_none());
    }

    #[test]
    fn parse_message_attachments_and_embeds() {
        let d = json!({
            "attachments": [
                {"url": "https://x/img.png", "filename": "img.png", "content_type": "image/png", "width": 100, "height": 50},
                {"filename": "no-url.png"}
            ],
            "embeds": [
                {"type": "gifv", "title": "Gif", "video": {"proxy_url": "https://x/g.mp4"}, "color": 16711680}
            ]
        });
        let m = parse_message(&d);
        // attachment без url отбрасывается
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].width, Some(100));
        assert_eq!(m.embeds.len(), 1);
        assert_eq!(m.embeds[0].kind, "gifv");
        assert_eq!(m.embeds[0].video.as_deref(), Some("https://x/g.mp4"));
        assert_eq!(m.embeds[0].color, Some(16711680));
    }
}
