//! Тёмная Material-тема: только токены, без логики.

use egui::Color32;

pub const BG: Color32 = Color32::from_rgb(18, 18, 18);
pub const PANEL: Color32 = Color32::from_rgb(30, 30, 30);
pub const RAISED: Color32 = Color32::from_rgb(38, 38, 38);
pub const HOVER: Color32 = Color32::from_rgb(50, 50, 50);
pub const BORDER: Color32 = Color32::from_rgb(58, 58, 58);
pub const ACCENT: Color32 = Color32::from_rgb(0, 172, 193);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(77, 208, 225);
pub const TEXT: Color32 = Color32::from_rgb(235, 235, 235);
pub const MUTED: Color32 = Color32::from_rgb(158, 158, 158);
pub const GRID: Color32 = Color32::from_rgb(36, 36, 36);
pub const ONLINE: Color32 = Color32::from_rgb(76, 175, 80);
pub const IDLE: Color32 = Color32::from_rgb(255, 193, 7);
pub const DND: Color32 = Color32::from_rgb(229, 115, 115);

/// Высота заголовка окна в локальных единицах.
pub const TITLE_H: f32 = 34.0;

pub fn radius(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Размер шрифта под масштаб, округлённый до целых пикселей.
///
/// Дробный `FontId` на каждом кадре зума — новый ключ в кэше глифов:
/// epaint заново растирует текст и перезаливает атлас, из-за этого
/// и проседает кадр. Целые пиксели держат кэш тёплым.
pub fn font(base: f32, scale: f32) -> egui::FontId {
    egui::FontId::proportional((base * scale).round().max(5.0))
}

/// Моноширинный шрифт под масштаб (см. [`font`]).
pub fn mono(base: f32, scale: f32) -> egui::FontId {
    egui::FontId::monospace((base * scale).round().max(5.0))
}
