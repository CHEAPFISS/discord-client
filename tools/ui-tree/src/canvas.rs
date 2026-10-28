//! Корневая нода-канвас: фон, сетка, зум/пан и рендер дерева.

use egui::{Id, Pos2, Rect, Sense, Ui, Vec2};

use crate::mock;
use crate::node::{count_nodes, Command, Node, NodeCtx, Stats, World};
use crate::nodes::label::LabelNode;
use crate::nodes::window::WindowNode;
use crate::store;
use crate::theme;

pub struct Canvas {
    pub nodes: Vec<Box<dyn Node>>,
    /// Экранная позиция точки (0,0) канваса.
    pub view_origin: Pos2,
    pub view_scale: f32,
    pub actions: Vec<String>,
    pub dirty: bool,
    pub save_requested: bool,
    id_counter: u64,
}

impl Canvas {
    pub fn new() -> Self {
        let nodes = store::load()
            .map(|specs| {
                specs
                    .iter()
                    .filter_map(crate::nodes::build)
                    .collect::<Vec<Box<dyn Node>>>()
            })
            .filter(|nodes| !nodes.is_empty())
            .unwrap_or_else(mock::default_layout);
        Self {
            nodes,
            view_origin: Pos2::ZERO,
            view_scale: 1.0,
            actions: Vec::new(),
            dirty: false,
            save_requested: false,
            id_counter: now_millis(),
        }
    }

    pub fn reset(&mut self) {
        self.nodes = mock::default_layout();
        self.view_origin = Pos2::ZERO;
        self.view_scale = 1.0;
        self.dirty = true;
    }

    pub fn add_window(&mut self) {
        let offset = (self.nodes.len() as f32 * 24.0) % 160.0;
        let mut win = WindowNode::new(
            self.next_id("win"),
            "Новое окно",
            Pos2::new(80.0 + offset, 80.0 + offset),
            Vec2::new(260.0, 170.0),
        );
        win.base.children.push(Box::new(LabelNode::new(
            self.next_id("label"),
            "Пустое окно — добавь ноды",
            Pos2::new(12.0, 44.0),
            14.0,
        )));
        self.nodes.push(Box::new(win));
        self.dirty = true;
    }

    pub fn render(&mut self, ui: &mut Ui, stats: &Stats) {
        let rect = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 0.0, theme::BG);
        self.draw_grid(&painter, rect);

        // Подсказка внизу; окна рисуются поверх.
        painter.text(
            rect.left_bottom() + Vec2::new(12.0, -10.0),
            egui::Align2::LEFT_BOTTOM,
            "Ctrl+колесо — зум · СКМ — панорама",
            egui::FontId::proportional(11.0),
            theme::MUTED,
        );

        // Ctrl+колесо: зум к курсору. Читаем колесо явно, чтобы не зависеть
        // от глобального egui-зума и случайных pinch-событий.
        let (ctrl, scroll, pointer) = ui.input(|i| {
            (
                i.modifiers.ctrl,
                i.smooth_scroll_delta().y,
                i.pointer.hover_pos(),
            )
        });
        if ctrl && scroll.abs() > 0.01 {
            if let Some(pointer) = pointer.filter(|p| rect.contains(*p)) {
                let factor = (scroll * 0.003).exp();
                self.zoom_at(pointer, factor);
            }
        }

        // Средняя кнопка мыши — панорама.
        let (middle_down, delta) = ui.input(|i| (i.pointer.middle_down(), i.pointer.delta()));
        if middle_down {
            self.view_origin += delta;
        }

        // Фон канваса: клики по пустому месту не нужны, но регистрируем
        // сенсор, чтобы окна гарантированно были выше по слою.
        let _ = ui.interact(rect, Id::unique("canvas-bg"), Sense::click());

        let mut ctx = NodeCtx {
            stats: Stats {
                node_count: count_nodes(&self.nodes),
                ..*stats
            },
            last_action: self.actions.last().cloned(),
            actions: Vec::new(),
            commands: Vec::new(),
            dirty: false,
        };

        let world = World {
            origin: self.view_origin,
            scale: self.view_scale,
        };
        let mut focus_id: Option<String> = None;
        for node in &mut self.nodes {
            node.show(ui, world, &mut ctx);
            if node.take_focus() {
                focus_id = Some(node.base().id.clone());
            }
        }

        // Закрытые ноды удаляются в конце кадра.
        let before = self.nodes.len();
        self.nodes.retain(|node| node.base().exists);
        if self.nodes.len() != before {
            ctx.dirty = true;
        }

        // Кликнутое окно — наверх.
        if let Some(id) = focus_id {
            if let Some(index) = self.nodes.iter().position(|node| node.base().id == id) {
                if index + 1 < self.nodes.len() {
                    let node = self.nodes.remove(index);
                    self.nodes.push(node);
                }
            }
        }

        for command in ctx.commands.drain(..) {
            match command {
                Command::ResetLayout => self.reset(),
                Command::AddWindow => self.add_window(),
                Command::ZoomBy(factor) => self.zoom_at(rect.center(), factor),
                Command::SaveNow => self.save_requested = true,
            }
        }

        if ctx.dirty {
            self.dirty = true;
        }

        for action in ctx.actions.drain(..) {
            self.actions.push(action);
        }
        if self.actions.len() > 20 {
            let extra = self.actions.len() - 20;
            self.actions.drain(0..extra);
        }
    }

    fn next_id(&mut self, kind: &str) -> String {
        self.id_counter += 1;
        format!("{kind}-{}", self.id_counter)
    }

    fn zoom_at(&mut self, screen: Pos2, factor: f32) {
        let (origin, scale) = zoom_around(self.view_origin, self.view_scale, screen, factor);
        self.view_origin = origin;
        self.view_scale = scale;
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: Rect) {
        const STEP: f32 = 32.0;
        let min = (rect.min - self.view_origin) / self.view_scale;
        let max = (rect.max - self.view_origin) / self.view_scale;
        let x0 = (min.x / STEP).floor() as i32;
        let x1 = (max.x / STEP).ceil() as i32;
        let y0 = (min.y / STEP).floor() as i32;
        let y1 = (max.y / STEP).ceil() as i32;
        if (x1 - x0) as i64 * (y1 - y0) as i64 > 20_000 {
            return;
        }
        let radius = (1.2 * self.view_scale).clamp(0.6, 2.0);
        for ix in x0..=x1 {
            for iy in y0..=y1 {
                let point = self.view_origin
                    + Vec2::new(ix as f32 * STEP, iy as f32 * STEP) * self.view_scale;
                painter.circle_filled(point, radius, theme::GRID);
            }
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Зум вокруг экранной точки: мировая точка под курсором не должна
/// сдвигаться. Вынесено отдельно, чтобы покрыть тестом.
fn zoom_around(origin: Pos2, scale: f32, screen: Pos2, factor: f32) -> (Pos2, f32) {
    let new = (scale * factor).clamp(0.25, 3.0);
    if (new - scale).abs() < f32::EPSILON {
        return (origin, scale);
    }
    let origin = screen - (screen - origin) * (new / scale);
    (origin, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let origin = Pos2::new(10.0, 20.0);
        let scale = 1.0;
        let cursor = Pos2::new(100.0, 200.0);
        let world_before = (cursor - origin) / scale;

        let (new_origin, new_scale) = zoom_around(origin, scale, cursor, 2.0);
        let world_after = (cursor - new_origin) / new_scale;

        assert!((world_before - world_after).length() < 1e-4);
        assert!((new_scale - 2.0).abs() < 1e-6);
    }
}
