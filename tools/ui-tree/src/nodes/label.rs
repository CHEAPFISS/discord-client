//! Текстовая нода.

use egui::{Align2, FontId, Pos2, Ui, Vec2};

use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

pub struct LabelNode {
    pub base: BaseNode,
    pub text: String,
    pub font_size: f32,
}

impl LabelNode {
    pub fn new(id: impl Into<String>, text: impl Into<String>, pos: Pos2, font_size: f32) -> Self {
        Self {
            base: BaseNode::new(id, pos, Vec2::new(0.0, font_size * 1.4)),
            text: text.into(),
            font_size,
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Box<dyn Node> {
        let font_size = spec
            .props
            .get("font_size")
            .and_then(|v| v.as_f64())
            .unwrap_or(14.0) as f32;
        let mut node = Self {
            base: BaseNode::from_spec(spec),
            text: spec
                .props
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            font_size,
        };
        node.base.children = build_children(spec);
        Box::new(node)
    }
}

impl Node for LabelNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "label"
    }

    fn props(&self) -> serde_json::Value {
        serde_json::json!({ "text": self.text, "font_size": self.font_size })
    }

    fn draw(&mut self, ui: &mut Ui, world: World, _ctx: &mut NodeCtx) {
        ui.painter().text(
            world.origin,
            Align2::LEFT_TOP,
            &self.text,
            FontId::proportional((self.font_size * world.scale).max(6.0)),
            theme::TEXT,
        );
    }
}
