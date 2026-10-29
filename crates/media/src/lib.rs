//! Аудио- и видеопайплайн бэкенда.
//!
//! - [`video`] — декод H264 (просмотр демок), даунскейл, JPEG.
//! - [`video_render`] — очередь кадров и decode-таск; кадр уходит в UI через
//!   [`client_core::VideoFrameSink`] (в Tauri это Channel, здесь — трейт).
//! - [`audio`] — захват микрофона, шумодав, микшер и вывод звука.

pub mod audio;
pub mod video;
pub mod video_render;
