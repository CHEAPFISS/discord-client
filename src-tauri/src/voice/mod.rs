use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{command, AppHandle, Emitter, Manager, State};

use client_media::audio::mic::MicSource;
use discord_client::{DiscordState, GatewaySender};

pub mod connect;
pub mod stream;

/// Лимит авто-переподключений (re-join) после 4014/4022 NeedsFreshVoiceInfo.
pub(crate) const VOICE_REJOIN_LIMIT: u64 = 3;

/// Голосовое подключение к каналу: выставляет op 4 (Discord пришлёт VOICE_SERVER_UPDATE/STATE).
/// Повторный клик по активному каналу — no-op, без разрыва аудио.
#[command]
pub(crate) async fn join_voice_channel(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
    channel_id: String,
    guild_id: String,
) -> Result<(), String> {
    // user_id берём из сессии
    let user_id = {
        let guard = state.session.lock().unwrap();
        guard
            .as_ref()
            .ok_or(crate::i18n::t(crate::i18n::Key::NotLoggedIn))?
            .user_id
            .clone()
    };

    // Повторный клик по каналу, где мы уже в голосе: живое соединение не рвём
    // (раньше op 4 выдавал свежую пару → connect_voice пересоединялся и рвал
    // аудио) и не сбрасываем mute/deaf. Подтверждаем UI.
    {
        let slot = state.voice_conn.lock().unwrap();
        let alive = slot.conn.as_ref().is_some_and(|c| {
            !matches!(
                c.state().phase(),
                oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed
            )
        });
        let in_target = state
            .voice
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|v| v.channel_id == channel_id);
        if alive && in_target {
            log::debug!("Voice: повторный вход в активный канал — no-op");
            let _ = app.emit("voice_status", json!({"status": "connected"}));
            return Ok(());
        }
    }

    // фиксируем цель голосового подключения
    state.voice_rejoin.store(0, Ordering::SeqCst);
    // На входе — чистые флаги mute/deaf (иначе протухшее «глухой» из прошлого
    // подключения будет гасить приём звука и обводку).
    state.self_mute.store(false, Ordering::SeqCst);
    state.self_deaf.store(false, Ordering::SeqCst);
    // Флаги сброшены — локальную паузу захвата снимаем тоже (мьют от прошлого
    // канала иначе переживёт смену: UI unmuted, а микрофон молчит).
    let mic = state.mic_sender.lock().unwrap().clone();
    if let Some(mic) = mic {
        if let Err(e) = mic.set_paused(false).await {
            log::warn!("Join (снятие паузы микрофона): {e}");
        }
    }
    {
        let mut voice = state.voice.lock().unwrap();
        *voice = Some(discord_client::VoiceData {
            guild_id: guild_id.clone(),
            channel_id: channel_id.clone(),
            user_id: user_id.clone(),
            session_id: None,
            token: None,
            endpoint: None,
        });
    }

    // op 4 — обновить голосовое состояние (Discord пришлёт VOICE_SERVER_UPDATE/STATE)
    let op4 = json!({
        "op": 4,
        "d": {
            "guild_id": guild_id,
            "channel_id": channel_id,
            "self_mute": false,
            "self_deaf": false
        }
    });
    sender.send(op4).await?;

    // уведомление UI
    let _ = app.emit("voice_status", json!({"status": "joining"}));
    Ok(())
}

