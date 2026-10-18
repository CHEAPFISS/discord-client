use serde_json::json;
use futures_util::{SinkExt, StreamExt};
use tauri::{command, AppHandle, Emitter, Manager, State};
use tokio_tungstenite::tungstenite::Message;

use discord_client::{parse_presence, DiscordState, GatewaySender};
use crate::api::parse_message;
use crate::voice::connect::connect_voice;

/// Защита от повторного запуска: только одно соединение за раз.
pub(crate) static GATEWAY_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Служебный payload для rx-канала gateway: «остановись окончательно» (смена аккаунта).
pub(crate) const GATEWAY_RELOG_MARKER: &str = "__relog__";
/// Флаг «релогин запрошен»: маркер в rx не опрашивается во время сонов между
/// реконнектами и внутри connect/HELLO — без этого флага restart зависал бы в
/// ожидании, а новый старт получал бы «already-running» и шлюз молча умирал.
pub(crate) static GATEWAY_RELOG_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Сброс `GATEWAY_RUNNING` при любом выходе задачи gateway — ранний return
/// (нет токена) и паника в loop иначе оставляли флаг true навсегда
/// (паника: drop future внутри задачи выполняется и при unwind).
struct GatewayRunGuard;
impl Drop for GatewayRunGuard {
    fn drop(&mut self) {
        GATEWAY_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
/// Адрес Discord Gateway v10.
pub(crate) const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// Клоны Arc-полей состояния для spawned задач.
pub(crate) fn clone_state(state: &State<'_, DiscordState>) -> DiscordState {
    DiscordState {
        session: state.session.clone(),
        voice: state.voice.clone(),
        voice_conn: state.voice_conn.clone(),
        voice_connect_lock: state.voice_connect_lock.clone(),
        voice_rejoin: state.voice_rejoin.clone(),
        mic_sender: state.mic_sender.clone(),
        self_deaf: state.self_deaf.clone(),
        self_mute: state.self_mute.clone(),
        voice_speaking: state.voice_speaking.clone(),
        presences: state.presences.clone(),
        voice_states: state.voice_states.clone(),
        guild_member_counts: state.guild_member_counts.clone(),
        gateway_status: state.gateway_status.clone(),
        member_fetch: state.member_fetch.clone(),
    }
}

/// Единая точка отправки статуса шлюза: запоминаем последний статус (для
/// get_gateway_status после перезагрузки страницы) и эмитим во фронт.
pub(crate) fn emit_gateway_status(app: &AppHandle, status: &str, error: Option<&str>) {
    {
        let state: State<'_, DiscordState> = app.state();
        let mut guard = state.gateway_status.lock().unwrap();
        guard.0 = status.to_string();
        guard.1 = error.map(str::to_string);
    }
    let mut payload = json!({"status": status});
    if let Some(e) = error {
        payload["error"] = json!(e);
    }
    let _ = app.emit("gateway_status", payload);
}

/// Текущий статус шлюза — для восстановления индикатора после перезагрузки страницы.
#[command]
pub(crate) fn get_gateway_status(state: State<'_, DiscordState>) -> serde_json::Value {
    let guard = state.gateway_status.lock().unwrap();
    json!({"status": guard.0.clone(), "error": guard.1.clone()})
}

/// Смена идентичности: гасим голос (oto) и голосовой стейт под старым аккаунтом.
/// Берёт voice_connect_lock — иначе чистка идёт параллельно с активным
/// connect_voice и старый канал воскресает уже под новым аккаунтом.
pub(crate) async fn reset_voice_on_identity_change(app: &AppHandle, state: &DiscordState) {
    let _connect_guard = state.voice_connect_lock.lock().await;
    let conn = {
        let mut slot = state.voice_conn.lock().unwrap();
        let conn = slot.conn.take();
        slot.session_id = None;
        slot.token = None;
        slot.pump_spawned = false;
        conn
    };
    state
        .voice_rejoin
        .store(0, std::sync::atomic::Ordering::SeqCst);
    *state.voice.lock().unwrap() = None;
    if let Some(conn) = conn {
        let _ = conn.shutdown().await;
    }
    let _ = app.emit("voice_status", json!({"status": "left"}));
}

/// Просит активный gateway-цикл выйти окончательно (релогин/выход):
/// флаг + маркер в rx — флаг ловится даже в паузах и при connect/HELLO,
/// маркер — когда цикл сидит в select. true = цикл остановился (или его не было).
pub(crate) async fn request_gateway_stop(sender: &GatewaySender, wait: std::time::Duration) -> bool {
    GATEWAY_RELOG_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
    let marker = json!({"internal": GATEWAY_RELOG_MARKER});
    let _ = sender.send(marker).await;
    let deadline = std::time::Instant::now() + wait;
    while GATEWAY_RUNNING.load(std::sync::atomic::Ordering::SeqCst)
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let stopped = !GATEWAY_RUNNING.load(std::sync::atomic::Ordering::SeqCst);
    if !stopped {
        log::warn!("Gateway: цикл не остановился за {}мс", wait.as_millis());
    }
    GATEWAY_RELOG_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
    stopped
}

/// Перезапуск gateway-цикла под НОВЫМ токеном: стоп → ожидание → старт.
pub(crate) async fn restart_gateway_for_new_token(
    app: &AppHandle,
    state: &DiscordState,
    sender: &GatewaySender,
) {
    // 1) просим активный loop выйти и ждём ФАКТИЧЕСКОЙ остановки
    //    (раньше — фиксированные 5с ожидания: при паузе/зависании цикл не
    //    успевал выйти, новый старт получал «already-running» и шлюз умирал).
    request_gateway_stop(sender, std::time::Duration::from_secs(5)).await;
    // 2) заводим новый gateway с текущей (новой) сессией
    let app2 = app.clone();
    let state2 = state.clone();
    // sender: GatewaySender не Clone — живём через app.state::<GatewaySender>()
    tokio::spawn(async move {
        let app3 = app2.clone();
        let sender2 = app3.state::<GatewaySender>();
        let _ = start_gateway_inner(app2, state2, sender2.inner()).await;
    });
}

/// После переподключения gateway (это НОВАЯ gateway-сессия) Discord мог молча
/// похоронить голос от старой сессии — переотправляем op 4, чтобы он выдал свежую paire.
pub(crate) fn rekick_voice_if_joined(app: &AppHandle) {
    let st = app.state::<DiscordState>();
    let target = st
        .voice
        .lock()
        .unwrap()
        .as_ref()
        .map(|v| (v.guild_id.clone(), v.channel_id.clone()));
    drop(st);
    let Some((guild_id, channel_id)) = target else { return };
    log::info!("Gateway: новая сессия — переотправка op 4 (канал {channel_id})");
    let app2 = app.clone();
    tokio::spawn(async move {
        let sender = app2.state::<GatewaySender>();
        let op4 = json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": channel_id,
                "self_mute": false,
                "self_deaf": false
            }
        });
        if let Err(e) = sender.send(op4).await {
            log::warn!("Gateway rekick op4: {e}");
        }
    });
}

