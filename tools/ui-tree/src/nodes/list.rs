//! Простой вертикальный список строк с выбором.

use egui::{Align2, CornerRadius, FontId, Id, Sense, Ui, Vec2};

use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

const ROW_H: f32 = 26.0;

pub struct ListNode {
    pub base: BaseNode,
    pub items: Vec<String>,
    pub selected: usize,
}

impl ListNode {
    pub fn new(id: impl Into<String>, items: Vec<String>, pos: egui::Pos2, size: Vec2) -> Self {
        Self {
            base: BaseNode::new(id, pos, size),
            items,
            selected: 0,
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Box<dyn Node> {
        let items = spec
            .props
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let selected = spec
            .props
            .get("selected")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let mut node = Self {
            base: BaseNode::from_spec(spec),
            items,
            selected,
        };
        node.base.children = build_children(spec);
        Box::new(node)
    }
}

impl Node for ListNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "list"
    }

    fn props(&self) -> serde_json::Value {
        serde_json::json!({ "items": self.items, "selected": self.selected })
    }

    fn draw(&mut self, ui: &mut Ui, world: World, ctx: &mut NodeCtx) {
        let scale = world.scale;
        let painter = ui.painter().clone();
        for (i, item) in self.items.iter().enumerate() {
            let y = i as f32 * ROW_H;
            if y >= self.base.size.y {
                break;
            }
            let row = world.rect(Vec2::new(0.0, y), Vec2::new(self.base.size.x, ROW_H));
            let response = ui.interact(row, Id::unique((self.base.id.as_str(), i)), Sense::click());
            let selected = i == self.selected;
            if selected || response.hovered() {
                painter.rect_filled(
                    row,
                    CornerRadius::same(theme::radius(4.0 * scale)),
                    if selected {
                        theme::RAISED
                    } else {
                        theme::HOVER
                    },
                );
            }
            painter.text(
                row.left_center() + Vec2::new(8.0 * scale, 0.0),
                Align2::LEFT_CENTER,
                item,
                FontId::proportional((13.0 * scale).max(6.0)),
                if selected { theme::TEXT } else { theme::MUTED },
            );
            if response.clicked() {
                self.selected = i;
                ctx.dirty = true;
            }
        }
    }
}
