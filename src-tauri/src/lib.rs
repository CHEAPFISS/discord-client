use serde::{Deserialize, Serialize};
use std::sync::Mutex;

// --- Состояние приложения ---

pub struct DiscordState {
    pub session: Mutex<Option<DiscordSession>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSession {
    pub token: String,
    pub user_id: String,
    pub username: String,
}

impl Default for DiscordState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

// --- Типы для команд ---

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub code: String,
    pub redirect_uri: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub user: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Channel {
    pub id: String,
    pub guild_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: u8,
}
