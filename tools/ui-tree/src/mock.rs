//! Мок-данные и стартовая раскладка. Никакого бэкенда — только заглушки.

use egui::{pos2, vec2, Color32};

use crate::node::Node;
use crate::nodes::window::WindowNode;
use crate::nodes::{button, debug, label, list, members, servers, voice};
use crate::theme;

pub const SERVERS: &[&str] = &["S1", "S2", "S3", "S4", "S5"];

pub struct VoiceChannel {
    pub name: &'static str,
    pub users: &'static [&'static str],
}

pub const VOICE_CHANNELS: &[VoiceChannel] = &[
    VoiceChannel {
        name: "Голосовой 1",
        users: &["User 2", "User 5"],
    },
    VoiceChannel {
        name: "Голосовой 2",
        users: &["User 3"],
    },
    VoiceChannel {
        name: "AFK",
        users: &[],
    },
];

pub struct Member {
    pub name: &'static str,
    pub status: Color32,
}

pub const MEMBERS_ONLINE: &[Member] = &[
    Member {
        name: "User 1",
        status: theme::ONLINE,
    },
    Member {
        name: "User 2",
        status: theme::ONLINE,
    },
    Member {
        name: "User 3",
        status: theme::IDLE,
    },
    Member {
        name: "User 4",
        status: theme::DND,
    },
    Member {
        name: "User 5",
        status: theme::ONLINE,
    },
];

pub const MEMBERS_OFFLINE: &[Member] = &[
    Member {
        name: "User 6",
        status: theme::MUTED,
    },
    Member {
        name: "User 7",
        status: theme::MUTED,
    },
];

/// Раскладка по эскизу: серверы и ГС слева, участники и активность справа,
/// отладка внизу. Всё — обычные ноды на канвасе.
pub fn default_layout() -> Vec<Box<dyn Node>> {
    let mut nodes: Vec<Box<dyn Node>> = Vec::new();

    let mut win = WindowNode::new(
        "win-servers",
        "Сервера",
        pos2(20.0, 20.0),
        vec2(190.0, 350.0),
    );
    win.base.children.push(Box::new(servers::ServersNode::new(
        "servers",
        pos2(10.0, 44.0),
        vec2(170.0, 290.0),
    )));
    nodes.push(Box::new(win));

    let mut win = WindowNode::new(
        "win-voice",
        "Голосовые каналы",
        pos2(220.0, 20.0),
        vec2(240.0, 350.0),
    );
    win.base.children.push(Box::new(voice::VoiceNode::new(
        "voice",
        pos2(10.0, 44.0),
        vec2(220.0, 290.0),
    )));
    nodes.push(Box::new(win));

    let mut win = WindowNode::new(
        "win-notes",
        "Заметки",
        pos2(470.0, 20.0),
        vec2(220.0, 190.0),
    );
    win.base.children.push(Box::new(label::LabelNode::new(
        "notes-label",
        "Нода-заглушка: текст, кнопка\nи любое дерево внутри окна.",
        pos2(12.0, 44.0),
        13.0,
    )));
    win.base.children.push(Box::new(button::ButtonNode::new(
        "notes-button",
        "Нажми меня",
        "demo.button",
        pos2(12.0, 96.0),
        vec2(140.0, 30.0),
    )));
    nodes.push(Box::new(win));

    let mut win = WindowNode::new(
        "win-members",
        "Участники",
        pos2(700.0, 20.0),
        vec2(250.0, 300.0),
    );
    win.base.children.push(Box::new(members::MembersNode::new(
        "members",
        pos2(10.0, 44.0),
        vec2(230.0, 240.0),
    )));
    nodes.push(Box::new(win));

    let mut win = WindowNode::new(
        "win-activity",
        "Активность",
        pos2(700.0, 330.0),
        vec2(250.0, 230.0),
    );
    win.base.children.push(Box::new(list::ListNode::new(
        "activity",
        vec![
            "User 3 начал стрим".to_string(),
            "User 1 зашёл в ГС".to_string(),
            "User 4: привет".to_string(),
            "User 2 вышел".to_string(),
        ],
        pos2(10.0, 44.0),
        vec2(230.0, 180.0),
    )));
    nodes.push(Box::new(win));

    let mut win = WindowNode::new("win-debug", "Debug", pos2(220.0, 390.0), vec2(320.0, 240.0));
    win.base.children.push(Box::new(debug::DebugNode::new(
        "debug",
        pos2(10.0, 44.0),
        vec2(300.0, 180.0),
    )));
    nodes.push(Box::new(win));

    nodes
}
