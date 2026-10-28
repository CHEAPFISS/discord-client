//! Сохранение раскладки: дерево нод ⇄ `layout.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::node::{save_node, Node};

/// Версия формата. Старые/чужие версии не загружаем — прототип ещё
/// меняется, и лучше показать дефолт, чем сломать дерево.
const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSpec {
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub pos: [f32; 2],
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub size: [f32; 2],
    #[serde(default)]
    pub props: serde_json::Value,
    #[serde(default)]
    pub children: Vec<NodeSpec>,
}

#[derive(Serialize, Deserialize)]
struct Layout {
    version: u32,
    nodes: Vec<NodeSpec>,
}

fn one() -> f32 {
    1.0
}

/// `$XDG_CONFIG_HOME/discord-client/layout.json` или `~/.config/...`.
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("discord-client").join("layout.json")
}

pub fn load() -> Option<Vec<NodeSpec>> {
    load_from(&config_path())
}

fn load_from(path: &std::path::Path) -> Option<Vec<NodeSpec>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Layout>(&text) {
        Ok(layout) if layout.version == VERSION => Some(layout.nodes),
        Ok(layout) => {
            eprintln!(
                "ui-tree: версия раскладки {} не поддерживается (нужна {VERSION})",
                layout.version
            );
            None
        }
        Err(err) => {
            eprintln!("ui-tree: не удалось прочитать {}: {err}", path.display());
            None
        }
    }
}

pub fn save(nodes: &[Box<dyn Node>]) -> std::io::Result<()> {
    save_to(&config_path(), nodes)
}

fn save_to(path: &std::path::Path, nodes: &[Box<dyn Node>]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let layout = Layout {
        version: VERSION,
        nodes: nodes.iter().map(|node| save_node(node.as_ref())).collect(),
    };
    let text = serde_json::to_string_pretty(&layout).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::count_nodes;

    #[test]
    fn layout_roundtrip() {
        let nodes = crate::mock::default_layout();
        let before = count_nodes(&nodes);
        let dir = std::env::temp_dir().join(format!("ui-tree-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layout.json");

        save_to(&path, &nodes).unwrap();
        let specs = load_from(&path).unwrap();
        let rebuilt: Vec<Box<dyn Node>> = specs.iter().filter_map(crate::nodes::build).collect();

        assert_eq!(count_nodes(&rebuilt), before);
        assert_eq!(rebuilt.len(), nodes.len());
        assert_eq!(rebuilt[0].kind(), "window");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
