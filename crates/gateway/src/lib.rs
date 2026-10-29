//! Discord Gateway v10, OAuth-вход и список участников — бэкенд без Tauri.
//!
//! Модули [`gateway`] (WebSocket-цикл), [`auth`] (вход по токену/OAuth) и
//! [`members`] (подписка op 37 на диапазоны списка участников) не знают о
//! Tauri: они работают через [`GatewayCtx`]. UI-слой (src-tauri) подставляет
//! реализации [`EventSink`] (эмиты событий во фронт) и [`VoiceOps`]
//! (подключение голоса/стрима) — так разрывается цикл gateway → voice.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use client_core::{DiscordState, GatewaySender};

/// Будущее, отдаваемое трейт-объектом (объектная безопасность).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Эмит события во фронт.
///
/// В Tauri-клиенте это `app.emit(...)`; egui-конструктор и MCP могут писать
/// события в свой канал (или игнорировать — реализация по умолчанию молчит).
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// Пустая реализация: события никому не нужны (браузерный фронт отсутствует).
pub struct NullEvents;

impl EventSink for NullEvents {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

/// Параметры голосового подключения из VOICE_STATE_UPDATE + VOICE_SERVER_UPDATE.
#[derive(Debug, Clone)]
pub struct VoiceConnectParams {
    pub guild_id: String,
    pub channel_id: String,
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
}

/// Голосовой/стримовый слой, которым управляет gateway.
///
/// Нужен, чтобы gateway не зависел от voice-модуля напрямую (иначе цикл
/// gateway ↔ voice). Tauri-реализация живёт в src-tauri.
pub trait VoiceOps: Send + Sync {
    /// Авто-подключение к голосовому каналу (тройка session/token/endpoint собрана).
    fn connect_voice(&self, state: DiscordState, params: VoiceConnectParams);

    /// Полный офф RTC-коннекта стрима.
    fn disconnect_stream(&self) -> BoxFuture<'_, ()>;

    /// Поднять RTC-коннект стрима, если его цель уже собрана.
    fn maybe_connect_stream(&self);

    /// Сбросить индикаторы «говорит» у всех участников.
    fn clear_speaking(&self, state: &DiscordState);
}

/// Голос/стрим недоступны (браузерный фронт или MCP без звука).
pub struct NullVoice;

impl VoiceOps for NullVoice {
    fn connect_voice(&self, _state: DiscordState, params: VoiceConnectParams) {
        log::debug!(
            "VoiceOps отключён: пропускаю подключение к {}",
            params.channel_id
        );
    }
    fn disconnect_stream(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn maybe_connect_stream(&self) {}
    fn clear_speaking(&self, _state: &DiscordState) {}
}

/// Контекст шлюза: всё, что раньше бралось из `tauri::AppHandle`.
///
/// Клонируется дёшево (все поля — Arc/клоны Arc): задачи gateway-цикла живут
/// со своим клоном, разделяя состояние с Tauri-состоянием приложения.
#[derive(Clone)]
pub struct GatewayCtx {
    /// Общее состояние клиента (сессия, presences, голос, стримы…).
    pub state: DiscordState,
    /// Канал отправки payload в ws-задачу (op 4, heartbeat, релогин-маркер).
    pub sender: GatewaySender,
    /// Эмиты событий во фронт.
    pub events: Arc<dyn EventSink>,
    /// Голос/стрим (разрыв цикла gateway → voice).
    pub voice: Arc<dyn VoiceOps>,
}

impl GatewayCtx {
    pub fn new(
        state: DiscordState,
        sender: GatewaySender,
        events: Arc<dyn EventSink>,
        voice: Arc<dyn VoiceOps>,
    ) -> Self {
        Self {
            state,
            sender,
            events,
            voice,
        }
    }

    /// Эмит события во фронт (как `AppHandle::emit`).
    pub fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.emit(event, payload);
    }
}

pub mod auth;
pub mod gateway;
pub mod members;

pub use auth::{get_me, login_with_token, logout, start_login};
pub use gateway::{connect_gateway, get_gateway_status};
pub use members::get_guild_members;
