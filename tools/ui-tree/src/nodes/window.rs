//! Окно: рамка, заголовок, перетаскивание, закрытие и свой масштаб.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Id, Painter, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

pub struct WindowNode {
    pub base: BaseNode,
    pub title: String,
}

impl WindowNode {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        pos: egui::Pos2,
        size: Vec2,
    ) -> Self {
        Self {
            base: BaseNode::new(id, pos, size),
            title: title.into(),
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Box<dyn Node> {
        let mut node = Self {
            base: BaseNode::from_spec(spec),
            title: spec
                .props
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Окно")
                .to_string(),
        };
        node.base.children = build_children(spec);
        Box::new(node)
    }
}

impl Node for WindowNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "window"
    }

    fn props(&self) -> serde_json::Value {
        serde_json::json!({ "title": self.title })
    }

    fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.base.focus_request)
    }

    fn draw(&mut self, ui: &mut Ui, world: World, ctx: &mut NodeCtx) {
        let painter = ui.painter().clone();
        let rect = world.rect(Vec2::ZERO, self.base.size);
        let scale = world.scale;
        let rounding = CornerRadius::same(theme::radius(8.0 * scale));

        // Тень и рамка окна.
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, 4.0 * scale)),
            rounding,
            Color32::from_black_alpha(90),
        );
        painter.rect_filled(rect, rounding, theme::PANEL);
        painter.rect_stroke(
            rect,
            rounding,
            Stroke::new(1.0 * scale, theme::BORDER),
            StrokeKind::Inside,
        );

        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), theme::TITLE_H * scale));
        let header_rounding = CornerRadius {
            nw: rounding.nw,
            ne: rounding.ne,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(header, header_rounding, theme::RAISED);
        painter.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0 * scale, theme::BORDER),
        );
        // Бирюзовая полоска под заголовком — акцент Material.
        painter.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(2.0 * scale, theme::ACCENT),
        );

        let title_font = theme::font(14.0, scale);
        painter.text(
            header.left_center() + Vec2::new(12.0 * scale, 0.0),
            Align2::LEFT_CENTER,
            &self.title,
            title_font,
            theme::TEXT,
        );

        // Клик по телу окна поднимает его наверх.
        let focus = ui.interact(
            rect,
            Id::unique((self.base.id.as_str(), "focus")),
            Sense::click(),
        );
        if focus.is_pointer_button_down_on() {
            self.base.focus_request = true;
        }

        // Перетаскивание за заголовок. Дельта экранная, поэтому делим
        // на масштаб родителя: позиция ноды живёт в координатах родителя.
        let drag = ui.interact(
            header,
            Id::unique((self.base.id.as_str(), "title")),
            Sense::drag(),
        );
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(if drag.dragged() {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
        }
        if drag.is_pointer_button_down_on() {
            self.base.focus_request = true;
        }
        if drag.dragged() {
            let parent_scale = (world.scale / self.base.scale.max(0.01)).max(0.01);
            self.base.pos += drag.drag_delta() / parent_scale;
            ctx.dirty = true;
        }

        // Кнопки заголовка: −, +, ×. Регистрируются после drag,
        // поэтому получают клик первыми.
        let b = 24.0;
        let gap = 4.0;
        let y = (theme::TITLE_H - b) / 2.0;
        let close_x = self.base.size.x - b - 8.0;
        let plus_x = close_x - b - gap;
        let minus_x = plus_x - b - gap;

        let close = world.rect(Vec2::new(close_x, y), Vec2::splat(b));
        if title_button(
            ui,
            &painter,
            close,
            (self.base.id.as_str(), "close"),
            "×",
            scale,
        ) {
            self.base.exists = false;
            ctx.dirty = true;
        }

        let plus = world.rect(Vec2::new(plus_x, y), Vec2::splat(b));
        if title_button(
            ui,
            &painter,
            plus,
            (self.base.id.as_str(), "plus"),
            "+",
            scale,
        ) {
            self.base.scale = (self.base.scale * 1.1).clamp(0.3, 3.0);
            ctx.dirty = true;
        }

        let minus = world.rect(Vec2::new(minus_x, y), Vec2::splat(b));
        if title_button(
            ui,
            &painter,
            minus,
            (self.base.id.as_str(), "minus"),
            "−",
            scale,
        ) {
            self.base.scale = (self.base.scale / 1.1).clamp(0.3, 3.0);
            ctx.dirty = true;
        }
    }
}

fn title_button(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    scale: f32,
) -> bool {
    let response = ui.interact(rect, Id::unique(id), Sense::click());
    if response.hovered() {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::radius(4.0 * scale)),
            theme::HOVER,
        );
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::font(15.0, scale),
        if response.hovered() {
            theme::TEXT
        } else {
            theme::MUTED
        },
    );
    response.clicked()
}
