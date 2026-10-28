//! Готовые типы нод и реестр для загрузки из JSON.

pub mod button;
pub mod debug;
pub mod label;
pub mod list;
pub mod members;
pub mod servers;
pub mod voice;
pub mod window;

use crate::node::Node;
use crate::store::NodeSpec;

/// Собрать ноду по описанию. Неизвестные типы пропускаем — так старые
/// раскладки не ломают запуск после удаления типа.
pub fn build(spec: &NodeSpec) -> Option<Box<dyn Node>> {
    match spec.kind.as_str() {
        "window" => Some(window::WindowNode::from_spec(spec)),
        "label" => Some(label::LabelNode::from_spec(spec)),
        "button" => Some(button::ButtonNode::from_spec(spec)),
        "list" => Some(list::ListNode::from_spec(spec)),
        "servers" => Some(servers::ServersNode::from_spec(spec)),
        "voice" => Some(voice::VoiceNode::from_spec(spec)),
        "members" => Some(members::MembersNode::from_spec(spec)),
        "debug" => Some(debug::DebugNode::from_spec(spec)),
        other => {
            eprintln!("ui-tree: неизвестный тип ноды '{other}', пропускаю");
            None
        }
    }
}

pub fn build_children(spec: &NodeSpec) -> Vec<Box<dyn Node>> {
    spec.children.iter().filter_map(build).collect()
}