#[command]
pub(crate) async fn leave_voice_channel(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<(), String> {
    // Сериализация с in-flight connect: иначе коннект успеет "воскресить" голос после выхода.
    let _guard = state.voice_connect_lock.lock().await;
    let _guild_id = {
        let mut voice = state.voice.lock().unwrap();
        let guild = voice.as_ref().map(|v| v.guild_id.clone());
        *voice = None;
        guild
    };
    // Единая секция: conn + пара + флаг насоса согласованно, без окна между локами.
    let active_conn = {
        let mut slot = state.voice_conn.lock().unwrap();
        let conn = slot.conn.take();
        slot.session_id = None;
        slot.token = None;
        slot.pump_spawned = false;
        conn
    };
    // Микрофон: graceful-тишина, затем отключаем отправителя (после этого слот пуст).
    // take() на отдельной строке: временный MutexGuard не должен пересекать await.
    let mic_sender = state.mic_sender.lock().unwrap().take();
    if let Some(sender) = mic_sender {
        let _ = sender.stop().await;
    }
    drop(active_conn);
    // Mixer больше не нужен (насос остановлен) — стрим-звук в него уже не пойдёт.
    *state.playback_tx.lock().unwrap() = None;
    // Выход из голоса = выход из канала → стрим смотреть больше нельзя: рвём RTC.
    crate::voice::stream::disconnect_stream(&app).await;
    // Обводки «говорит» из покинутого канала больше не актуальны.
    clear_speaking(&app, &state);
    // Флаги mute/deaf сбрасываем: насос останавливается, а флаги жили бы дальше
    // и после rejoin глушили бы приём звука/обводку.
    state.self_mute.store(false, Ordering::SeqCst);
    state.self_deaf.store(false, Ordering::SeqCst);

    // op 4 — покинуть канал
    let op4 = json!({
        "op": 4,
        "d": {
            "guild_id": null,
            "channel_id": null,
            "self_mute": false,
            "self_deaf": false
        }
    });
    let _ = sender.send(op4).await;
    let _ = app.emit("voice_status", json!({"status": "left"}));
    Ok(())
}

/// Смотреть чужой стрим (Go Live): op 20 Watch Stream. Требует, чтобы мы уже
/// были в голосовом канале (Discord отвечает STREAM_CREATE + STREAM_SERVER_UPDATE
/// либо STREAM_DELETE с причиной). RTC-коннект стрима поднимается отдельно (Шаг 1.x).
#[command]
pub(crate) async fn watch_stream(
    sender: State<'_, GatewaySender>,
    stream_key: String,
) -> Result<(), String> {
    let op20 = json!({
        "op": 20,
        "d": { "stream_key": stream_key }
    });
    sender.send(op20).await
}

/// Перестать смотреть стрим: op 19 Delete Stream (для владельца стрима — закончит его).
#[command]
pub(crate) async fn unwatch_stream(
    app: AppHandle,
    state: State<'_, DiscordState>,
    sender: State<'_, GatewaySender>,
) -> Result<(), String> {
    let key = state
        .stream
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| t.stream_key.clone());
    if let Some(stream_key) = key {
        let op19 = json!({
            "op": 19,
            "d": { "stream_key": stream_key }
        });
        let _ = sender.send(op19).await;
    }
    *state.stream.lock().unwrap() = None;
    crate::voice::stream::disconnect_stream(&app).await;
    let _ = app.emit("stream_update", json!({"watching": false}));
    Ok(())
}

/// Мьют микрофона: серверное состояние op 4 (видно другим клиентам, как у deafen)
/// + локальная пауза захвата.
#[command]
pub(crate) async fn set_self_mute(
    app: AppHandle,
    state: State<'_, DiscordState>,
    muted: bool,
) -> Result<(), String> {
    // Локальная пауза захвата (если микрофон-поток есть).
    let sender = state.mic_sender.lock().unwrap().clone();
    if let Some(sender) = sender {
        if let Err(e) = sender.set_paused(muted).await {
            log::warn!("Mute (локальная пауза): {e}");
        }
    }
    // Флаг мьюта — для обводки «говорю сам» (своё аудио не приходит).
    state.self_mute.store(muted, Ordering::SeqCst);

    // op 4 — сообщаем Discord; текущий self_deaf сохраняем, чтобы не сбросить глухоту.
    let deaf = state.self_deaf.load(Ordering::SeqCst);
    let target = state
        .voice
        .lock()
        .unwrap()
        .as_ref()
        .map(|v| (v.guild_id.clone(), v.channel_id.clone()));
    if let Some((guild_id, channel_id)) = target {
        let op4 = json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": channel_id,
                "self_mute": muted,
                "self_deaf": deaf
            }
        });
        if let Err(e) = app.state::<GatewaySender>().send(op4).await {
            log::warn!("Mute op4: {e}");
        }
    }
    Ok(())
}

