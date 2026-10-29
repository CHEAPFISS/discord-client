#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use discord_client::{DiscordState, GatewaySender};
use tauri::{generate_context, generate_handler};

pub use discord_client::i18n;

mod api;
mod audio;
mod auth;
mod gateway;
mod members;
mod video;
mod video_render;
mod voice;

fn main() {
    // glibc создаёт отдельную malloc-арену почти под каждый поток (до 64 MB),
    // а у клиента потоков много: без лимита это десятки MB пустой памяти.
    // Ограничиваем до двух арен до старта tokio/аудио-потоков.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }

    // Дефолт-фильтр: info + глушим крипто/WS-крейсы. RUST_LOG=debug/trace перекрывает.
    // RUST_LOG=trace раскрывает заголовки (Authorization) и Identify-фрейм — не для реальных сессий.
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,hyper=warn,tungstenite=warn,rustls=warn"),
    )
    .init();

    // Явно фиксируем криптопровайдер rustls: иначе первое WS-подключение паникует,
    // если в дереве зависимостей включены и ring, и aws-lc-rs (неоднозначность).
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    // config.json may contain the OAuth client secret; tighten it on startup.
    discord_client::harden_config_permissions();

    tauri::Builder::default()
        .manage(DiscordState::load_or_default())
        .manage(GatewaySender::new())
        .invoke_handler(generate_handler![
            // i18n (обёртка команды — в Tauri-слое, crates/core без Tauri)
            discord_client::commands::set_language,
            // API
            api::get_guilds,
            api::get_channels,
            members::get_guild_members,
            api::get_guild_roles,
            api::get_guild_activity,
            api::get_messages,
            api::send_message,
            api::open_url,
            // Auth
            auth::start_login,
            auth::login_with_token,
            auth::get_me,
            auth::logout,
            // Gateway
            gateway::connect_gateway,
            gateway::get_gateway_status,
            // Voice
            voice::join_voice_channel,
            voice::leave_voice_channel,
            voice::set_self_mute,
            voice::set_self_deaf,
            voice::watch_stream,
            voice::unwatch_stream,
            voice::stream::stream_video_attach,
        ])
        .setup(|app| {
            // Монитор состояния отправки микрофона (диагностика «пропадания звука»).
            // setup идёт вне tokio-контекста — нужен tauri::async_runtime,
            // иначе паника "there is no reactor running".
            tauri::async_runtime::spawn(voice::mic_stats_task(app.handle().clone()));
            // Индикатор «говорит»: гасит зелёную обводку, когда кадры стихли.
            tauri::async_runtime::spawn(voice::speaking_watch_task(app.handle().clone()));
            Ok(())
        })
        .run(generate_context!())
        .expect("error while running tauri application");
}
