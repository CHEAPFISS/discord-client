//! Кнопка: рисует себя и отправляет действие в лог.

use egui::{Align2, Color32, CornerRadius, Id, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

pub struct ButtonNode {
    pub base: BaseNode,
    pub text: String,
    pub action: String,
}

impl ButtonNode {
    pub fn new(
        id: impl Into<String>,
        text: impl Into<String>,
        action: impl Into<String>,
        pos: egui::Pos2,
        size: Vec2,
    ) -> Self {
        Self {
            base: BaseNode::new(id, pos, size),
            text: text.into(),
            action: action.into(),
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Box<dyn Node> {
        let mut node = Self {
            base: BaseNode::from_spec(spec),
            text: spec
                .props
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("Кнопка")
                .to_string(),
            action: spec
                .props
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("noop")
                .to_string(),
        };
        node.base.children = build_children(spec);
        Box::new(node)
    }
}

impl Node for ButtonNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "button"
    }

    fn props(&self) -> serde_json::Value {
        serde_json::json!({ "text": self.text, "action": self.action })
    }

    fn draw(&mut self, ui: &mut Ui, world: World, ctx: &mut NodeCtx) {
        let rect = world.rect(Vec2::ZERO, self.base.size);
        let scale = world.scale;
        let response = ui.interact(
            rect,
            Id::unique((self.base.id.as_str(), "btn")),
            Sense::click(),
        );
        let fill = if response.hovered() {
            theme::ACCENT_LIGHT
        } else {
            theme::RAISED
        };
        let rounding = CornerRadius::same(theme::radius(5.0 * scale));
        let painter = ui.painter();
        painter.rect_filled(rect, rounding, fill);
        painter.rect_stroke(
            rect,
            rounding,
            Stroke::new(1.0 * scale, theme::ACCENT),
            StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            &self.text,
            theme::font(13.0, scale),
            if response.hovered() {
                Color32::BLACK
            } else {
                theme::TEXT
            },
        );
        if response.clicked() {
            ctx.actions.push(self.action.clone());
        }
    }
}
