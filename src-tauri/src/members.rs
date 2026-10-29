//! Tauri-команда списка участников поверх крейта `client-gateway` (модуль `members`).

use discord_client::MemberGroup;
use tauri::{command, AppHandle};

use crate::gateway::make_ctx;

#[command]
pub async fn get_guild_members(
    app: AppHandle,
    guild_id: String,
    channel_id: String,
) -> Result<Vec<MemberGroup>, String> {
    let ctx = make_ctx(&app);
    client_gateway::get_guild_members(&ctx.state, &ctx.sender, &guild_id, &channel_id).await
}
