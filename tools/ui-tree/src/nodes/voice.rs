//! Список голосовых каналов и кто в них сидит (заглушки).

use egui::{Align2, CornerRadius, FontId, Id, Sense, Ui, Vec2};

use crate::mock;
use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

pub struct VoiceNode {
    pub base: BaseNode,
}

impl VoiceNode {
    pub fn new(id: impl Into<String>, pos: egui::Pos2, size: Vec2) -> Self {
        Self {
            base: BaseNode::new(id, pos, size),
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Box<dyn Node> {
        let mut node = Self {
            base: BaseNode::from_spec(spec),
        };
        node.base.children = build_children(spec);
        Box::new(node)
    }
}

impl Node for VoiceNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "voice"
    }

    fn draw(&mut self, ui: &mut Ui, world: World, _ctx: &mut NodeCtx) {
        let scale = world.scale;
        let painter = ui.painter().clone();
        let mut y = 0.0;

        painter.text(
            world.point(Vec2::new(0.0, y)),
            Align2::LEFT_TOP,
            "ГОЛОСОВЫЕ КАНАЛЫ",
            FontId::proportional((10.0 * scale).max(5.0)),
            theme::MUTED,
        );
        y += 20.0;

        for channel in mock::VOICE_CHANNELS {
            if y >= self.base.size.y {
                break;
            }
            let row = world.rect(Vec2::new(0.0, y), Vec2::new(self.base.size.x, 24.0));
            let response = ui.interact(
                row,
                Id::unique((self.base.id.as_str(), channel.name)),
                Sense::click(),
            );
            if response.hovered() {
                painter.rect_filled(
                    row,
                    CornerRadius::same(theme::radius(4.0 * scale)),
                    theme::HOVER,
                );
            }
            painter.text(
                row.left_center() + Vec2::new(4.0 * scale, 0.0),
                Align2::LEFT_CENTER,
                "▾",
                FontId::proportional((11.0 * scale).max(5.0)),
                theme::MUTED,
            );
            painter.text(
                row.left_center() + Vec2::new(20.0 * scale, 0.0),
                Align2::LEFT_CENTER,
                channel.name,
                FontId::proportional((13.0 * scale).max(6.0)),
                theme::ACCENT_LIGHT,
            );
            y += 24.0;

            for user in channel.users {
                if y >= self.base.size.y {
                    break;
                }
                let row = world.rect(Vec2::new(28.0, y), Vec2::new(self.base.size.x - 28.0, 20.0));
                painter.circle_filled(
                    world.point(Vec2::new(36.0, y + 10.0)),
                    (3.0 * scale).max(1.0),
                    theme::ONLINE,
                );
                painter.text(
                    row.left_center(),
                    Align2::LEFT_CENTER,
                    *user,
                    FontId::proportional((12.0 * scale).max(6.0)),
                    theme::TEXT,
                );
                y += 20.0;
            }
            y += 8.0;
        }
    }
}
