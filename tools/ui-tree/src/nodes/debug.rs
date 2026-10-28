//! Отладочная нода: память, зум, команды канвасу.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Sense, Ui, Vec2};

use crate::node::{BaseNode, Command, Node, NodeCtx, World};
use crate::nodes::build_children;
use crate::store::NodeSpec;
use crate::theme;

pub struct DebugNode {
    pub base: BaseNode,
}

impl DebugNode {
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

impl Node for DebugNode {
    fn base(&self) -> &BaseNode {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseNode {
        &mut self.base
    }

    fn kind(&self) -> &'static str {
        "debug"
    }

    fn draw(&mut self, ui: &mut Ui, world: World, ctx: &mut NodeCtx) {
        let scale = world.scale;
        let painter = ui.painter().clone();
        let mono = FontId::monospace((12.0 * scale).max(6.0));
        let small = FontId::proportional((10.0 * scale).max(5.0));

        painter.text(
            world.point(Vec2::ZERO),
            Align2::LEFT_TOP,
            format!(
                "PSS {:.1} MB · RSS {:.1} MB",
                ctx.stats.pss_mb, ctx.stats.rss_mb
            ),
            mono,
            theme::ACCENT_LIGHT,
        );
        painter.text(
            world.point(Vec2::new(0.0, 18.0)),
            Align2::LEFT_TOP,
            format!("зум {:.2} · нод {}", ctx.stats.zoom, ctx.stats.node_count),
            small.clone(),
            theme::MUTED,
        );
        let action = ctx
            .last_action
            .as_deref()
            .map(|s| truncate(s, 30))
            .unwrap_or_else(|| "—".to_string());
        painter.text(
            world.point(Vec2::new(0.0, 34.0)),
            Align2::LEFT_TOP,
            format!("действие: {action}"),
            small.clone(),
            theme::MUTED,
        );

        let mut x = 0.0;
        let rect = world.rect(Vec2::new(x, 58.0), Vec2::new(96.0, 26.0));
        if action_button(
            ui,
            &painter,
            (self.base.id.as_str(), "add"),
            rect,
            "＋ окно",
            true,
            scale,
        ) {
            ctx.commands.push(Command::AddWindow);
        }
        x += 102.0;
        let rect = world.rect(Vec2::new(x, 58.0), Vec2::new(78.0, 26.0));
        if action_button(
            ui,
            &painter,
            (self.base.id.as_str(), "save"),
            rect,
            "Сохранить",
            false,
            scale,
        ) {
            ctx.commands.push(Command::SaveNow);
        }
        x += 84.0;
        let rect = world.rect(Vec2::new(x, 58.0), Vec2::new(78.0, 26.0));
        if action_button(
            ui,
            &painter,
            (self.base.id.as_str(), "reset"),
            rect,
            "Сброс",
            false,
            scale,
        ) {
            ctx.commands.push(Command::ResetLayout);
        }

        let rect = world.rect(Vec2::new(0.0, 92.0), Vec2::new(26.0, 26.0));
        if action_button(
            ui,
            &painter,
            (self.base.id.as_str(), "zoom-out"),
            rect,
            "−",
            false,
            scale,
        ) {
            ctx.commands.push(Command::ZoomBy(0.9));
        }
        let rect = world.rect(Vec2::new(32.0, 92.0), Vec2::new(26.0, 26.0));
        if action_button(
            ui,
            &painter,
            (self.base.id.as_str(), "zoom-in"),
            rect,
            "+",
            false,
            scale,
        ) {
            ctx.commands.push(Command::ZoomBy(1.1));
        }
        painter.text(
            world.point(Vec2::new(68.0, 105.0)),
            Align2::LEFT_CENTER,
            "масштаб холста",
            small.clone(),
            theme::MUTED,
        );

        painter.text(
            world.point(Vec2::new(0.0, 132.0)),
            Align2::LEFT_TOP,
            "Ctrl+колесо — зум к курсору\nСКМ — панорама холста",
            small,
            theme::MUTED,
        );
    }
}

fn action_button(
    ui: &mut Ui,
    painter: &egui::Painter,
    id: (&str, &str),
    rect: egui::Rect,
    label: &str,
    accent: bool,
    scale: f32,
) -> bool {
    let response = ui.interact(rect, Id::unique(id), Sense::click());
    let fill = if accent {
        if response.hovered() {
            theme::ACCENT_LIGHT
        } else {
            theme::ACCENT
        }
    } else if response.hovered() {
        theme::HOVER
    } else {
        theme::RAISED
    };
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::radius(5.0 * scale)), fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional((12.0 * scale).max(6.0)),
        if accent { Color32::BLACK } else { theme::TEXT },
    );
    response.clicked()
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}
