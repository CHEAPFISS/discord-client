use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Готовность (session, token, endpoint) → задача подключения.
///
/// Хендл активного oto-соединения + session_id, для которого оно создавалось.
/// Токен Discord одноразовый — повторное подключение на ту же сессию даёт 4014/4022.
pub struct VoiceSlot {
    pub conn: Option<ActiveVoiceConnection>,
    pub session_id: Option<String>,
    pub token: Option<String>,
    /// Насос событий уже заспавнен для текущего соединения (один, не на каждый connect).
    pub pump_spawned: bool,
}

/// Данные стрима (Go Live), которые мы смотрим: RTC-сервер/канал + токен/endpoint.
/// Собираются из `STREAM_CREATE` (rtc_server_id/rtc_channel_id) и
/// `STREAM_SERVER_UPDATE` (token/endpoint); когда обе части есть — можно
/// поднимать отдельный RTC-коннект стрима (Шаг 1.x: oto video).
#[derive(Debug, Clone, Default)]
pub struct StreamTarget {
    /// `guild:channel:owner` — идентификатор стрима (нужен для Watch/Delete).
    pub stream_key: String,
    /// ID стримера (владельца стрима).
    pub owner_id: String,
    /// RTC-сервер стрима (= server_id для stream-Identify). Своя DAVE-группа
    /// вычисляется отсюда: media_session_id = rtc_server_id - 1.
    pub rtc_server_id: Option<String>,
    /// RTC-канал стрима (= channel_id для stream-Identify).
    pub rtc_channel_id: Option<String>,
    /// Токен голосового подключения стрима (из STREAM_SERVER_UPDATE).
    pub token: Option<String>,
    /// Endpoint голосового сервера стрима (из STREAM_SERVER_UPDATE).
    pub endpoint: Option<String>,
}

impl StreamTarget {
    /// Обе части собраны (RTC-идентичность + токен/endpoint) → можно подключаться.
    pub fn ready(&self) -> bool {
        self.rtc_server_id.is_some()
            && self.rtc_channel_id.is_some()
            && self.token.is_some()
            && self.endpoint.is_some()
    }
}

impl Default for VoiceSlot {
    fn default() -> Self {
        Self { conn: None, session_id: None, token: None, pump_spawned: false }
    }
}

// --- Состояние приложения ---

/// Аккумулятор одного запроса списка участников (op 37 → GUILD_MEMBER_LIST_UPDATE).
pub struct FetchSlot {
    /// Начала запрошенных диапазонов (например [0, 100]).
    pub wanted: Vec<i64>,
    pub state: Mutex<FetchState>,
    pub notify: tokio::sync::Notify,
}

#[derive(Default)]
pub struct FetchState {
    /// Элементы SYNC по диапазонам (start → `{"group":…}` / `{"member":…}`
    /// в порядке отображения Discord) — из них собираем группы при выдаче.
    pub ranges: std::collections::BTreeMap<i64, Vec<serde_json::Value>>,
    /// Начала диапазонов, по которым уже пришёл SYNC.
    pub seen: std::collections::HashSet<i64>,
    pub done: bool,
}

/// Реестр активных запросов + кэш готовых списков участников.
#[derive(Default)]
pub struct MemberFetchHub {
    pub fetches: Mutex<HashMap<String, Arc<FetchSlot>>>,
    /// guild_id → (когда, готовые группы) — чтобы не дёргать gateway на каждый клик.
    pub cache: Mutex<HashMap<String, (std::time::Instant, Vec<MemberGroup>)>>,
}

/// Участник сервера (общая структура для фронта).
#[derive(Debug, Clone, Serialize)]
pub struct MemberInfo {
    pub user_id: String,
    /// Отображаемое имя: ник → display_name → global_name → username.
    pub username: String,
    pub avatar: Option<String>,
    pub bot: bool,
    pub roles: Vec<String>,
}

/// Группа списка участников, как её прислал Discord:
/// id роли (hoist) либо "online"/"offline".
#[derive(Debug, Clone, Serialize)]
pub struct MemberGroup {
    pub id: String,
    pub members: Vec<MemberInfo>,
}

/// Активность участника (Rich Presence): игра, стрим, Spotify, кастом-статус и т.д.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityInfo {
    /// Тип: 0 — играет, 1 — стримит, 2 — слушает, 3 — смотрит, 4 — кастом-статус,
    /// 5 — соревнуется.
    pub kind: u64,
    pub name: String,
    pub details: Option<String>,
    pub state: Option<String>,
    pub url: Option<String>,
    /// Эмодзи кастом-статуса (юникод-символ или `:имя:` для эмодзи сервера).
    pub emoji: Option<String>,
}

