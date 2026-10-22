//! Minimal backend localization for user-facing error strings.
//!
//! The frontend owns the language switch and calls `set_language` on startup
//! and whenever the user changes it. Only strings that can reach the user
//! through commands or events are localized here; log messages are not.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ru = 0,
    En = 1,
}

/// Preserves the previous (Russian) behaviour until the UI reports a language.
static LANG: AtomicU8 = AtomicU8::new(Lang::Ru as u8);

/// Accepts `ru`, `ru-RU`, `en`, `en-US`, … Unknown tags fall back to English.
pub fn set(tag: &str) {
    let is_ru = tag.trim().to_ascii_lowercase().starts_with("ru");
    let value = if is_ru { Lang::Ru } else { Lang::En };
    LANG.store(value as u8, Ordering::Relaxed);
}

fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Ru
    }
}

/// User-facing backend strings. Keep this list small: the UI owns most text.
pub enum Key {
    EmptyToken,
    NotLoggedIn,
    GatewayNotConnected,
    OnlyHttpLinks,
    OpenUrlFailed,
    ConfigMissing,
    ConfigParse,
    RequestFailed,
    JsonParse,
    ApiError,
    Unknown,
    WsHello,
    CommandChannelClosed,
    GatewayClosed,
    GatewayReconnect,
    StreamClosed,
    NoMembersChannel,
    MembersSubscribe,
    MembersNoList,
    FreshVoicePair,
    PortBusy,
    BrowserOpenFailed,
    LoginCancelled,
    OAuthHttp,
    OAuthSuccess,
    OAuthReturn,
    TokenParse,
    UserRequest,
    UserParse,
    TokenRejected,
    NoOutputAudio,
    NoOutputConfig,
    NoInputAudio,
    NoMicConfig,
    MicEncodeTask,
    MicStreamTask,
    MicStreamEnded,
    StreamNoRtcServer,
    StreamNoRtcChannel,
    StreamNoEndpoint,
    StreamNoToken,
}

