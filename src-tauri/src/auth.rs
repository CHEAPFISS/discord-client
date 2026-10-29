//! Tauri-команды входа поверх крейта `client-gateway` (модуль `auth`).
//!
//! Вся логика (проверка токена, OAuth-сервер, обмен кода) — в крейте;
//! здесь только обёртки команд, собирающие контекст из Tauri-состояний.

use tauri::{command, AppHandle};

use crate::gateway::make_ctx;

#[command]
pub async fn login_with_token(app: AppHandle, token: String) -> Result<serde_json::Value, String> {
    client_gateway::login_with_token(&make_ctx(&app), &token).await
}

#[command]
pub async fn start_login(app: AppHandle) -> Result<serde_json::Value, String> {
    client_gateway::start_login(&make_ctx(&app).state).await
}

#[command]
pub async fn logout(app: AppHandle) -> Result<(), String> {
    client_gateway::logout(&make_ctx(&app)).await
}

#[command]
pub fn get_me(app: AppHandle) -> Result<serde_json::Value, String> {
    client_gateway::get_me(&make_ctx(&app).state)
}