/// Presence участника: статус + активности.
#[derive(Debug, Clone, Serialize)]
pub struct PresenceInfo {
    pub status: String,
    pub activities: Vec<ActivityInfo>,
}

/// Голосовое состояние участника: канал + флаги демонстрации экрана/камеры.
/// `self_stream`/`self_video` приходят в VOICE_STATE_UPDATE (и в снапшотах
/// READY/GUILD_CREATE) — по ним рисуем индикатор «Демонстрирует свой экран».
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct VoiceStateInfo {
    pub channel_id: String,
    pub self_stream: bool,
    pub self_video: bool,
}

/// Разбор голосового состояния Discord (VOICE_STATE_UPDATE, READY/GUILD_CREATE)
/// в VoiceStateInfo. Требует непустой channel_id (иначе участник вне голоса).
pub fn parse_voice_state(s: &serde_json::Value) -> Option<VoiceStateInfo> {
    let channel_id = s["channel_id"].as_str()?.to_string();
    if channel_id.is_empty() {
        return None;
    }
    Some(VoiceStateInfo {
        channel_id,
        self_stream: s["self_stream"].as_bool().unwrap_or(false),
        self_video: s["self_video"].as_bool().unwrap_or(false),
    })
}

/// Разбор presence-объекта Discord (PRESENCE_UPDATE, READY/GUILD_CREATE,
/// member-элементы списка участников) в PresenceInfo.
pub fn parse_presence(p: &serde_json::Value) -> PresenceInfo {
    let status = p["status"].as_str().unwrap_or("offline").to_string();
    let activities = p["activities"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|a| ActivityInfo {
                    kind: a["type"].as_u64().unwrap_or(0),
                    name: a["name"].as_str().unwrap_or("").to_string(),
                    details: a["details"].as_str().map(String::from),
                    state: a["state"].as_str().map(String::from),
                    url: a["url"].as_str().map(String::from),
                    emoji: a["emoji"]["name"].as_str().map(|n| {
                        if a["emoji"]["id"].is_string() {
                            format!(":{n}:")
                        } else {
                            n.to_string()
                        }
                    }),
                })
                .collect()
        })
        .unwrap_or_default();
    PresenceInfo { status, activities }
}

/// Индикатор «говорит» для мини-списка голосового канала.
///
/// oto не пробрасывает входящий SPEAKING (op 5), но отдаёт расшифрованные
/// кадры `AudioReceived` с автором: пока кадры идут — участник говорит.
/// Зажигаем по первому кадру (насос oto), гасим в `speaking_watch_task`
/// по паузе без кадров или явным `clear()` при выходе из канала.
#[derive(Default)]
pub struct SpeakingTracker {
    users: Mutex<HashMap<u64, Instant>>,
}

