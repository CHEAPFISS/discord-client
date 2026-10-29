//! Tauri-слой поверх ядра `client-core`.
//!
//! Ядро (типы, состояние, сессия, парсеры, i18n) живёт в `crates/core` и не
//! зависит от Tauri — здесь только то, что нужно Tauri-клиенту: ре-экспорт
//! ядра и обёртки команд.

pub use client_core::*;

/// Tauri-команды этого слоя. В корне библиотеки `#[tauri::command]`
/// конфликтует с собственными макросами, поэтому живут в модуле.
pub mod commands {
    /// Смена языка: фронт зовёт при старте и при переключении.
    #[tauri::command]
    pub fn set_language(lang: String) {
        client_core::i18n::set_language(lang);
    }
}