/// Бесконечный цикл соединения с ретраями.
pub(crate) async fn gateway_loop(
    app: AppHandle,
    token: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Result<(), String> {
    let mut first_pass = true;
    loop {
        // Релогин запрошен — выходим, не переподключаясь (флаг виден даже когда
        // маркер в rx ещё не опрашивается — см. GATEWAY_RELOG_REQUESTED).
        if GATEWAY_RELOG_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
            log::info!("Gateway: запрошен релогин — окончательная остановка");
            return Ok(());
        }
        match gateway_once(&app, &token, &mut rx).await {
            Ok(true) => {
                log::info!("Gateway: окончательная остановка (релогин)");
                return Ok(());
            }
            Ok(_) => {
                log::info!("Gateway: закрыто штатно, переподключение через 3с");
                if !first_pass {
                    rekick_voice_if_joined(&app);
                }
                if sleep_unless_relog(std::time::Duration::from_secs(3)).await {
                    log::info!("Gateway: релогин во время паузы — останавливаюсь");
                    return Ok(());
                }
            }
            Err(e) => {
                log::warn!("Gateway: ошибка {e}, переподключение через 5с");
                emit_gateway_status(&app, "reconnecting", Some(e.as_str()));
                if !first_pass {
                    rekick_voice_if_joined(&app);
                }
                if sleep_unless_relog(std::time::Duration::from_secs(5)).await {
                    log::info!("Gateway: релогин во время паузы — останавливаюсь");
                    return Ok(());
                }
            }
        }
        first_pass = false;
    }
}