/// Глухота (self_deaf): серверное состояние (op 4, видно другим клиентам),
/// пауза микрофона (в Discord deafen подразумевает mute) и гашение приёма.
#[command]
pub(crate) async fn set_self_deaf(
    state: State<'_, DiscordState>,
    app: AppHandle,
    deaf: bool,
) -> Result<(), String> {
    state.self_deaf.store(deaf, Ordering::SeqCst);

    // Deafen в Discord тянет за собой мьют микрофона; un-deaf снимает оба.
    let sender = state.mic_sender.lock().unwrap().clone();
    if let Some(sender) = sender {
        let _ = sender
            .set_paused(deaf)
            .await
            .map_err(|e| format!("deaf mute: {e}"));
    }

    // op 4 — обновить self_mute/self_deaf в голосовом состоянии.
    let target = state
        .voice
        .lock()
        .unwrap()
        .as_ref()
        .map(|v| (v.guild_id.clone(), v.channel_id.clone()));
    if let Some((guild_id, channel_id)) = target {
        let op4 = json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": channel_id,
                "self_mute": deaf,
                "self_deaf": deaf
            }
        });
        if let Err(e) = app.state::<GatewaySender>().send(op4).await {
            log::warn!("Deaf op4: {e}");
        }
    }
    Ok(())
}

/// Подключение микрофона к живому соединению. Ото требует готовой DAVE-группы
/// на момент attach (ошибка DaveRequired, сам не ретраит) — поэтому ретраим:
/// в свежем канале ExecuteTransition приходит через ~сотню мс после коннекта.
///
/// Замыкание «зажечь обводку «говорю сам»»: своя речь не приходит по сети
/// (AudioReceived ловит только чужую), поэтому ловим по громкости захваченных
/// кадров (RMS-порог в MicSource). Мьют/глушение гасят; выключение делает
/// общий watcher-таймаут (250 мс без кадров).
fn self_speak_hook(app: &AppHandle) -> std::sync::Arc<dyn Fn() + Send + Sync> {
    let state = app.state::<DiscordState>();
    let self_uid: u64 = {
        let guard = state.session.lock().unwrap();
        guard
            .as_ref()
            .and_then(|s| s.user_id.parse().ok())
            .unwrap_or(0)
    };
    let tracker = state.voice_speaking.clone();
    let mute_flag = state.self_mute.clone();
    let deaf_flag = state.self_deaf.clone();
    let app_for_speak = app.clone();
    std::sync::Arc::new(move || {
        if self_uid == 0 || mute_flag.load(Ordering::Relaxed) || deaf_flag.load(Ordering::Relaxed) {
            return;
        }
        if tracker.mark(self_uid) {
            let _ = app_for_speak.emit(
                "voice_speaking",
                json!({"user_id": self_uid.to_string(), "speaking": true}),
            );
        }
    })
}

