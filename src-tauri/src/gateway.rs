//! Tauri-слой шлюза поверх крейта `client-gateway`.
//!
//! Здесь живут адаптеры под Tauri ([`TauriEventSink`] — эмиты во фронт,
//! [`TauriVoiceOps`] — вызовы voice-модуля) и тонкие обёртки команд.
//! Сам WebSocket-цикл, auth и members — в `client-gateway` (без Tauri).

use std::sync::Arc;

use client_gateway::{BoxFuture, EventSink, GatewayCtx, VoiceConnectParams, VoiceOps};
use discord_client::{DiscordState, GatewaySender};
use tauri::{command, AppHandle, Emitter, Manager, State};

/// Эмит события во фронт: реализация [`EventSink`] через `AppHandle::emit`.
pub(crate) struct TauriEventSink {
    app: AppHandle,
}

impl EventSink for TauriEventSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.app.emit(event, payload);
    }
}

/// Голос/стрим: реализация [`VoiceOps`] поверх voice-модуля Tauri-слоя —
/// так разрывается цикл gateway → voice (крейт не знает о voice напрямую).
pub(crate) struct TauriVoiceOps {
    app: AppHandle,
}

impl VoiceOps for TauriVoiceOps {
    fn connect_voice(&self, state: DiscordState, params: VoiceConnectParams) {
        let app = self.app.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::voice::connect::connect_voice(
                app,
                state,
                params.guild_id,
                params.channel_id,
                params.session_id,
                params.token,
                params.endpoint,
            )
            .await
            {
                log::error!("Voice error: {e}");
            }
        });
    }

    fn disconnect_stream(&self) -> BoxFuture<'_, ()> {
        let app = self.app.clone();
        Box::pin(async move {
            crate::voice::stream::disconnect_stream(&app).await;
        })
    }

    fn maybe_connect_stream(&self) {
        crate::voice::stream::maybe_connect_stream(&self.app);
    }

    fn clear_speaking(&self, state: &DiscordState) {
        crate::voice::clear_speaking(&self.app, state);
    }
}

/// Собирает контекст шлюза из Tauri-состояний.
///
/// `GatewaySender` клонируется дёшево (канал в Arc) — контекст разделяет его
/// с Tauri-состоянием, опубликованным в main.rs.
pub(crate) fn make_ctx(app: &AppHandle) -> GatewayCtx {
    let state = app.state::<DiscordState>().inner().clone();
    let sender = app.state::<GatewaySender>().inner().clone();
    GatewayCtx::new(
        state,
        sender,
        Arc::new(TauriEventSink { app: app.clone() }),
        Arc::new(TauriVoiceOps { app: app.clone() }),
    )
}

#[command]
pub async fn connect_gateway(app: AppHandle) -> Result<String, String> {
    client_gateway::connect_gateway(make_ctx(&app)).await
}

#[command]
pub fn get_gateway_status(state: State<'_, DiscordState>) -> serde_json::Value {
    client_gateway::get_gateway_status(state.inner())
}
