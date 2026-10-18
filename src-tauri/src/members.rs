// Список участников сервера через gateway.
//
// REST /guilds/{id}/members для user-аккаунтов запрещён (403 Missing Access),
// op 8 (GUILD_MEMBERS_CHUNK) работает только с правами модератора — поэтому
// используем механизм официального клиента: подписка op 37 на диапазоны
// списка участников → ответ GUILD_MEMBER_LIST_UPDATE с SYNC-диапазонами
// (в элементах есть user, roles и presence).

use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{command, AppHandle, Manager};

use discord_client::{DiscordState, FetchSlot, FetchState, GatewaySender, MemberGroup, MemberInfo};

/// Сколько максимум участников тянем (10 диапазонов по 100).
const MAX_MEMBERS_FETCH: u64 = 1000;
/// Сколько ждём ответ на подписку.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// Кэш списка участников, чтобы не переподписываться на каждый клик.
const CACHE_TTL: Duration = Duration::from_secs(60);

fn to_member(m: &serde_json::Value) -> MemberInfo {
    let u = &m["user"];
    let username = m["nick"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| u["display_name"].as_str())
        .or_else(|| u["global_name"].as_str())
        .or_else(|| u["username"].as_str())
        .unwrap_or("")
        .to_string();
    MemberInfo {
        user_id: u["id"].as_str().unwrap_or("").to_string(),
        username,
        avatar: u["avatar"].as_str().map(String::from),
        bot: u["bot"].as_bool().unwrap_or(false),
        roles: m["roles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

#[command]
pub async fn get_guild_members(
    app: AppHandle,
    guild_id: String,
    channel_id: String,
) -> Result<Vec<MemberGroup>, String> {
    let state = app.state::<DiscordState>();

    // 1) Кэш (не дёргаем gateway на каждый клик по серверу).
    {
        let cache = state.member_fetch.cache.lock().unwrap();
        if let Some((when, members)) = cache.get(&guild_id) {
            if when.elapsed() < CACHE_TTL {
                return Ok(members.clone());
            }
        }
    }

    if channel_id.is_empty() {
        return Err("Нет канала для запроса списка участников".into());
    }

    // 2) Диапазоны по числу участников (из READY); неизвестно — первый.
    let ranges: Vec<(i64, i64)> = {
        let counts = state.guild_member_counts.lock().unwrap();
        match counts.get(&guild_id).copied() {
            Some(count) => {
                let n = count.clamp(1, MAX_MEMBERS_FETCH).div_ceil(100);
                (0..n)
                    .map(|i| (i as i64 * 100, i as i64 * 100 + 99))
                    .collect()
            }
            None => vec![(0, 99)],
        }
    };

    // 3) Регистрируем слот и подписываемся (op 37).
    let slot = Arc::new(FetchSlot {
        wanted: ranges.iter().map(|(s, _)| *s).collect(),
        state: std::sync::Mutex::new(FetchState::default()),
        notify: tokio::sync::Notify::new(),
    });
    {
        let mut map = state.member_fetch.fetches.lock().unwrap();
        map.insert(guild_id.clone(), slot.clone());
    }

    let sender = app.state::<GatewaySender>();
    let ranges_json: Vec<serde_json::Value> =
        ranges.iter().map(|(s, e)| json!([s, e])).collect();
    let subscribe = json!({
        "op": 37,
        "d": {"subscriptions": {
            guild_id.clone(): {
                "typing": true,
                "channels": { channel_id.clone(): ranges_json }
            }
        }}
    });
    if let Err(e) = sender.send(subscribe).await {
        state.member_fetch.fetches.lock().unwrap().remove(&guild_id);
        return Err(format!("Не удалось отправить подписку на список: {e}"));
    }

    // 4) Ждём SYNC по всем запрошенным диапазонам.
    let deadline = Instant::now() + FETCH_TIMEOUT;
    let mut complete = false;
    loop {
        {
            let st = slot.state.lock().unwrap();
            if st.done {
                complete = true;
                break;
            }
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let wait = std::cmp::min(deadline - now, Duration::from_millis(500));
        let _ = tokio::time::timeout(wait, slot.notify.notified()).await;
    }

    // 5) Забираем результат и отписываемся от диапазонов.
    //    remove только «нашего» слота: параллельный вызов для того же guild'а
    //    мог уже заменить его в map — чужой слот стирать нельзя (иначе его
    //    ожидание никогда не завершится).
    let ranges = {
        let mut map = state.member_fetch.fetches.lock().unwrap();
        if map.get(&guild_id).is_some_and(|cur| Arc::ptr_eq(cur, &slot)) {
            map.remove(&guild_id);
        }
        let st = slot.state.lock().unwrap();
        st.ranges.clone()
    };
    let unsubscribe = json!({
        "op": 37,
        "d": {"subscriptions": {guild_id.clone(): {"channels": {}}}}
    });
    let _ = sender.send(unsubscribe).await;

    // 6) Сборка групп в порядке отображения Discord:
    //    идём по диапазонам и элементам подряд; `group`-маркер открывает
    //    новую группу, `member`-элемент попадает в текущую.
    let mut seen_ids = HashSet::new();
    let mut groups: Vec<MemberGroup> = Vec::new();
    for items in ranges.values() {
        for it in items {
            if let Some(g) = it.get("group") {
                let id = g["id"].as_str().unwrap_or("").to_string();
                if groups.last().map(|p| p.id == id).unwrap_or(false) {
                    continue; // повторный маркер той же группы (граница диапазона)
                }
                groups.push(MemberGroup { id, members: Vec::new() });
            } else if let Some(m) = it.get("member") {
                let info = to_member(m);
                if info.user_id.is_empty() || !seen_ids.insert(info.user_id.clone()) {
                    continue;
                }
                if groups.is_empty() {
                    // Маркеров групп не встретилось — не теряем участника:
                    // кладём в общую группу (фронт подпишет её «Участники»).
                    groups.push(MemberGroup { id: String::new(), members: Vec::new() });
                }
                groups.last_mut().unwrap().members.push(info);
            }
        }
    }
    groups.retain(|g| !g.members.is_empty());
    if groups.is_empty() {
        return Err("Discord не прислал список участников".into());
    }

    // Проверка по логам: состав групп vs d.groups из GUILD_MEMBER_LIST_UPDATE,
    // покрытие активностями (Rich Presence).
    log::debug!(
        "группы участников gid={guild_id}: {}",
        groups
            .iter()
            .map(|g| format!(
                "{}:{}",
                if g.id.is_empty() { "-" } else { &g.id },
                g.members.len()
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    {
        let pres = state.presences.lock().unwrap();
        if let Some(map) = pres.get(&guild_id) {
            let with_acts = map.values().filter(|p| !p.activities.is_empty()).count();
            log::debug!(
                "presence gid={guild_id}: записей {}, с активностями {}",
                map.len(),
                with_acts
            );
        }
    }

    // Кэшируем только полный ответ: таймаутный (частичный) список протух бы
    // 60с с недостающими участниками.
    if complete {
        let mut cache = state.member_fetch.cache.lock().unwrap();
        cache.insert(guild_id.clone(), (Instant::now(), groups.clone()));
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn to_member_prefers_nick_then_display_then_username() {
        let m = json!({
            "nick": "Мойник",
            "user": {"id": "1", "username": "user", "display_name": "Display", "avatar": "av", "bot": true},
            "roles": ["10", "20"]
        });
        let mi = to_member(&m);
        assert_eq!(mi.user_id, "1");
        assert_eq!(mi.username, "Мойник");
        assert_eq!(mi.avatar.as_deref(), Some("av"));
        assert!(mi.bot);
        assert_eq!(mi.roles, vec!["10".to_string(), "20".to_string()]);
    }

    #[test]
    fn to_member_empty_nick_falls_back() {
        let m = json!({
            "nick": "",
            "user": {"id": "2", "username": "user2", "display_name": "Display2"}
        });
        assert_eq!(to_member(&m).username, "Display2");
    }

    #[test]
    fn to_member_missing_user_is_empty() {
        let mi = to_member(&json!({}));
        assert_eq!(mi.user_id, "");
        assert_eq!(mi.username, "");
        assert!(!mi.bot);
        assert!(mi.roles.is_empty());
    }
}
