//! Рейл серверов (заглушки).

use egui::{Align2, CornerRadius, Id, Sense, Ui, Vec2};

use crate::mock;
use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

const ITEM: f32 = 42.0;
const GAP: f32 = 10.0;

pub struct ServersNode {
    pub base: BaseNode,
    pub selected: usize,
}

impl ServersNode {
    pub fn new(id: impl Into<String>, pos: egui::Pos2, size: Vec2) -> Self {
        Self {
            base: BaseNode::new(id, pos, size),
            selected: 0,
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Box<dyn Node> {
        let selected = spec
            .props
            .get("selected")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let mut node = Self {
            base: BaseNode::from_spec(spec),
            selected,
        };
        node.base.children = build_children(spec);
        Box::new(node)
    }
}

impl Node for ServersNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "servers"
    }

    fn props(&self) -> serde_json::Value {
        serde_json::json!({ "selected": self.selected })
    }

    fn draw(&mut self, ui: &mut Ui, world: World, ctx: &mut NodeCtx) {
        let scale = world.scale;
        let painter = ui.painter().clone();
        let x = (self.base.size.x - ITEM) / 2.0;

        for (i, name) in mock::SERVERS.iter().enumerate() {
            let y = i as f32 * (ITEM + GAP);
            if y >= self.base.size.y {
                break;
            }
            let rect = world.rect(Vec2::new(x, y), Vec2::splat(ITEM));
            let response =
                ui.interact(rect, Id::unique((self.base.id.as_str(), i)), Sense::click());
            let selected = i == self.selected;
            let fill = if selected {
                theme::ACCENT
            } else if response.hovered() {
                theme::HOVER
            } else {
                theme::RAISED
            };
            let rounding =
                CornerRadius::same(theme::radius(if selected { 14.0 } else { 10.0 } * scale));
            painter.rect_filled(rect, rounding, fill);

            if selected {
                // Акцентная полоска слева, как в Discord.
                let bar = egui::Rect::from_min_size(
                    egui::pos2(rect.left() - 8.0 * scale, rect.center().y - 12.0 * scale),
                    Vec2::new(3.0 * scale, 24.0 * scale),
                );
                painter.rect_filled(bar, CornerRadius::same(2), theme::ACCENT_LIGHT);
            }

            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                name,
                theme::font(15.0, scale),
                if selected {
                    egui::Color32::BLACK
                } else {
                    theme::TEXT
                },
            );

            if response.clicked() {
                self.selected = i;
                ctx.dirty = true;
            }
        }
    }
}