/// Пауза между реконнектами с ранним выходом, если запрошен релогин.
/// true = релогин.
async fn sleep_unless_relog(d: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + d;
    while std::time::Instant::now() < deadline {
        if GATEWAY_RELOG_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// true = остановиться окончательно (маркер релогина), false/Err = обычный reconnect-цикл.
async fn gateway_once(
    app: &AppHandle,
    token: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Result<bool, String> {
    
    let (ws, _) = tokio_tungstenite::connect_async(GATEWAY_URL)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let (mut write, mut read) = ws.split();

    // 1) жду HELLO (op 10) с heartbeat_interval
    let mut last_seq: Option<u64> = None;
    let heartbeat_ms = loop {
        let msg = read.next().await.ok_or("поток закрылся")?;
        let ping = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Ping(pay)) => {
                write.send(Message::Pong(pay)).await.ok();
                continue;
            }
            Ok(other) => {
                log::info!("Gateway до HELLO: {other:?}");
                continue;
            }
            Err(e) => return Err(format!("ws при HELLO: {e}")),
        };
        let v: serde_json::Value = serde_json::from_str(&ping).map_err(|e| e.to_string())?;
        if v["op"].as_u64() == Some(10) {
            break v["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
        }
    };

    // 2) Identify (user-токен, без intents)
    let identify = json!({
        "op": 2,
        "d": {
            "token": token,
            "properties": {"os": "linux", "browser": "discord-client", "device": "discord-client"},
            "compress": false
        }
    });
    write
        .send(Message::Text(identify.to_string()))
        .await
        .map_err(|e| e.to_string())?;

    emit_gateway_status(app, "connected", None);
    log::info!("Gateway: подключен, heartbeat {heartbeat_ms}мс");
    log::info!("Identify отправлен");

    // 3) heartbeat + приём событий
    let mut hb = tokio::time::interval(std::time::Duration::from_millis(heartbeat_ms));
    hb.tick().await; // первый tick срабатывает сразу
    // Контроль живости: нет HEARTBEAT ACK к следующему тику — рвём и реконнектим.
    let mut awaiting_ack = false;

    loop {
        tokio::select! {
            _ = hb.tick() => {
                if awaiting_ack {
                    log::warn!("Gateway: HEARTBEAT ACK не получен — переподключаюсь");
                    return Err("heartbeat ack timeout".into());
                }
                awaiting_ack = true;
                let hb = json!({"op": 1, "d": last_seq});
                if write.send(Message::Text(hb.to_string())).await.is_err() {
                    return Err("heartbeat send failed".into());
                }
            }
            out = rx.recv() => {
                match out {
                    Some(payload) => {
                        if payload.get("internal").and_then(|v| v.as_str()) == Some(GATEWAY_RELOG_MARKER) {
                            log::info!("Gateway: маркер релогина — останавливаю цикл");
                            return Ok(true);
                        }
                        if write.send(Message::Text(payload.to_string())).await.is_err() {
                            return Err("outgoing send failed".into());
                        }
                    }
                    None => { return Err("командный канал закрылся".into()); }
                }
            }
            msg = read.next() => {
                let msg = msg.ok_or("поток закрылся")?;
                let text = match msg.map_err(|e| e.to_string())? {
                    Message::Text(t) => t,
                    Message::Ping(pay) => {
                        log::debug!("WebSocket Ping, отвечаю Pong");
                        if write.send(Message::Pong(pay)).await.is_err() {
                            return Err("pong send failed".into());
                        }
                        continue;
                    }
                    Message::Pong(_) => continue,
                    Message::Close(c) => {
                        return Err(format!("gateway закрыл соединение: {c:?}"));
                    }
                    other => {
                        log::debug!("Gateway прочее: {other:?}");
                        continue;
                    }
                };
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let op = v["op"].as_u64().unwrap_or(0);
                let t = v["t"].as_str().unwrap_or("");
                log::info!("Gateway recv: op={op} t={t}");
                match op {
                    11 => { awaiting_ack = false; /* HEARTBEAT ACK */ }
                    7 | 9 => {
                        return Err(format!("gateway требует реконнект (op {op})"));
                    }
                    _ => {}
                }
                if let Some(s) = v["s"].as_u64() {
                    last_seq = Some(s);
                }
                let t = v["t"].as_str().unwrap_or("");
                match t {
                    "READY" => {
                        // Данные гильдий приходят прямо в READY (GUILD_CREATE для
                        // изначальных гильдий Discord больше не шлёт): засеваем
                        // presences и голосовые состояния из guilds[].
                        let state: State<'_, DiscordState> = app.state();
                        let d = &v["d"];
                        if let Some(guilds) = d["guilds"].as_array() {
                            let mut pres_map = state.presences.lock().unwrap();
                            let mut vs_map = state.voice_states.lock().unwrap();
                            let mut counts = state.guild_member_counts.lock().unwrap();
                            for g in guilds {
                                let gid = g["id"].as_str().unwrap_or("");
                                if gid.is_empty() {
                                    continue;
                                }
                                if let Some(mc) = g["member_count"].as_u64() {
                                    counts.insert(gid.to_string(), mc);
                                }
                                let pres_entry = pres_map.entry(gid.to_string()).or_default();
                                if let Some(arr) = g["presences"].as_array() {
                                    for p in arr {
                                        if let Some(uid) = p["user"]["id"].as_str() {
                                            pres_entry
                                                .insert(uid.to_string(), parse_presence(p));
                                        }
                                    }
                                }
                                let vs_entry = vs_map.entry(gid.to_string()).or_default();
                                if let Some(arr) = g["voice_states"].as_array() {
                                    for s in arr {
                                        if let (Some(uid), Some(cid)) =
                                            (s["user_id"].as_str(), s["channel_id"].as_str())
                                        {
                                            vs_entry.insert(uid.to_string(), cid.to_string());
                                        }
                                    }
                                }
                            }
                            log::info!(
                                "READY: засеяно presences/voice_states для {} гильдий",
                                guilds.len()
                            );
                        }
                        emit_gateway_status(app, "ready", None);
                    }
                    "MESSAGE_CREATE" => {
                        let d = &v["d"];
                        if d["type"].as_u64().unwrap_or(0) == 0 {
                            let _ = app.emit("message_create", parse_message(d));
                        }
                    }
                    "VOICE_STATE_UPDATE" => {
                        handle_voice_state(app, &v["d"]).await;
                    }
                    "VOICE_SERVER_UPDATE" => {
                        handle_voice_server(app, &v["d"]).await;
                    }
                    // GUILD_CREATE приходит на каждый сервер после READY: засеваем
                    // presences и голосовые состояния участников.
                    "GUILD_CREATE" => {
                        let state: State<'_, DiscordState> = app.state();
                        let d = &v["d"];
                        let gid = d["id"].as_str().unwrap_or("");
                        if !gid.is_empty() {
                            let mut pres = state.presences.lock().unwrap();
                            let pres_entry = pres.entry(gid.to_string()).or_default();
                            if let Some(arr) = d["presences"].as_array() {
                                for p in arr {
                                    if let Some(uid) = p["user"]["id"].as_str() {
                                        pres_entry.insert(uid.to_string(), parse_presence(p));
                                    }
                                }
                            }
                            drop(pres);
                            let mut vs = state.voice_states.lock().unwrap();
                            let vs_entry = vs.entry(gid.to_string()).or_default();
                            if let Some(arr) = d["voice_states"].as_array() {
                                for s in arr {
                                    if let (Some(uid), Some(cid)) =
                                        (s["user_id"].as_str(), s["channel_id"].as_str())
                                    {
                                        vs_entry.insert(uid.to_string(), cid.to_string());
                                    }
                                }
                            }
                        }
                    }
                    "PRESENCE_UPDATE" => {
                        let state: State<'_, DiscordState> = app.state();
                        let d = &v["d"];
                        let gid = d["guild_id"].as_str().unwrap_or("");
                        let uid = d["user"]["id"].as_str().unwrap_or("");
                        if !gid.is_empty() && !uid.is_empty() {
                            let info = parse_presence(d);
                            state
                                .presences
                                .lock()
                                .unwrap()
                                .entry(gid.to_string())
                                .or_default()
                                .insert(uid.to_string(), info.clone());
                            let _ = app.emit(
                                "presence_update",
                                json!({
                                    "guild_id": gid,
                                    "user_id": uid,
                                    "status": info.status,
                                    "activities": info.activities,
                                }),
                            );
                        }
                    }
                    // Ответ на подписку op 37: SYNC-диапазоны списка участников.
                    "GUILD_MEMBER_LIST_UPDATE" => {
                        let state: State<'_, DiscordState> = app.state();
                        let d = &v["d"];
                        let gid = d["guild_id"].as_str().unwrap_or("");
                        if !gid.is_empty() {
                            let slot = {
                                let map = state.member_fetch.fetches.lock().unwrap();
                                map.get(gid).cloned()
                            };
                            if let Some(slot) = slot {
                                let mut st = slot.state.lock().unwrap();
                                if let Some(ops) = d["ops"].as_array() {
                                    for op in ops {
                                        if op["op"].as_str() != Some("SYNC") {
                                            continue;
                                        }
                                        let Some(start) =
                                            op["range"].get(0).and_then(|x| x.as_i64())
                                        else {
                                            continue;
                                        };
                                        st.seen.insert(start);
                                        if let Some(items) = op["items"].as_array() {
                                            // элементы как есть (group-маркеры + members):
                                            // порядок items = порядок отображения Discord.
                                            st.ranges.insert(start, items.clone());
                                        }
                                    }
                                }
                                // presence из элементов — в общую карту статусов
                                {
                                    let mut pres = state.presences.lock().unwrap();
                                    let entry = pres.entry(gid.to_string()).or_default();
                                    for items in st.ranges.values() {
                                        for it in items {
                                            if let Some(m) = it.get("member") {
                                                if let Some(uid) = m["user"]["id"].as_str() {
                                                    if m.get("presence").is_some() {
                                                        entry.insert(
                                                            uid.to_string(),
                                                            parse_presence(&m["presence"]),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if !st.done
                                    && !st.seen.is_empty()
                                    && slot.wanted.iter().all(|s| st.seen.contains(s))
                                {
                                    st.done = true;
                                    slot.notify.notify_one();
                                }
                                let members_count: usize = st
                                    .ranges
                                    .values()
                                    .map(|items| {
                                        items
                                            .iter()
                                            .filter(|it| it.get("member").is_some())
                                            .count()
                                    })
                                    .sum();
                                log::debug!(
                                    "member list sync: gid={gid} членов={} seen={:?}",
                                    members_count,
                                    st.seen
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Накапливает данные голосового подключения; когда все три есть — готово.
/// Плюс: ведёт учёт, кто из участников в каком голосовом канале (для списка участников).
async fn handle_voice_state(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    {
        let mut guard = state.voice.lock().unwrap();
        if let Some(voice) = guard.as_mut() {
            if d["guild_id"].as_str() == Some(voice.guild_id.as_str())
                && d["user_id"].as_str() == Some(voice.user_id.as_str())
            {
                let session_id = d["session_id"].as_str().unwrap_or("").to_string();
                if !session_id.is_empty() {
                    voice.session_id = Some(session_id);
                }
                // Подключаемся только когда придёт свежий token (VOICE_SERVER_UPDATE) —
                // иначе используем протухший и получим 4014/4022.
            }
        }
    }

    // Голосовые состояния участников (для колонки участников на фронте).
    let gid = d["guild_id"].as_str().unwrap_or("");
    let uid = d["user_id"].as_str().unwrap_or("");
    if !gid.is_empty() && !uid.is_empty() {
        let channel_id = d["channel_id"].as_str().map(String::from);
        {
            let mut map = state.voice_states.lock().unwrap();
            let entry = map.entry(gid.to_string()).or_default();
            match &channel_id {
                Some(cid) => {
                    entry.insert(uid.to_string(), cid.clone());
                }
                None => {
                    entry.remove(uid);
                }
            }
        }
        let _ = app.emit(
            "voice_member_update",
            json!({"guild_id": gid, "user_id": uid, "channel_id": channel_id}),
        );
    }
}

async fn handle_voice_server(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    let mut guard = state.voice.lock().unwrap();
    if let Some(voice) = guard.as_mut() {
        if d["guild_id"].as_str() == Some(voice.guild_id.as_str()) {
            if let Some(token) = d["token"].as_str() {
                voice.token = Some(token.to_string());
            }
            if let Some(endpoint) = d["endpoint"].as_str() {
                voice.endpoint = Some(endpoint.trim_start_matches("wss://").to_string());
            }
            if voice.session_id.is_some() && voice.token.is_some() && voice.endpoint.is_some() {
                // Токен в webview не отдаём: фронт подключение не выполняет.
                let payload = json!({
                    "guild_id": voice.guild_id,
                    "channel_id": voice.channel_id,
                    "session_id": voice.session_id,
                    "endpoint": voice.endpoint,
                });
                log::info!(
                    "Голосовое подключение собрано: guild={} channel={} session={} token=<redacted>",
                    voice.guild_id, voice.channel_id, voice.session_id.clone().unwrap_or_default()
                );
                let _ = app.emit("voice_channel_ready", payload);

                // Авто-подключение из бэкенда: свежий токен одноразовый, тратим сразу.
                let app2 = app.clone();
                let discord_state = clone_state(&state);
                let guild_ids = voice.guild_id.clone();
                let channel_ids = voice.channel_id.clone();
                let session_id = voice.session_id.clone().unwrap();
                let token = voice.token.clone().unwrap();
                let endpoint = voice.endpoint.clone().unwrap();
                tokio::spawn(async move {
                    if let Err(e) = connect_voice(
                        app2, discord_state, guild_ids, channel_ids, session_id, token, endpoint,
                    ).await {
                        log::error!("Voice error: {e}");
                    }
                });
            }
        }
    }
}

#[command]
pub(crate) async fn connect_gateway(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<String, String> {
    start_gateway_inner(app, clone_state(&state), &sender).await
}

async fn start_gateway_inner(
    app: AppHandle,
    state: DiscordState,
    sender: &GatewaySender,
) -> Result<String, String> {
    use std::sync::atomic::Ordering;

    if GATEWAY_RUNNING.swap(true, Ordering::SeqCst) {
        // уже запущен — считаем это успехом
        return Ok("already-running".into());
    }
    // Любой выход ниже (нет токена — ранний return; конец/паника loop) снимает флаг.
    let run_guard = GatewayRunGuard;

    let token = state.token().ok_or("Не залогинен")?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    *sender.tx.lock().await = Some(tx);

    tokio::spawn(async move {
        let _run_guard = run_guard;
        let _ = gateway_loop(app.clone(), token, rx).await;
    });

    Ok("started".into())
}
