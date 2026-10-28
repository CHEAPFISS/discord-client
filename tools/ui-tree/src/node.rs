//! Ядро дерева нод: трансформации, общие поля и трейт `Node`.
//!
//! Модель как в Godot: у каждой ноды есть локальная позиция и масштаб
//! относительно родителя. Мир ребёнка считается как `parent * local`,
//! поэтому масштаб родителя автоматически наследуется детьми.

use egui::{Pos2, Rect, Ui, Vec2};

use crate::store::NodeSpec;

/// Наследуемая трансформация: где находится локальное начало координат
/// ноды на экране и во сколько раз увеличен её локальный юнит.
#[derive(Clone, Copy, Debug)]
pub struct World {
    pub origin: Pos2,
    pub scale: f32,
}

impl World {
    /// Мир ребёнка: позиция умножается на масштаб родителя, масштабы
    /// перемножаются. Это единственное место, где происходит наследование.
    pub fn child(self, base: &BaseNode) -> Self {
        Self {
            origin: self.origin + base.pos.to_vec2() * self.scale,
            scale: self.scale * base.scale,
        }
    }

    pub fn point(self, local: Vec2) -> Pos2 {
        self.origin + local * self.scale
    }

    pub fn rect(self, min: Vec2, size: Vec2) -> Rect {
        Rect::from_min_size(self.origin + min * self.scale, size * self.scale)
    }
}

/// Общие поля любой ноды — аналог `Node2D`/`Control` в Godot.
pub struct BaseNode {
    pub id: String,
    /// Локальная позиция внутри родителя (не в экранных пикселях).
    pub pos: Pos2,
    /// Локальный масштаб; наследуется детьми.
    pub scale: f32,
    /// Локальный размер для хит-теста и раскладки.
    pub size: Vec2,
    pub visible: bool,
    /// `false` — канвас удалит ноду в конце кадра.
    pub exists: bool,
    pub children: Vec<Box<dyn Node>>,
    /// Нода просит поднять её наверх (окна при клике).
    pub focus_request: bool,
}

impl BaseNode {
    pub fn new(id: impl Into<String>, pos: Pos2, size: Vec2) -> Self {
        Self {
            id: id.into(),
            pos,
            scale: 1.0,
            size,
            visible: true,
            exists: true,
            children: Vec::new(),
            focus_request: false,
        }
    }

    pub fn from_spec(spec: &NodeSpec) -> Self {
        Self {
            id: spec.id.clone(),
            pos: Pos2::new(spec.pos[0], spec.pos[1]),
            scale: spec.scale.max(0.01),
            size: Vec2::new(spec.size[0], spec.size[1]),
            visible: true,
            exists: true,
            children: Vec::new(),
            focus_request: false,
        }
    }
}

/// Показатели, которые приложение собирает раз в секунду и отдаёт нодам.
#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub pss_mb: f64,
    pub rss_mb: f64,
    pub node_count: usize,
    pub zoom: f32,
}

/// Команды нод канвасу. Нода не трогает чужие ноды напрямую — она просит.
pub enum Command {
    ResetLayout,
    AddWindow,
    ZoomBy(f32),
    SaveNow,
}

/// Контекст кадра: статистика, лог действий и команды канвасу.
pub struct NodeCtx {
    pub stats: Stats,
    pub last_action: Option<String>,
    /// Действия, нажатые в этом кадре.
    pub actions: Vec<String>,
    pub commands: Vec<Command>,
    pub dirty: bool,
}

/// Любой элемент UI: окно, список, кнопка, канвас.
pub trait Node {
    fn base(&self) -> &BaseNode;
    fn base_mut(&mut self) -> &mut BaseNode;
    fn kind(&self) -> &'static str;

    /// Нарисовать себя в своём локальном мире.
    fn draw(&mut self, ui: &mut Ui, world: World, ctx: &mut NodeCtx);

    /// Состояние для `layout.json`.
    fn props(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Забрать запрос на поднятие наверх.
    fn take_focus(&mut self) -> bool {
        false
    }

    /// Общий рендер: посчитать свой мир, нарисовать себя, затем детей.
    fn show(&mut self, ui: &mut Ui, parent: World, ctx: &mut NodeCtx) {
        if !self.base().visible {
            return;
        }
        let world = parent.child(self.base());
        self.draw(ui, world, ctx);
        for child in &mut self.base_mut().children {
            child.show(ui, world, ctx);
        }
    }
}

/// Сериализация ноды в спеку (рекурсивно).
pub fn save_node(node: &dyn Node) -> NodeSpec {
    let base = node.base();
    NodeSpec {
        kind: node.kind().to_string(),
        id: base.id.clone(),
        pos: [base.pos.x, base.pos.y],
        scale: base.scale,
        size: [base.size.x, base.size.y],
        props: node.props(),
        children: base
            .children
            .iter()
            .map(|child| save_node(child.as_ref()))
            .collect(),
    }
}

pub fn count_nodes(nodes: &[Box<dyn Node>]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_nodes(&node.base().children))
        .sum()
}
