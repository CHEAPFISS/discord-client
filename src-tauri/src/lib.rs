use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

/// Отправитель сообщений Gateway (op 4, heartbeat и т.д.) — bridges команды к ws-задаче.
pub struct GatewaySender {
    pub tx: tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
}

impl GatewaySender {
    pub fn new() -> Self {
        Self { tx: tokio::sync::Mutex::new(None) }
    }

    pub async fn send(&self, payload: serde_json::Value) -> Result<(), String> {
        let guard = self.tx.lock().await;
        let sender = guard.as_ref().ok_or("Gateway не подключен")?;
        sender.send(payload).map_err(|e| e.to_string())
    }
}

/// Активное голосовое соединение (oto) — держим хендл живым, иначе соединение закроется.
pub type ActiveVoiceConnection = oto::VoiceConnection;

/// Хендл активного oto-соединения + session_id, для которого оно создавалось.
/// Токен Discord одноразовый — повторное подключение на ту же сессию даёт 4014/4022.
pub struct VoiceSlot {
    pub conn: Option<ActiveVoiceConnection>,
    pub session_id: Option<String>,
    pub token: Option<String>,
    /// Насос событий уже заспавнен для текущего соединения (один, не на каждый connect).
    pub pump_spawned: bool,
}

impl Default for VoiceSlot {
    fn default() -> Self {
        Self { conn: None, session_id: None, token: None, pump_spawned: false }
    }
}

// --- Состояние приложения ---

#[derive(Clone)]
pub struct DiscordState {
    pub session: Arc<Mutex<Option<DiscordSession>>>,
    /// Данные голосового подключения (session_id/token/endpoint приходят асинхронно).
    pub voice: Arc<Mutex<Option<VoiceData>>>,
    /// Хендл активного oto-соединения + сессия, для которой оно создавалось.
    pub voice_conn: Arc<Mutex<VoiceSlot>>,
    /// Сериализация голосовых подключений (токен одноразовый, параллельные коннекты запрещены).
    pub voice_connect_lock: Arc<tokio::sync::Mutex<()>>,
    /// Счётчик авто-переподключений (re-join) после 4014/4022 NeedsFreshVoiceInfo.
    pub voice_rejoin: Arc<AtomicU64>,
    /// Отправитель микрофонного аудио в oto (None вне голоса).
    pub mic_sender: Arc<Mutex<Option<oto::PacedAudioSender>>>,
}

/// Цель подключения к голосовому каналу + собранные данные авторизации.
#[derive(Debug, Clone)]
pub struct VoiceData {
    pub guild_id: String,
    pub channel_id: String,
    pub user_id: String,
    pub session_id: Option<String>,
    pub token: Option<String>,
    pub endpoint: Option<String>,
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
            session: Arc::new(Mutex::new(None)),
            voice: Arc::new(Mutex::new(None)),
            voice_conn: Arc::new(Mutex::new(VoiceSlot::default())),
            voice_connect_lock: Arc::new(tokio::sync::Mutex::new(())),
            voice_rejoin: Arc::new(AtomicU64::new(0)),
            mic_sender: Arc::new(Mutex::new(None)),
        }
    }
}

impl DiscordState {
    /// Загружает состояние с восстановлением сессии с диска.
    pub fn load_or_default() -> Self {
        let state = Self::default();
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
    // type — имя в API Discord (только при разборе ответов); наружу отдаём kind
    #[serde(rename(deserialize = "type"))]
    pub kind: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub id: String,
    pub channel_id: String,
    pub author: String,
    pub content: String,
    pub timestamp: String,
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
    let path = session_path();
    let text = fs::read_to_string(&path).ok()?;
    // Файл мог остаться от старой версии с 0644 — зажимаем права до 0600.
    #[cfg(unix)]
    if let Ok(meta) = fs::metadata(&path) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o777 != 0o600 {
            let mut perms = meta.permissions();
            perms.set_mode((mode & !0o777) | 0o600);
            let _ = fs::set_permissions(&path, perms);
        }
    }
    serde_json::from_str(&text).ok()
}

fn save_session_to_disk(state: &DiscordState) -> Result<(), String> {
    use std::io::Write;
    let guard = state.session.lock().unwrap();
    let path = session_path();
    match guard.as_ref() {
        Some(session) => {
            let json = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
            // Пишем ONLY 0600: в файле plaintext user-токен полного доступа к аккаунту.
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                file.write_all(json.as_bytes()).map_err(|e| e.to_string())
            }
            #[cfg(not(unix))]
            {
                fs::write(&path, json).map_err(|e| e.to_string())
            }
        }
        None => {
            let _ = fs::remove_file(&path);
            Ok(())
        }
    }
}
