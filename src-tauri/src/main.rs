#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use discord_client::{Channel, DiscordState, Guild, LoginRequest, LoginResponse};
use serde::Deserialize;
use tauri::{command, State, AppHandle, Manager, generate_handler, generate_context};

#[derive(Debug, Deserialize)]
pub struct JoinVoiceRequest {
    pub channel_id: String,
    pub guild_id: String,
}

// --- Команды ---

#[command]
async fn login(
    app: AppHandle,
    state: State<'_, DiscordState>,
    payload: LoginRequest,
) -> Result<LoginResponse, String> {
    // TODO: обмен кода OAuth2 на токен через Discord API
    log::info!("Login attempt with code: {}", payload.code);
    let _ = (app, state);
    Ok(LoginResponse {
        success: false,
        user: None,
        error: Some("OAuth exchange not implemented yet".into()),
    })
}

#[command]
async fn get_guilds(state: State<'_, DiscordState>) -> Result<Vec<Guild>, String> {
    // TODO: GET /users/@me/guilds
    let _ = state;
    Ok(vec![])
}

#[command]
async fn get_channels(guild_id: String) -> Result<Vec<Channel>, String> {
    // TODO: GET /guilds/{guild_id}/channels
    let _ = guild_id;
    Ok(vec![])
}

#[command]
async fn join_voice_channel(
    _app: AppHandle,
    _state: State<'_, DiscordState>,
    payload: JoinVoiceRequest,
) -> Result<(), String> {
    // TODO: подключение к голосовому каналу через WebRTC мост
    let _ = payload;
    Err("Voice not implemented yet".into())
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .manage(DiscordState::default())
        .invoke_handler(generate_handler![
            login,
            get_guilds,
            get_channels,
            join_voice_channel
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").expect("main window not found");
            #[cfg(debug_assertions)]
            {
                window.open_devtools();
            }
            Ok(())
        })
        .run(generate_context!())
        .expect("error while running tauri application");
}
