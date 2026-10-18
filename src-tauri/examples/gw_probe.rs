// Пробник gateway: проверяем, какие события шлёт Discord при разных Identify.
// Запуск: cargo run --example gw_probe -- plain|caps
// Подключается ВТОРОЙ сессией (для user-аккаунта это допустимо), приложение не трогает.
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

fn read_token() -> String {
    let home = std::env::var("HOME").unwrap();
    let text =
        std::fs::read_to_string(format!("{home}/.config/discord-client/session.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["token"].as_str().unwrap().to_string()
}

#[tokio::main]
async fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "plain".into());
    let token = read_token();
    let (ws, _) = tokio_tungstenite::connect_async("wss://gateway.discord.gg/?v=10&encoding=json")
        .await
        .expect("connect");
    let (mut tx, mut rx) = ws.split();

    let mut last_seq: Option<u64> = None;
    let mut hb_interval = Duration::from_secs(60);
    let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
    let start = Instant::now();
    let mut sent_identify = false;
    let mut guild_create_seen = 0u32;
    let mut guild_create_sample: Option<serde_json::Value> = None;
    let mut next_hb = Instant::now() + hb_interval;

    while start.elapsed() < Duration::from_secs(30) {
        let msg = tokio::time::timeout(Duration::from_millis(500), rx.next()).await;
        match msg {
            Ok(Some(Ok(Message::Text(text)))) => {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                let op = v["op"].as_u64().unwrap_or(0);
                let t = v["t"].as_str().unwrap_or("");
                if let Some(s) = v["s"].as_u64() {
                    last_seq = Some(s);
                }
                if op == 10 {
                    hb_interval =
                        Duration::from_millis(v["d"]["heartbeat_interval"].as_u64().unwrap_or(41250));
                    next_hb = Instant::now() + hb_interval;
                    if !sent_identify {
                        sent_identify = true;
                        let d = match mode.as_str() {
                            "caps" => json!({
                                "token": token,
                                "capabilities": 22541,
                                "properties": {
                                    "os": "Linux",
                                    "browser": "Discord Client",
                                    "device": "",
                                    "system_locale": "en-US",
                                    "client_version": "0.0.104",
                                    "has_client_mods": false
                                },
                                "presence": {"status": "online", "activities": [], "afk": false, "since": 0},
                                "compress": false,
                                "client_state": {"guild_versions": {}}
                            }),
                            _ => json!({
                                "token": token,
                                "properties": {"os": "linux", "browser": "discord-client", "device": "discord-client"},
                                "compress": false
                            }),
                        };
                        tx.send(Message::Text(json!({"op": 2, "d": d}).to_string()))
                            .await
                            .unwrap();
                        println!("[{mode}] Identify отправлен");
                    }
                } else {
                    println!("[{mode}] op={op} t={t}");
                    if !t.is_empty() {
                        *counts.entry(t.to_string()).or_default() += 1;
                    }
                    if t == "READY" {
                        let d = &v["d"];
                        let g = &d["guilds"];
                        let n = g.as_array().map(|a| a.len()).unwrap_or(0);
                        let keys: Vec<String> = d
                            .as_object()
                            .map(|o| o.keys().cloned().collect())
                            .unwrap_or_default();
                        println!("[{mode}] READY: гильдий {n}, keys: {keys:?}");
                        let first = g.get(0).cloned().unwrap_or(json!(null));
                        let s = first.to_string();
                        let cut: String = s.chars().take(900).collect();
                        println!("[{mode}] READY guilds[0] (обрезка 900): {cut}");
                        if let Some(arr) = g.as_array() {
                            for guild in arr {
                                let gkeys: Vec<String> = guild
                                    .as_object()
                                    .map(|o| o.keys().cloned().collect())
                                    .unwrap_or_default();
                                println!(
                                    "[{mode}] guild {} keys: {gkeys:?}",
                                    guild["id"]
                                );
                                for f in ["channels", "members", "presences", "voice_states"] {
                                    let len = guild[f].as_array().map(|a| a.len()).unwrap_or(0);
                                    println!("[{mode}]   {f}: {len}");
                                }
                            }
                        }
                        let tp = d["presences"].as_array().map(|a| a.len()).unwrap_or(0);
                        println!("[{mode}] top-level presences: {tp}");
                        if let Some(p0) = d["presences"].get(0) {
                            let pkeys: Vec<String> = p0
                                .as_object()
                                .map(|o| o.keys().cloned().collect())
                                .unwrap_or_default();
                            println!("[{mode}] presences[0] keys: {pkeys:?}");
                            let s = p0.to_string();
                            let cut: String = s.chars().take(400).collect();
                            println!("[{mode}] presences[0]: {cut}");
                        }
                    }
                    if t == "GUILD_CREATE" {
                        guild_create_seen += 1;
                        let d = &v["d"];
                        let pres = d["presences"].as_array().map(|a| a.len()).unwrap_or(0);
                        let mem = d["members"].as_array().map(|a| a.len()).unwrap_or(0);
                        let vs = d["voice_states"].as_array().map(|a| a.len()).unwrap_or(0);
                        println!(
                            "[{mode}] GUILD_CREATE {}: presences={pres} members={mem} voice_states={vs}",
                            d["id"]
                        );
                        if guild_create_sample.is_none() {
                            guild_create_sample = Some(d.clone());
                        }
                    }
                }
            }
            Ok(Some(Ok(Message::Close(c)))) => {
                println!("[{mode}] CLOSE {c:?}");
                break;
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => {
                println!("[{mode}] err {e}");
                break;
            }
            Ok(None) => break,
            Err(_) => {}
        }
        if Instant::now() >= next_hb {
            let _ = tx
                .send(Message::Text(json!({"op": 1, "d": last_seq}).to_string()))
                .await;
            next_hb = Instant::now() + hb_interval;
        }
    }

    println!("[{mode}] === ИТОГИ за 30с ===");
    for (k, c) in &counts {
        println!("[{mode}] {k}: {c}");
    }
    println!("[{mode}] GUILD_CREATE всего: {guild_create_seen}");
    if let Some(s) = guild_create_sample {
        let keys: Vec<String> = s
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        println!("[{mode}] sample GUILD_CREATE keys: {keys:?}");
    }
}
