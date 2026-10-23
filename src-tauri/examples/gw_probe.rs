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
    let guild_create_seen = 0u32;
    let guild_create_sample: Option<serde_json::Value> = None;
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
                    hb_interval = Duration::from_millis(
                        v["d"]["heartbeat_interval"].as_u64().unwrap_or(41250),
                    );
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
                        tx.send(Message::text(json!({"op": 2, "d": d}).to_string()))
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
                                println!("[{mode}] guild {} keys: {gkeys:?}", guild["id"]);
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
                    if t == "READY" && mode == "chunks" {
                        // шлём op 8 на каждую гильдию: query="", limit=0 (все), presences=true
                        if let Some(arr) = v["d"]["guilds"].as_array() {
                            for g in arr {
                                let gid = g["id"].clone();
                                let payload = json!({
                                    "op": 8,
                                    "d": {
                                        "guild_id": [gid],
                                        "query": "",
                                        "limit": 0,
                                        "presences": true
                                    }
                                });
                                println!("[{mode}] op8 → {payload}");
                                tx.send(Message::text(payload.to_string())).await.unwrap();
                            }
                        }
                    }
                    if t == "GUILD_MEMBERS_CHUNK" {
                        let d = &v["d"];
                        let mem = d["members"].as_array().map(|a| a.len()).unwrap_or(0);
                        let pres = d["presences"].as_array().map(|a| a.len()).unwrap_or(0);
                        println!(
                            "[{mode}] CHUNK guild={} members={mem} presences={pres} index={} count={}",
                            d["guild_id"],
                            d["chunk_index"],
                            d["chunk_count"]
                        );
                        if let Some(m0) = d["members"].get(0) {
                            let s = m0.to_string();
                            let cut: String = s.chars().take(350).collect();
                            println!("[{mode}]   members[0]: {cut}");
                        }
                    }
                    if t == "READY" && mode == "mlist" {
                        // подписка на список участников (op 37): typing + channels ranges
                        if let Some(arr) = v["d"]["guilds"].as_array() {
                            for g in arr {
                                let gid = g["id"].as_str().unwrap_or("").to_string();
                                let mc = g["member_count"].as_u64().unwrap_or(0);
                                println!("[{mode}] guild {gid} member_count={mc}");
                                if mc == 0 || mc > 400 {
                                    continue;
                                }
                                let ch = g["channels"]
                                    .as_array()
                                    .and_then(|a| a.iter().find(|c| c["type"].as_u64() == Some(0)));
                                let Some(ch) = ch else { continue };
                                let chid = ch["id"].as_str().unwrap_or("").to_string();
                                let mut ranges = vec![];
                                let n = mc.div_ceil(100).max(1);
                                for i in 0..n {
                                    ranges.push(json!([i * 100, i * 100 + 99]));
                                }
                                let payload = json!({
                                    "op": 37,
                                    "d": {"subscriptions": {
                                        gid.clone(): {
                                            "typing": true,
                                            "channels": { chid.clone(): ranges }
                                        }
                                    }}
                                });
                                println!("[{mode}] op37 → {payload}");
                                tx.send(Message::text(payload.to_string())).await.unwrap();
                            }
                        }
                    }
                    if t == "GUILD_MEMBER_LIST_UPDATE" {
                        let d = &v["d"];
                        let mut items_total = 0usize;
                        let mut ops_desc = vec![];
                        if let Some(ops) = d["ops"].as_array() {
                            for opd in ops {
                                let name = opd["op"].as_str().unwrap_or("?");
                                let items = opd["items"].as_array().map(|a| a.len()).unwrap_or(0);
                                if name == "SYNC" {
                                    items_total += items;
                                }
                                ops_desc.push(format!(
                                    "{name}{} items={items}",
                                    opd["range"]
                                        .as_array()
                                        .map(|r| format!("{r:?}"))
                                        .unwrap_or_default()
                                ));
                            }
                        }
                        println!(
                            "[{mode}] MLIST guild={} member_count={} online_count={} items(SYNC)={items_total} ops: {}",
                            d["guild_id"],
                            d["member_count"],
                            d["online_count"],
                            ops_desc.join(" | ")
                        );
                        if let Some(o0) = d["ops"].as_array().and_then(|a| a.first()) {
                            if let Some(items) = o0["items"].as_array() {
                                // ищем первый настоящий member-элемент (пропускаем group)
                                let m = items.iter().find(|it| it.get("member").is_some());
                                if let Some(item0) = m {
                                    let s = item0.to_string();
                                    let cut: String = s.chars().take(500).collect();
                                    println!("[{mode}]   member item: {cut}");
                                    let has_pres = item0["member"].get("presence").is_some();
                                    let with_pres = items
                                        .iter()
                                        .filter(|it| it["member"].get("presence").is_some())
                                        .count();
                                    println!(
                                        "[{mode}]   presence у member-элементов: {with_pres} (у первого: {has_pres})"
                                    );
                                    if let Some(mi) = items
                                        .iter()
                                        .find(|it| it["member"].get("presence").is_some())
                                    {
                                        let p = &mi["member"]["presence"];
                                        let pk: Vec<String> = p
                                            .as_object()
                                            .map(|o| o.keys().cloned().collect())
                                            .unwrap_or_default();
                                        let acts = p["activities"]
                                            .as_array()
                                            .map(|a| a.len())
                                            .unwrap_or(0);
                                        println!(
                                            "[{mode}]   presence keys: {pk:?}, activities: {acts}"
                                        );
                                        if let Some(a0) = p["activities"].get(0) {
                                            let s = a0.to_string();
                                            let cut: String = s.chars().take(600).collect();
                                            println!("[{mode}]   activity[0]: {cut}");
                                        }
                                    }
                                }
                                // форма первых элементов: G(id,count) / M(user_id)
                                let sample: Vec<String> = items
                                    .iter()
                                    .take(8)
                                    .map(|it| {
                                        if let Some(g) = it.get("group") {
                                            format!("G({},{})", g["id"], g["count"])
                                        } else if let Some(m) = it.get("member") {
                                            format!("M({})", m["user"]["id"])
                                        } else {
                                            "?".into()
                                        }
                                    })
                                    .collect();
                                println!("[{mode}]   items[0..8]: {}", sample.join(" "));
                                // Порядок активностей КАК ПРИСЛАЛ DISCORD: для каждого
                                // участника с активностями (тип:название, timestamps.start)
                                println!(
                                    "[{mode}]   === активности по участникам (порядок Discord) ==="
                                );
                                for it in items {
                                    let Some(m) = it.get("member") else { continue };
                                    let uid = m["user"]["id"].as_str().unwrap_or("");
                                    let uname = m["user"]["username"].as_str().unwrap_or("");
                                    let Some(acts) = m["presence"]["activities"].as_array() else {
                                        continue;
                                    };
                                    if acts.is_empty() {
                                        continue;
                                    }
                                    let desc: Vec<String> = acts
                                        .iter()
                                        .map(|a| {
                                            let kind = a["type"].as_u64().unwrap_or(999);
                                            let name = a["name"].as_str().unwrap_or("");
                                            let details = a["details"].as_str().unwrap_or("");
                                            let state = a["state"].as_str().unwrap_or("");
                                            let start = a["timestamps"]["start"]
                                                .as_i64()
                                                .map(|v| v.to_string())
                                                .unwrap_or_else(|| "-".into());
                                            format!(
                                                "{kind}:{name}[d={details},s={state},start={start}]"
                                            )
                                        })
                                        .collect();
                                    println!(
                                        "[{mode}]   acts {uname} ({uid}): {}",
                                        desc.join(" | ")
                                    );
                                }
                            }
                        }
                        let groups: Vec<String> = d["groups"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .map(|g| format!("{}:{}", g["id"], g["count"]))
                                    .collect()
                            })
                            .unwrap_or_default();
                        println!("[{mode}]   groups: {groups:?}");
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
                .send(Message::text(json!({"op": 1, "d": last_seq}).to_string()))
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
