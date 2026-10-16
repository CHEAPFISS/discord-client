use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
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

impl DiscordState {
    /// Загружает состояние с восстановлением сессии с диска.
    pub fn load_or_default() -> Self {
        let mut state = Self::default();
        if let Some(session) = load_session_from_disk() {
            log::info!("Session restored for {}", session.username);
            *state.session.lock().unwrap() = Some(session);
        }
        state
    }

    pub fn set_session(&self, session: Option<DiscordSession>) -> Result<(), String> {
        {
            let mut guard = self.session.lock().unwrap();
            *guard = session;
        }
        save_session_to_disk(&self)
    }

    pub fn token(&self) -> Option<String> {
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.token.clone())
    }
}

// --- Типы для команд ---

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

// --- Конфиг OAuth (client_id / client_secret) ---

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// Читает ~/.config/discord-client/config.json
pub fn read_oauth_config() -> Result<OAuthConfig, String> {
    let path = app_config_dir().join("config.json");
    let text = fs::read_to_string(&path).map_err(|_| {
        format!(
            "Не найден конфиг {}. Создай его с содержимым:\n{{\"client_id\": \"ТВОЙ_CLIENT_ID\", \"client_secret\": \"ТВОЙ_CLIENT_SECRET\"}}\n(id/secret из Discord Developer Portal → приложение → OAuth2)",
            path.display()
        )
    })?;
    let cfg: OAuthConfig = serde_json::from_str(&text)
        .map_err(|e| format!("Ошибка разбора {}: {e}", path.display()))?;
    Ok(cfg)
}

/// ~/.config/discord-client/
pub fn app_config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(home).join(".config/discord-client");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn session_path() -> PathBuf {
    app_config_dir().join("session.json")
}

fn load_session_from_disk() -> Option<DiscordSession> {
    let text = fs::read_to_string(session_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_session_to_disk(state: &DiscordState) -> Result<(), String> {
    let guard = state.session.lock().unwrap();
    match guard.as_ref() {
        Some(session) => {
            let json = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
            fs::write(session_path(), json).map_err(|e| e.to_string())
        }
        None => {
            let _ = fs::remove_file(session_path());
            Ok(())
        }
    }
}