pub fn t(key: Key) -> &'static str {
    match (lang(), key) {
        (Lang::Ru, Key::EmptyToken) => "Пустой токен",
        (Lang::En, Key::EmptyToken) => "Empty token",

        (Lang::Ru, Key::NotLoggedIn) => "Не залогинен",
        (Lang::En, Key::NotLoggedIn) => "Not signed in",

        (Lang::Ru, Key::GatewayNotConnected) => "Gateway не подключен",
        (Lang::En, Key::GatewayNotConnected) => "Gateway is not connected",

        (Lang::Ru, Key::OnlyHttpLinks) => "Разрешены только http(s)-ссылки",
        (Lang::En, Key::OnlyHttpLinks) => "Only http(s) links are allowed",

        (Lang::Ru, Key::OpenUrlFailed) => "Не удалось открыть: {e}",
        (Lang::En, Key::OpenUrlFailed) => "Failed to open: {e}",

        (Lang::Ru, Key::ConfigMissing) => {
            "Не найден конфиг {path}. Создай его с содержимым:\n{example}\n(client_id/client_secret из Discord Developer Portal → приложение → OAuth2)"
        }
        (Lang::En, Key::ConfigMissing) => {
            "Config not found: {path}. Create it with:\n{example}\n(client_id/client_secret from the Discord Developer Portal → application → OAuth2)"
        }

        (Lang::Ru, Key::ConfigParse) => "Ошибка разбора {path}: {e}",
        (Lang::En, Key::ConfigParse) => "Failed to parse {path}: {e}",

        (Lang::Ru, Key::RequestFailed) => "Ошибка запроса: {e}",
        (Lang::En, Key::RequestFailed) => "Request failed: {e}",

        (Lang::Ru, Key::JsonParse) => "Ошибка разбора JSON: {e}",
        (Lang::En, Key::JsonParse) => "Failed to parse JSON: {e}",

        (Lang::Ru, Key::ApiError) => "API вернул ошибку: {message} (code {code})",
        (Lang::En, Key::ApiError) => "API returned an error: {message} (code {code})",

        (Lang::Ru, Key::Unknown) => "неизвестно",
        (Lang::En, Key::Unknown) => "unknown",

        (Lang::Ru, Key::WsHello) => "ws при HELLO",
        (Lang::En, Key::WsHello) => "ws during HELLO",

        (Lang::Ru, Key::CommandChannelClosed) => "командный канал закрылся",
        (Lang::En, Key::CommandChannelClosed) => "command channel closed",

        (Lang::Ru, Key::GatewayClosed) => "gateway закрыл соединение",
        (Lang::En, Key::GatewayClosed) => "gateway closed the connection",

        (Lang::Ru, Key::GatewayReconnect) => "gateway требует реконнект",
        (Lang::En, Key::GatewayReconnect) => "gateway requested a reconnect",

        (Lang::Ru, Key::StreamClosed) => "поток закрылся",
        (Lang::En, Key::StreamClosed) => "stream closed",

        (Lang::Ru, Key::NoMembersChannel) => "Нет канала для запроса списка участников",
        (Lang::En, Key::NoMembersChannel) => "No channel available to request the member list",

        (Lang::Ru, Key::MembersSubscribe) => "Не удалось отправить подписку на список",
        (Lang::En, Key::MembersSubscribe) => "Failed to send the member list subscription",

        (Lang::Ru, Key::MembersNoList) => "Discord не прислал список участников",
        (Lang::En, Key::MembersNoList) => "Discord did not send the member list",

        (Lang::Ru, Key::FreshVoicePair) => "не получил свежую voice-пару после 3 попыток",
        (Lang::En, Key::FreshVoicePair) => "could not obtain a fresh voice pair after 3 attempts",

        (Lang::Ru, Key::PortBusy) => "Порт 1436 занят: {e}",
        (Lang::En, Key::PortBusy) => "Port 1436 is busy: {e}",

        (Lang::Ru, Key::BrowserOpenFailed) => "Не удалось открыть браузер: {e}",
        (Lang::En, Key::BrowserOpenFailed) => "Failed to open the browser: {e}",

        (Lang::Ru, Key::LoginCancelled) => "Вход отменён: не пришёл ответ из браузера за 10 минут",
        (Lang::En, Key::LoginCancelled) => "Sign-in cancelled: no browser response within 10 minutes",

        (Lang::Ru, Key::OAuthHttp) => "Ошибка HTTP при обмене кода: {e}",
        (Lang::En, Key::OAuthHttp) => "HTTP error while exchanging the code: {e}",

        (Lang::Ru, Key::OAuthSuccess) => "Вход выполнен!",
        (Lang::En, Key::OAuthSuccess) => "Signed in!",

        (Lang::Ru, Key::OAuthReturn) => "Вернитесь в приложение.",
        (Lang::En, Key::OAuthReturn) => "Return to the app.",

        (Lang::Ru, Key::TokenParse) => "Не удалось разобрать токен-ответ: {e}",
        (Lang::En, Key::TokenParse) => "Failed to parse the token response: {e}",

        (Lang::Ru, Key::UserRequest) => "Ошибка запроса пользователя: {e}",
        (Lang::En, Key::UserRequest) => "User request failed: {e}",

        (Lang::Ru, Key::UserParse) => "Не удалось разобрать пользователя: {e}",
        (Lang::En, Key::UserParse) => "Failed to parse the user: {e}",

        (Lang::Ru, Key::TokenRejected) => "Токен не принят Discord: {message} (code {code})",
        (Lang::En, Key::TokenRejected) => "Discord rejected the token: {message} (code {code})",

        (Lang::Ru, Key::NoOutputAudio) => "нет выходного аудиоустройства",
        (Lang::En, Key::NoOutputAudio) => "no audio output device",

        (Lang::Ru, Key::NoOutputConfig) => "нет конфигурации выходного аудиоустройства",
        (Lang::En, Key::NoOutputConfig) => "no audio output device configuration",

        (Lang::Ru, Key::NoInputAudio) => "нет входного аудиоустройства (микрофон)",
        (Lang::En, Key::NoInputAudio) => "no audio input device (microphone)",

        (Lang::Ru, Key::NoMicConfig) => {
            "нет подходящей конфигурации микрофона (f32, моно, 48 кГц)"
        }
        (Lang::En, Key::NoMicConfig) => "no suitable microphone configuration (f32, mono, 48 kHz)",

        (Lang::Ru, Key::MicEncodeTask) => "поток кодирования микрофона: {e}",
        (Lang::En, Key::MicEncodeTask) => "microphone encoder task: {e}",

        (Lang::Ru, Key::MicStreamTask) => "поток микрофонного потока: {e}",
        (Lang::En, Key::MicStreamTask) => "microphone stream task: {e}",

        (Lang::Ru, Key::MicStreamEnded) => "поток микрофонного потока завершился рано",
        (Lang::En, Key::MicStreamEnded) => "microphone stream ended early",

        (Lang::Ru, Key::StreamNoRtcServer) => "stream: нет rtc_server_id",
        (Lang::En, Key::StreamNoRtcServer) => "stream: no rtc_server_id",

        (Lang::Ru, Key::StreamNoRtcChannel) => "stream: нет rtc_channel_id",
        (Lang::En, Key::StreamNoRtcChannel) => "stream: no rtc_channel_id",

        (Lang::Ru, Key::StreamNoEndpoint) => "stream: нет endpoint",
        (Lang::En, Key::StreamNoEndpoint) => "stream: no endpoint",

        (Lang::Ru, Key::StreamNoToken) => "stream: нет token",
        (Lang::En, Key::StreamNoToken) => "stream: no token",
    }
}

/// Translate a key and substitute `{name}` placeholders.
pub fn f(key: Key, vars: &[(&str, &str)]) -> String {
    let mut out = t(key).to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Called by the frontend on startup and on every language change.
#[tauri::command]
pub fn set_language(lang: String) {
    log::debug!("set_language called with {lang:?}");
    set(&lang);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // These tests mutate the process-wide language, so they must not overlap.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn language_tags_are_parsed() {
        let _guard = TEST_LOCK.lock().unwrap();
        set("ru-RU");
        assert_eq!(t(Key::EmptyToken), "Пустой токен");
        set("en-US");
        assert_eq!(t(Key::EmptyToken), "Empty token");
        set("de");
        assert_eq!(t(Key::EmptyToken), "Empty token");
        // Restore the default for other tests.
        set("ru");
    }

    #[test]
    fn placeholders_are_substituted() {
        let _guard = TEST_LOCK.lock().unwrap();
        set("en");
        assert_eq!(
            f(Key::RequestFailed, &[("e", "timeout")]),
            "Request failed: timeout"
        );
        set("ru");
    }
}
