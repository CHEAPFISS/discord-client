//! egui-probe: макет чата на egui + живой замер RSS/PSS.
//!
//! Цель — понять, сколько памяти ест нативный UI без WebKit. Это не клиент:
//! ни сети, ни голоса, ни картинок; только текст и раскладка.

use std::time::{Duration, Instant};

use eframe::egui;

const BG: egui::Color32 = egui::Color32::from_rgb(18, 18, 18);
const PANEL: egui::Color32 = egui::Color32::from_rgb(26, 26, 26);
const PANEL_2: egui::Color32 = egui::Color32::from_rgb(32, 32, 32);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0, 172, 193);
const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(77, 208, 225);
const TEXT: egui::Color32 = egui::Color32::from_rgb(235, 235, 235);
const TEXT_LOW: egui::Color32 = egui::Color32::from_rgb(150, 150, 150);

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([800.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "egui memory probe",
        options,
        Box::new(|cc| Ok(Box::new(ProbeApp::new(cc)))),
    )
}

struct ProbeApp {
    input: String,
    messages: Vec<(String, String, String)>,
    last_mem: Instant,
    rss_mb: f64,
    pss_mb: f64,
}

impl ProbeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = egui::Color32::from_rgb(12, 12, 12);
        visuals.faint_bg_color = PANEL_2;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = ACCENT;
        visuals.hyperlink_color = ACCENT_LIGHT;
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(45, 45, 45);
        visuals.widgets.active.bg_fill = ACCENT;
        cc.egui_ctx.set_visuals(visuals);

        let (rss_mb, pss_mb) = read_mem();
        Self {
            input: String::new(),
            messages: demo_messages(),
            last_mem: Instant::now(),
            rss_mb,
            pss_mb,
        }
    }
}

impl eframe::App for ProbeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_mem.elapsed() >= Duration::from_secs(1) {
            let (rss, pss) = read_mem();
            self.rss_mb = rss;
            self.pss_mb = pss;
            self.last_mem = Instant::now();
        }
        // Обновляем цифры, даже когда окно простаивает.
        ui.ctx().request_repaint_after(Duration::from_secs(1));

        egui::Panel::top("top")
            .exact_size(48.0)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(PANEL))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("# general").strong().size(16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "RSS {:.1} MB  ·  PSS {:.1} MB",
                                self.rss_mb, self.pss_mb
                            ))
                            .color(ACCENT_LIGHT)
                            .monospace(),
                        );
                    });
                });
            });

        egui::Panel::left("rail")
            .exact_size(72.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(BG))
            .show(ui, |ui| {
                ui.add_space(12.0);
                for (i, name) in ["D", "W", "M", "G", "L"].iter().enumerate() {
                    let selected = i == 0;
                    let button = egui::Button::new(
                        egui::RichText::new(*name).size(18.0).color(if selected {
                            egui::Color32::BLACK
                        } else {
                            TEXT
                        }),
                    )
                    .fill(if selected { ACCENT } else { PANEL_2 })
                    .min_size(egui::vec2(44.0, 44.0));
                    if ui.add(button).clicked() {
                        // Заглушка: в реальном клиенте здесь выбор сервера.
                    }
                    ui.add_space(8.0);
                }
            });

        egui::Panel::left("channels")
            .exact_size(220.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(PANEL))
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("TEXT CHANNELS")
                        .size(11.0)
                        .color(TEXT_LOW),
                );
                ui.add_space(6.0);
                for (i, name) in ["general", "off-topic", "dev", "media", "random"]
                    .iter()
                    .enumerate()
                {
                    if ui
                        .selectable_label(
                            i == 0,
                            egui::RichText::new(format!("# {name}")).size(14.0),
                        )
                        .clicked()
                    {
                        // Заглушка.
                    }
                }
                ui.add_space(14.0);
                ui.label(egui::RichText::new("VOICE").size(11.0).color(TEXT_LOW));
                ui.add_space(6.0);
                ui.label(egui::RichText::new("🔊 General").size(14.0));
                ui.label(egui::RichText::new("🔊 Gaming").size(14.0));
            });

        egui::Panel::right("members")
            .exact_size(220.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(PANEL))
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("ONLINE — 5").size(11.0).color(TEXT_LOW));
                ui.add_space(6.0);
                for (name, color) in [
                    ("vlad", egui::Color32::from_rgb(0, 200, 120)),
                    ("mira", egui::Color32::from_rgb(255, 180, 0)),
                    ("kot", egui::Color32::from_rgb(0, 172, 193)),
                    ("nix", egui::Color32::from_rgb(150, 150, 150)),
                    ("dev", egui::Color32::from_rgb(200, 80, 80)),
                ] {
                    ui.horizontal(|ui| {
                        ui.colored_label(color, "●");
                        ui.label(egui::RichText::new(name).size(14.0));
                    });
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (author, time, text) in &self.messages {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(author)
                                        .strong()
                                        .color(ACCENT_LIGHT)
                                        .size(14.0),
                                );
                                ui.label(egui::RichText::new(time).size(11.0).color(TEXT_LOW));
                            });
                            ui.label(egui::RichText::new(text).size(14.0));
                            ui.add_space(8.0);
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.input)
                            .hint_text("Message")
                            .desired_width(f32::INFINITY),
                    );
                    if ui.button("Send").clicked() && !self.input.trim().is_empty() {
                        self.messages
                            .push(("you".into(), "now".into(), self.input.clone()));
                        self.input.clear();
                    }
                });
            });
    }
}

fn demo_messages() -> Vec<(String, String, String)> {
    (0..30)
        .map(|i| {
            let author = ["vlad", "mira", "kot", "nix", "dev"][i % 5];
            (
                author.to_string(),
                format!("12:{:02}", i % 60),
                format!(
                    "Тестовое сообщение #{i}: проверяем раскладку чата, переносы строк и скролл."
                ),
            )
        })
        .collect()
}

/// RSS и PSS текущего процесса в MB (Linux).
fn read_mem() -> (f64, f64) {
    let mut rss = 0.0;
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("VmRSS:") {
                rss = v
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
            if let Some(v) = line.strip_prefix("Pss:") {
                pss = v
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
