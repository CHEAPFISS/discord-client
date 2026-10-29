//! Tauri-команды REST-слоя поверх крейта `client-api`.
//!
//! Типы ответов, запросы и парсер сообщений живут в `client-api` (без Tauri);
//! здесь только тонкие обёртки команд и ре-экспорт того, что нужно другим
//! Tauri-модулям (auth.rs, gateway.rs).

pub use client_api::{http, parse_message, API_BASE};

use client_api::{Channel, ChatMessage, Guild, GuildActivity, Role};
use discord_client::DiscordState;
use tauri::{command, State};

#[command]
pub async fn get_guilds(state: State<'_, DiscordState>) -> Result<Vec<Guild>, String> {
    client_api::get_guilds(state.inner()).await
}

#[command]
pub async fn get_channels(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<Vec<Channel>, String> {
    client_api::get_channels(state.inner(), &guild_id).await
}

#[command]
pub async fn get_guild_roles(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<Vec<Role>, String> {
    client_api::get_guild_roles(state.inner(), &guild_id).await
}

#[command]
pub async fn get_guild_activity(
    state: State<'_, DiscordState>,
    guild_id: String,
) -> Result<GuildActivity, String> {
    client_api::get_guild_activity(state.inner(), &guild_id).await
}

#[command]
pub async fn get_messages(
    state: State<'_, DiscordState>,
    channel_id: String,
) -> Result<Vec<ChatMessage>, String> {
    client_api::get_messages(state.inner(), &channel_id).await
}

#[command]
pub async fn send_message(
    state: State<'_, DiscordState>,
    channel_id: String,
    content: String,
) -> Result<ChatMessage, String> {
    client_api::send_message(state.inner(), &channel_id, &content).await
}

#[command]
pub fn open_url(url: String) -> Result<(), String> {
    client_api::open_url(&url)
}
