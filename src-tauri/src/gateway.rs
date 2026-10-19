use serde_json::json;
use futures_util::{SinkExt, StreamExt};
use tauri::{command, AppHandle, Emitter, Manager, State};
use tokio_tungstenite::tungstenite::Message;

use discord_client::{parse_presence, parse_voice_state, DiscordState, GatewaySender, StreamTarget};
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
        stream: state.stream.clone(),
        stream_conn: state.stream_conn.clone(),
        playback_tx: state.playback_tx.clone(),
        stream_audio_at: state.stream_audio_at.clone(),
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
    // Состояние mute/deaf берём из атомиков: жёсткие false/false молча
    // размьючивали/разглушали пользователя после reconnect-а.
    let self_mute = st.self_mute.load(std::sync::atomic::Ordering::SeqCst);
    let self_deaf = st.self_deaf.load(std::sync::atomic::Ordering::SeqCst);
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
                "self_mute": self_mute,
                "self_deaf": self_deaf
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
                                        if let Some(uid) = s["user_id"].as_str() {
                                            if let Some(info) = parse_voice_state(s) {
                                                vs_entry.insert(uid.to_string(), info);
                                            }
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
                    // Стримы (Go Live): отдельный RTC-коннект, свои события.
                    "STREAM_CREATE" => handle_stream_create(app, &v["d"]),
                    "STREAM_SERVER_UPDATE" => handle_stream_server_update(app, &v["d"]),
                    "STREAM_DELETE" => handle_stream_delete(app, &v["d"]),
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
                                    if let Some(uid) = s["user_id"].as_str() {
                                        if let Some(info) = parse_voice_state(s) {
                                            vs_entry.insert(uid.to_string(), info);
                                        }
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
    let mut session_set = false;
    {
        let mut guard = state.voice.lock().unwrap();
        if let Some(voice) = guard.as_mut() {
            if d["guild_id"].as_str() == Some(voice.guild_id.as_str())
                && d["user_id"].as_str() == Some(voice.user_id.as_str())
            {
                let session_id = d["session_id"].as_str().unwrap_or("").to_string();
                if !session_id.is_empty()
                    && voice.session_id.as_deref() != Some(session_id.as_str())
                {
                    voice.session_id = Some(session_id);
                    session_set = true;
                }
            }
        }
    }
    // Тройка (session, token, endpoint) могла собраться именно здесь: Discord
    // вправе прислать VOICE_SERVER_UPDATE раньше VOICE_STATE_UPDATE — раньше
    // подключение стартовало только из VOICE_SERVER_UPDATE и при таком порядке
    // не стартовало вообще.
    if session_set {
        maybe_connect_voice(app, &state);
    }

    // Голосовые состояния участников (для колонки участников на фронте).
    let gid = d["guild_id"].as_str().unwrap_or("");
    let uid = d["user_id"].as_str().unwrap_or("");
    if !gid.is_empty() && !uid.is_empty() {
        let info = parse_voice_state(d);
        let (self_stream, self_video) = match &info {
            Some(i) => (i.self_stream, i.self_video),
            None => (false, false),
        };
        let channel_id = info.as_ref().map(|i| i.channel_id.clone());
        {
            let mut map = state.voice_states.lock().unwrap();
            let entry = map.entry(gid.to_string()).or_default();
            match info {
                Some(info) => {
                    entry.insert(uid.to_string(), info);
                }
                None => {
                    entry.remove(uid);
                }
            }
        }
        let _ = app.emit(
            "voice_member_update",
            json!({
                "guild_id": gid,
                "user_id": uid,
                "channel_id": channel_id,
                "self_stream": self_stream,
                "self_video": self_video,
            }),
        );
    }
}

async fn handle_voice_server(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    {
        let mut guard = state.voice.lock().unwrap();
        if let Some(voice) = guard.as_mut() {
            if d["guild_id"].as_str() == Some(voice.guild_id.as_str()) {
                if let Some(token) = d["token"].as_str() {
                    voice.token = Some(token.to_string());
                }
                if let Some(endpoint) = d["endpoint"].as_str() {
                    voice.endpoint = Some(endpoint.trim_start_matches("wss://").to_string());
                }
            }
        }
    }
    // Тройка могла собраться здесь (токен пришёл последним) — общий триггер.
    maybe_connect_voice(app, &state);
}

/// STREAM_CREATE: пришёл стрим (или мы подключились к чужому). Несёт
/// rtc_server_id/rtc_channel_id — RTC-идентичность стрима (своя, не родительского канала).
/// Токен/endpoint придут позже в STREAM_SERVER_UPDATE.
fn handle_stream_create(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    let stream_key = d["stream_key"].as_str().unwrap_or("").to_string();
    if stream_key.is_empty() {
        return;
    }
    // owner_id — последний сегмент ключа: guild:channel:owner | call:channel:owner.
    let owner_id = stream_key.rsplit(':').next().unwrap_or("").to_string();
    {
        let mut slot = state.stream.lock().unwrap();
        let target = slot.get_or_insert_with(StreamTarget::default);
        target.stream_key = stream_key.clone();
        target.owner_id = owner_id.clone();
        if let Some(v) = d["rtc_server_id"].as_str() {
            target.rtc_server_id = Some(v.to_string());
        }
        if let Some(v) = d["rtc_channel_id"].as_str() {
            target.rtc_channel_id = Some(v.to_string());
        }
        log::info!(
            "Stream Create: key={stream_key} rtc_server={:?} rtc_channel={:?} paused={:?}",
            target.rtc_server_id,
            target.rtc_channel_id,
            d["paused"]
        );
    }
    maybe_emit_stream_ready(app, &state);
}

/// STREAM_SERVER_UPDATE: пришёл токен/endpoint голосового сервера стрима.
/// endpoint=null — сервер ушёл, переподключаться нельзя до новой аллокации.
fn handle_stream_server_update(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    let stream_key = d["stream_key"].as_str().unwrap_or("").to_string();
    if stream_key.is_empty() {
        return;
    }
    let endpoint = d["endpoint"].as_str();
    {
        let mut slot = state.stream.lock().unwrap();
        // Сервер стрима мог смениться: привязываемся к ключу, не к «любому» стриму.
        if slot.as_ref().is_none_or(|t| t.stream_key != stream_key) {
            let target = slot.get_or_insert_with(StreamTarget::default);
            target.stream_key = stream_key.clone();
            target.owner_id = stream_key.rsplit(':').next().unwrap_or("").to_string();
        }
        let target = slot.as_mut().expect("stream target ensured");
        match endpoint {
            Some(ep) => {
                target.endpoint = Some(ep.trim_start_matches("wss://").to_string());
                if let Some(tok) = d["token"].as_str() {
                    target.token = Some(tok.to_string());
                }
                log::info!("Stream Server Update: key={stream_key} endpoint={:?}", target.endpoint);
            }
            None => {
                // Endpoint ушёл: сбрасываем транспортную пару, не трогая RTC-идентичность.
                target.token = None;
                target.endpoint = None;
                log::info!("Stream Server Update: key={stream_key} endpoint=null (сервер ушёл)");
            }
        }
    }
    maybe_emit_stream_ready(app, &state);
}

/// STREAM_DELETE: стрим закончился / недоступен / не удалось подключиться
/// (reason=stream_full). Полностью сбрасываем цель.
fn handle_stream_delete(app: &AppHandle, d: &serde_json::Value) {
    let state: State<'_, DiscordState> = app.state();
    let stream_key = d["stream_key"].as_str().unwrap_or("");
    let reason = d["reason"].as_str().unwrap_or("");
    log::info!("Stream Delete: key={stream_key} reason={reason}");
    *state.stream.lock().unwrap() = None;
    // Если смотрели именно этот стрим — рвём его RTC-коннект (resets RTP/DAVE/транспорт).
    let mine = {
        let slot = state.stream_conn.lock().unwrap();
        slot.session_id.as_deref() == Some(stream_key) && !stream_key.is_empty()
    };
    if mine {
        let app2 = app.clone();
        tokio::spawn(async move {
            crate::voice::stream::disconnect_stream(&app2).await;
        });
    }
    let _ = app.emit(
        "stream_update",
        json!({"watching": false, "stream_key": stream_key, "reason": reason}),
    );
}

/// Когда цели стрима собраны (rtc-идентичность + токен/endpoint) — сообщаем фронту.
/// Само RTC-подключение стрима подключается отдельно (Шаг 1.x), здесь только сигнал.
fn maybe_emit_stream_ready(app: &AppHandle, state: &State<'_, DiscordState>) {
    let snapshot = {
        let slot = state.stream.lock().unwrap();
        slot.clone()
    };
    let Some(t) = snapshot else { return };
    if !t.ready() {
        return;
    }
    log::info!(
        "Stream target готов: key={} rtc_server={} rtc_channel={} endpoint={:?} → поднимаем RTC-коннект",
        t.stream_key,
        t.rtc_server_id.as_deref().unwrap_or("?"),
        t.rtc_channel_id.as_deref().unwrap_or("?"),
        t.endpoint
    );
    // Обе части собраны — поднимаем второй RTC-коннект стрима (если ещё не подключены).
    crate::voice::stream::maybe_connect_stream(app);
    let _ = app.emit(
        "stream_update",
        json!({
            "watching": true,
            "stream_key": t.stream_key,
            "owner_id": t.owner_id,
            "rtc_server_id": t.rtc_server_id,
            "rtc_channel_id": t.rtc_channel_id,
            "endpoint": t.endpoint,
        }),
    );
}

/// Общий триггер автоподключения: запускается, когда тройка
/// (session, token, endpoint) собрана — из VOICE_STATE_UPDATE или
/// VOICE_SERVER_UPDATE (Discord вправе прислать их в любом порядке).
/// Дедуп по паре (session, token) — внутри connect_voice, под локом.
fn maybe_connect_voice(app: &AppHandle, state: &State<'_, DiscordState>) {
    // Токен в webview не отдаём: фронт подключение не выполняет.
    let (guild_ids, channel_ids, session_id, token, endpoint) = {
        let mut guard = state.voice.lock().unwrap();
        let Some(voice) = guard.as_mut() else { return };
        if !(voice.session_id.is_some() && voice.token.is_some() && voice.endpoint.is_some()) {
            return;
        }
        log::info!(
            "Голосовое подключение собрано: guild={} channel={} session={} token=<redacted>",
            voice.guild_id, voice.channel_id, voice.session_id.clone().unwrap_or_default()
        );
        let _ = app.emit(
            "voice_channel_ready",
            json!({
                "guild_id": voice.guild_id,
                "channel_id": voice.channel_id,
                "session_id": voice.session_id,
                "endpoint": voice.endpoint,
            }),
        );
        (
            voice.guild_id.clone(),
            voice.channel_id.clone(),
            voice.session_id.clone().unwrap(),
            voice.token.clone().unwrap(),
            voice.endpoint.clone().unwrap(),
        )
    };

    // Авто-подключение из бэкенда: свежий токен одноразовый, тратим сразу.
    let app2 = app.clone();
    let discord_state = clone_state(state);
    tokio::spawn(async move {
        if let Err(e) = connect_voice(
            app2, discord_state, guild_ids, channel_ids, session_id, token, endpoint,
        ).await {
            log::error!("Voice error: {e}");
        }
    });
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
