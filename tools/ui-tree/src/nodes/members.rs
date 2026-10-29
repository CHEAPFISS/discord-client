//! Участники сервера: онлайн/офлайн (заглушки).

use egui::{Align2, Ui, Vec2};

use crate::mock;
use crate::node::{BaseNode, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

pub struct MembersNode {
    pub base: BaseNode,
}

impl MembersNode {
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

impl Node for MembersNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "members"
    }

    fn draw(&mut self, ui: &mut Ui, world: World, _ctx: &mut NodeCtx) {
        let scale = world.scale;
        let painter = ui.painter().clone();
        let mut y = 0.0;

        section(
            &painter,
            world,
            &mut y,
            &format!("В СЕТИ — {}", mock::MEMBERS_ONLINE.len()),
            scale,
        );
        for member in mock::MEMBERS_ONLINE {
            member_row(&painter, world, &mut y, member.name, member.status, scale);
        }

        y += 10.0;
        section(
            &painter,
            world,
            &mut y,
            &format!("НЕ В СЕТИ — {}", mock::MEMBERS_OFFLINE.len()),
            scale,
        );
        for member in mock::MEMBERS_OFFLINE {
            member_row(&painter, world, &mut y, member.name, member.status, scale);
        }
    }
}

fn section(painter: &egui::Painter, world: World, y: &mut f32, text: &str, scale: f32) {
    painter.text(
        world.point(Vec2::new(0.0, *y)),
        Align2::LEFT_TOP,
        text,
        theme::font(10.0, scale),
        theme::MUTED,
    );
    *y += 18.0;
}

fn member_row(
    painter: &egui::Painter,
    world: World,
    y: &mut f32,
    name: &str,
    status: egui::Color32,
    scale: f32,
) {
    painter.circle_filled(
        world.point(Vec2::new(5.0, *y + 9.0)),
        (3.5 * scale).max(1.0),
        status,
    );
    painter.text(
        world.point(Vec2::new(16.0, *y)),
        Align2::LEFT_TOP,
        name,
        theme::font(13.0, scale),
        theme::TEXT,
    );
    *y += 22.0;
}