pub(crate) async fn attach_mic_task(app: AppHandle, conn: oto::VoiceConnection) {
    let state = app.state::<DiscordState>();
    let on_loud = self_speak_hook(&app);
    for attempt in 0..8u32 {
        if state.mic_sender.lock().unwrap().is_some() {
            return; // уже подключён (живой pacer пережил replace_voice_info)
        }
        match MicSource::new(on_loud.clone()) {
            Ok(mic) => match conn.start_audio(mic).await {
                Ok(sender) => {
                    state.mic_sender.lock().unwrap().replace(sender);
                    log::info!("Микрофон подключён к голосовому каналу");
                    return;
                }
                Err(e) => {
                    if attempt == 7 {
                        log::warn!("Не удалось начать отправку аудио: {e}");
                    }
                }
            },
            Err(e) => {
                log::warn!("Микрофон недоступен (голос без микрофона): {e}");
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
}

/// Монитор микрофона/пасера: oto сам не логирует фазы отправки. Раз в 5 сек
/// снимаем снапшот: фазу (при смене) и счётчики (когда Sending — тревога).
/// Позволяет увидеть, где пасер оказался после «внезапного» обрыва.
pub(crate) async fn mic_stats_task(app: AppHandle) {
    use std::time::Instant;

    let mut last_phase: Option<String> = None;
    let mut last_frames_key: (String, u64, u64) = (String::new(), 0, 0);
    // Терминальный Failed (SendIo при обрыве сети, контракт poll и т.п.):
    // 3 быстрые попытки replace_source/переattach, далее раз в 10 с без конца —
    // сеть нередко возвращается через десятки секунд.
    let mut fails_in_a_row: u32 = 0;
    let mut last_fail_attempt_ms: u64 = 0;
    let mut last_missing_attach: Option<std::time::Instant> = None;
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        let sender = {
            let state = app.state::<DiscordState>();
            let guard = state.mic_sender.lock().unwrap();
            guard.clone()
        };
        let Some(sender) = sender else {
            // Пасера нет: либо не в голосе, либо attach_mic_task исчерпал
            // свои 8 попыток и вышел. Пока пользователь в голосовом канале —
            // пробуем снова (не чаще раза в 5 с), иначе после обрыва сети
            // канал останется без микрофона до перезахода.
            let in_voice = app.state::<DiscordState>().voice.lock().unwrap().is_some();
            if in_voice {
                let due = last_missing_attach.is_none_or(|t| t.elapsed().as_secs() >= 5);
                if due {
                    last_missing_attach = Some(Instant::now());
                    if let Some(conn) = app
                        .state::<DiscordState>()
                        .voice_conn
                        .lock()
                        .unwrap()
                        .conn
                        .clone()
                    {
                        log::info!("Микрофон: отправителя нет, соединение живо — переattach");
                        tokio::spawn(attach_mic_task(app.clone(), conn));
                    }
                }
            }
            continue;
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let snap = sender.state();
        let stats = snap.stats();
        let phase = format!("{:?}", snap.phase());

        if matches!(snap.phase(), oto::AudioPhase::Failed) {
            fails_in_a_row = fails_in_a_row.saturating_add(1);
            // Первые 3 попытки ежесекундно, дальше раз в 10 с — и без конца.
            // Бывший вечный cap («не молотим») молчал ровно с возвратом сети:
            // attach успевал умереть 3 раза за обрыв Wi-Fi, и микрофон
            // оставался мёртвым до перезахода в канал.
            if fails_in_a_row > 3 && now_ms.saturating_sub(last_fail_attempt_ms) < 10_000 {
                continue;
            }
            last_fail_attempt_ms = now_ms;
            log::warn!(
                "Микрофон/пасер: Failed (failure={:?}) — авто-восстановление, попытка {fails_in_a_row}",
                snap.failure(),
            );
            // Мягкий путь: заменить источник в живой задаче аудио.
            match MicSource::new(self_speak_hook(&app)) {
                Ok(src) => match sender.replace_source(src).await {
                    Ok(generation) => {
                        log::info!(
                            "Микрофон/пасер: источник заменён (generation={}), отправка возобновится",
                            generation.get()
                        );
                        continue;
                    }
                    // Жёсткий путь: задача аудио умерла (канал закрыт) — слот
                    // audio_active freed, переattachиваем заново как при входе в канал.
                    Err(e) => {
                        log::warn!("Микрофон/пасер: replace_source не удался ({e}) — переattach");
                        let state = app.state::<DiscordState>();
                        state.mic_sender.lock().unwrap().take();
                        // Метка для ветки «пасера нет»: не спавнить второй
                        // attach, пока этот ещё крутит свои попытки.
                        last_missing_attach = Some(Instant::now());
                        let conn = state.voice_conn.lock().unwrap().conn.clone();
                        match conn {
                            Some(conn) => {
                                tokio::spawn(attach_mic_task(app.clone(), conn));
                            }
                            None => {
                                log::warn!(
                                    "Микрофон/пасер: живого соединения нет, переattach отменён"
                                )
                            }
                        }
                        continue;
                    }
                },
                Err(e) => {
                    log::warn!("Микрофон/пасер: устройство снова не открылось: {e}");
                    continue;
                }
            }
        }
        if matches!(snap.phase(), oto::AudioPhase::Sending) {
            fails_in_a_row = 0;
        }

        if last_phase.as_deref() != Some(phase.as_str()) {
            log::info!(
                "Микрофон/пасер фаза: {} (frames={} silence={} unavailable={})",
                phase,
                stats.frames_sent(),
                stats.silence_frames_sent(),
                stats.frames_unavailable(),
            );
            last_phase = Some(phase.clone());
        }
        // Скорость потребления: кадров/с (норма ~50 при 20 мс тике). Если
        // стабильно меньше — пасер подсвечивает недобор и голос копится в кольце.
        if phase == last_frames_key.0 {
            let dt = (now_ms - last_frames_key.1) as f64;
            if dt >= 15_000.0 {
                let rate = (stats.frames_sent() as f64 - last_frames_key.2 as f64) * 1000.0 / dt;
                log::debug!("Микрофон/пасер скорость потребления: {rate:.1} кадров/с");
                last_frames_key = (phase.clone(), now_ms, stats.frames_sent());
            }
        } else {
            last_frames_key = (phase.clone(), now_ms, stats.frames_sent());
        }
        if !matches!(snap.phase(), oto::AudioPhase::Sending) {
            log::warn!(
                "Микрофон/пасер не отправляет: фаза {} unavailable={} skipped={} failures={} overruns={}",
                phase,
                stats.frames_unavailable(),
                stats.skipped_deadlines(),
                stats.send_failures(),
                stats.source_overruns(),
            );
        }
    }
}

/// Индикатор «говорит»: если от участника нет кадров дольше HOLD — гасим обводку.
/// Зажигает их насос oto по кадрам AudioReceived (входящий SPEAKING oto не отдаёт).
pub(crate) async fn speaking_watch_task(app: AppHandle) {
    const HOLD: Duration = Duration::from_millis(250);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tick.tick().await;
        let state: State<'_, DiscordState> = app.state();
        for uid in state.voice_speaking.tick(HOLD) {
            let _ = app.emit(
                "voice_speaking",
                json!({"user_id": uid.to_string(), "speaking": false}),
            );
        }
    }
}

/// Сброс индикаторов «говорит» (выход из канала/обрыв соединения).
pub(crate) fn clear_speaking(app: &AppHandle, state: &DiscordState) {
    for uid in state.voice_speaking.clear() {
        let _ = app.emit(
            "voice_speaking",
            json!({"user_id": uid.to_string(), "speaking": false}),
        );
    }
}

/// Голосовое подключение к серверу Discord:
/// Voice WebSocket (identify → READY с ssrc) → UDP (IP Discovery → Select Protocol → Heartbeat).
/// Повторный op 4 join текущего канала: Discord выдаст свежую пару (session, token).
/// Нужно после 4014/4022 NeedsFreshVoiceInfo — токен одноразовый и мог быть потрачен.
pub(crate) fn request_voice_rejoin(app: &AppHandle, state: &DiscordState) {
    let attempts = state.voice_rejoin.fetch_add(1, Ordering::SeqCst) + 1;
    if attempts > VOICE_REJOIN_LIMIT {
        log::warn!("Voice: re-join не запрошен — исчерпан лимит ({VOICE_REJOIN_LIMIT})");
        let _ = app.emit(
            "voice_status",
            json!({"status": "error", "error": crate::i18n::t(crate::i18n::Key::FreshVoicePair)}),
        );
        return;
    }
    let target = state
        .voice
        .lock()
        .unwrap()
        .as_ref()
        .map(|v| (v.guild_id.clone(), v.channel_id.clone()));
    let Some((guild_id, channel_id)) = target else {
        return;
    };
    // Актуальные флаги вместо жёстких false/false — re-join не должен
    // размьючивать/разглушать (состояние живёт в атомиках, см. set_self_mute).
    let self_mute = state.self_mute.load(Ordering::SeqCst);
    let self_deaf = state.self_deaf.load(Ordering::SeqCst);
    log::info!("Voice: запрашиваю свежую voice-пару (re-join #{attempts})");
    let app = app.clone();
    tokio::spawn(async move {
        let sender = app.state::<GatewaySender>();
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
            log::warn!("Voice re-join send: {e}");
        }
    });
}
