//! ui-tree-probe: мок-прототип конструктора окон на дереве нод.
//!
//! Главный элемент — канвас. На нём лежат окна, внутри окон — другие ноды.
//! У каждой ноды локальная позиция и масштаб, которые наследуются детьми.
//! Раскладка сохраняется в `~/.config/discord-client/layout.json`.

mod canvas;
mod mock;
mod node;
mod nodes;
mod store;
mod theme;

use std::time::{Duration, Instant};

use eframe::egui;

use node::Stats;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([800.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "UI Tree — Discord mock",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct App {
    canvas: canvas::Canvas,
    stats: Stats,
    last_mem: Instant,
    last_save: Instant,
    /// `UI_TREE_WIGGLE=1` — каждый кадр меняет масштаб холста.
    /// Это нагрузочный кейс для замера лагов зума.
    wiggle: bool,
    start: Instant,
    frames: u32,
    frame_sum: f32,
    frame_max: f32,
    report: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = theme::BG;
        visuals.window_fill = theme::PANEL;
        visuals.extreme_bg_color = theme::BG;
        visuals.faint_bg_color = theme::RAISED;
        visuals.override_text_color = Some(theme::TEXT);
        visuals.selection.bg_fill = theme::ACCENT;
        visuals.hyperlink_color = theme::ACCENT_LIGHT;
        cc.egui_ctx.set_visuals(visuals);
        // Свой зум холста не должен конфликтовать с глобальным Ctrl+±.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let (rss, pss) = read_mem();
        Self {
            canvas: canvas::Canvas::new(),
            stats: Stats {
                pss_mb: pss,
                rss_mb: rss,
                node_count: 0,
                zoom: 1.0,
                frame_ms: 0.0,
            },
            last_mem: Instant::now(),
            last_save: Instant::now(),
            wiggle: std::env::var_os("UI_TREE_WIGGLE").is_some(),
            start: Instant::now(),
            frames: 0,
            frame_sum: 0.0,
            frame_max: 0.0,
            report: Instant::now(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Нагрузочный кейс: масштаб меняется каждый кадр, как при зуме колесом.
        if self.wiggle {
            let t = self.start.elapsed().as_secs_f32();
            self.canvas.view_scale = 0.7 + 0.3 * (t * 2.0).sin();
            ui.ctx().request_repaint();
        }

        if self.last_mem.elapsed() >= Duration::from_secs(1) {
            let (rss, pss) = read_mem();
            self.stats.rss_mb = rss;
            self.stats.pss_mb = pss;
            self.last_mem = Instant::now();
        }
        self.stats.zoom = self.canvas.view_scale;
        self.stats.frame_ms = ui.input(|i| i.unstable_dt) * 1000.0;
        self.canvas.render(ui, &self.stats);

        if self.wiggle {
            self.frames += 1;
            self.frame_sum += self.stats.frame_ms;
            self.frame_max = self.frame_max.max(self.stats.frame_ms);
            if self.report.elapsed() >= Duration::from_secs(2) {
                eprintln!(
                    "wiggle: кадров {} · avg {:.1} ms · max {:.1} ms",
                    self.frames,
                    self.frame_sum / self.frames.max(1) as f32,
                    self.frame_max
                );
                self.frames = 0;
                self.frame_sum = 0.0;
                self.frame_max = 0.0;
                self.report = Instant::now();
            }
        }

        // Автосохранение: не чаще раза в секунду и только при изменениях.
        let save_due = self.canvas.save_requested
            || (self.canvas.dirty && self.last_save.elapsed() >= Duration::from_secs(1));
        if save_due {
            match store::save(&self.canvas.nodes) {
                Ok(()) => {
                    self.canvas.dirty = false;
                    self.canvas.save_requested = false;
                    self.last_save = Instant::now();
                }
                Err(err) => eprintln!("ui-tree: не удалось сохранить раскладку: {err}"),
            }
        }

        // Раз в секунду обновляем цифры памяти, даже когда окно простаивает.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = store::save(&self.canvas.nodes);
    }
}

/// RSS и PSS текущего процесса в MB (Linux).
fn read_mem() -> (f64, f64) {
    let mut rss = 0.0;
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                rss = value
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 1024.0;
            }
        }
    }
    let mut pss = 0.0;
    if let Ok(rollup) = std::fs::read_to_string("/proc/self/smaps_rollup") {
        for line in rollup.lines() {
            if let Some(value) = line.strip_prefix("Pss:") {
                pss = value
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 1024.0;
            }
        }
    }
    (rss, pss)
}