impl SpeakingTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Кадр от участника. true — он только начал говорить (зажечь индикатор).
    pub fn mark(&self, user_id: u64) -> bool {
        let mut map = self.users.lock().unwrap();
        map.insert(user_id, Instant::now()).is_none()
    }

    /// Гасит тех, от кого не было кадров дольше hold; возвращает их user_id.
    pub fn tick(&self, hold: Duration) -> Vec<u64> {
        let mut map = self.users.lock().unwrap();
        let stale: Vec<u64> = map
            .iter()
            .filter(|(_, at)| at.elapsed() >= hold)
            .map(|(uid, _)| *uid)
            .collect();
        for uid in &stale {
            map.remove(uid);
        }
        stale
    }

    /// Полный сброс (выход из голоса): возвращает всех, кого надо погасить.
    pub fn clear(&self) -> Vec<u64> {
        self.users.lock().unwrap().drain().map(|(uid, _)| uid).collect()
    }
}

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
    /// Глухота (self_deaf): серверу в op 4 + гашение приёма звука.
    pub self_deaf: Arc<AtomicBool>,
    /// Мьют микрофона: гасит обводку «говорю сам» (своё аудио мы не слышим).
    pub self_mute: Arc<AtomicBool>,
    /// Индикатор «говорит» для мини-списка голосового канала (зелёная обводка).
    pub voice_speaking: Arc<SpeakingTracker>,
    /// Presence участников (статус + активности): guild_id → user_id → PresenceInfo.
    pub presences: Arc<Mutex<HashMap<String, HashMap<String, PresenceInfo>>>>,
    /// Кто в каком голосовом канале: guild_id → user_id → голосовое состояние
    /// (канал + флаги демонстрации экрана/камеры).
    pub voice_states: Arc<Mutex<HashMap<String, HashMap<String, VoiceStateInfo>>>>,
    /// Число участников сервера (из READY) — для расчёта диапазонов списка.
    pub guild_member_counts: Arc<Mutex<HashMap<String, u64>>>,
    /// Последний статус gateway (status, error) — чтобы после полной перезагрузки
    /// страницы фронт мог спросить текущее состояние (команда get_gateway_status).
    pub gateway_status: Arc<Mutex<(String, Option<String>)>>,
    /// Запросы/кэш списка участников (op 37).
    pub member_fetch: Arc<MemberFetchHub>,
    /// Стрим (Go Live), который мы смотрим: RTC-данные + токен/endpoint.
    /// None — стрим не смотрим.
    pub stream: Arc<Mutex<Option<StreamTarget>>>,
    /// Хендл RTC-соединения стрима (второй коннект параллельно с голосовым).
    pub stream_conn: Arc<Mutex<VoiceSlot>>,
    /// Общий sender кадров в play_audio_task: голос и звук стрима играют через
    /// ОДИН mixer (второй аудио-прибор дал бы конфликт/двойной звук).
    pub playback_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<(u64, Vec<u8>, u8)>>>>,
    /// Когда со стрим-коннекта последний раз шёл звук (для анти-дубля: голос
    /// стримера в родительском канале отбрасываем, пока стрим его несёт).
    /// Пустой стрим (демка без звука) — значение стареет и голос не глушим.
    pub stream_audio_at: Arc<Mutex<Option<std::time::Instant>>>,
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
            self_deaf: Arc::new(AtomicBool::new(false)),
            self_mute: Arc::new(AtomicBool::new(false)),
            voice_speaking: Arc::new(SpeakingTracker::new()),
            presences: Arc::new(Mutex::new(HashMap::new())),
            voice_states: Arc::new(Mutex::new(HashMap::new())),
            guild_member_counts: Arc::new(Mutex::new(HashMap::new())),
            gateway_status: Arc::new(Mutex::new(("disconnected".to_string(), None))),
            member_fetch: Arc::new(MemberFetchHub::default()),
            stream: Arc::new(Mutex::new(None)),
            stream_conn: Arc::new(Mutex::new(VoiceSlot::default())),
            playback_tx: Arc::new(Mutex::new(None)),
            stream_audio_at: Arc::new(Mutex::new(None)),
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

// --- Конфиг OAuth (client_id / client_secret) ---

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// Читает ~/.config/discord-client/config.json
pub fn read_oauth_config() -> Result<OAuthConfig, String> {
    let path = app_config_dir().join("config.json");
    // В config.json лежит client_secret — приватизируем права, если файл открыт
    // шире (иначе любой локальный пользователь прочитает секрет приложения).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_presence_defaults_to_offline() {
        let p = parse_presence(&json!({}));
        assert_eq!(p.status, "offline");
        assert!(p.activities.is_empty());
    }

    #[test]
    fn parse_presence_activities() {
        let p = parse_presence(&json!({
            "status": "online",
            "activities": [
                {"type": 0, "name": "Dota 2", "details": "Ранг", "state": "В очереди"},
                {"type": 4, "name": "кастом", "emoji": {"name": "😀"}}
            ]
        }));
        assert_eq!(p.status, "online");
        assert_eq!(p.activities.len(), 2);
        assert_eq!(p.activities[0].kind, 0);
        assert_eq!(p.activities[0].name, "Dota 2");
        assert_eq!(p.activities[0].details.as_deref(), Some("Ранг"));
        assert_eq!(p.activities[0].state.as_deref(), Some("В очереди"));
        // unicode-эмодзи — без двоеточий
        assert_eq!(p.activities[1].emoji.as_deref(), Some("😀"));
    }

    #[test]
    fn parse_presence_custom_emoji_with_id_is_wrapped() {
        let p = parse_presence(&json!({
            "status": "idle",
            "activities": [
                {"type": 4, "name": "x", "emoji": {"id": "123", "name": "blob"}}
            ]
        }));
        assert_eq!(p.activities[0].emoji.as_deref(), Some(":blob:"));
    }
}
