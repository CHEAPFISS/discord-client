use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

use discord_client::DiscordState;
use crate::audio::playback::play_audio_task;
use crate::voice::{attach_mic_task, clear_speaking, request_voice_rejoin};

/// Голосовое подключение Discord через oto: Voice Gateway v8 + DAVE (E2EE) + RTP.
///
/// Правила управления соединением:
/// • Токен Discord одноразовый → сериализуем подключение локом.
/// • Пара (session, token) идентифицирует попытку → дедуп по ней.
/// • Смена канала в том же guild НЕ рвёт WS: живому соединению отдаём свежую пару
///   через replace_voice_info (иначе Discord шлёт каскад 4022/4014 за рструкцию).
pub(crate) async fn connect_voice(
    app: AppHandle,
    state: DiscordState,
    guild_ids: String,
    channel_ids: String,
    session_id: String,
    token: String,
    endpoint_in: String,
) -> Result<(), String> {
    let user_id = {
        let guard = state.session.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Не залогинен")?
            .user_id
            .clone()
            .parse::<u64>()
            .map_err(|e| format!("user_id: {e}"))?
    };
    let guild = guild_ids.parse::<u64>().map_err(|e| format!("guild_id: {e}"))?;
    let channel = channel_ids.parse::<u64>().map_err(|e| format!("channel_id: {e}"))?;
    let endpoint = endpoint_in.trim().trim_start_matches("wss://").to_string();

    // Цель могла смениться/сняться, пока задача стояла в очереди (leave, новый join,
    // смена аккаунта): неактуальная попытка не тратит одноразовый токен и не воскрешает голос.
    // Проверяем ПОД локом voice_connect_lock — иначе leave успевает пройти мимо и
    // connect воскрешает голос от уже отменённой цели (зомби-канал).
    let _guard = state.voice_connect_lock.lock().await;
    {
        let voice = state.voice.lock().unwrap();
        match voice.as_ref() {
            None => {
                log::info!("Voice connect отменён: цель снята (leave/смена аккаунта)");
                return Ok(());
            }
            Some(v) if v.channel_id != channel_ids => {
                log::info!("Voice connect отменён: цель сменилась на канал {}", v.channel_id);
                return Ok(());
            }
            _ => {}
        }
    }

    // Дедуп: пара (session, token) уже применена к не-терминальному соединению.
    let dup = {
        let slot = state.voice_conn.lock().unwrap();
        match slot.conn.as_ref() {
            Some(c) => {
                let ph = c.state().phase();
                let terminal =
                    matches!(ph, oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed);
                !terminal
                    && slot.session_id.as_deref() == Some(session_id.as_str())
                    && slot.token.as_deref() == Some(token.as_str())
            }
            None => false,
        }
    };
    if dup {
        log::debug!("Voice: пара (session, token) уже применена, дедуп-пропуск");
        return Ok(());
    }

    log::info!(
        "Voice connect (oto): guild={guild} channel={channel} session={} endpoint={endpoint}",
        &session_id[..session_id.len().min(8)]
    );

    let info = oto::VoiceConnectInfo::new(
        guild,
        user_id,
        channel,
        session_id.clone(),
        endpoint,
        oto::VoiceToken::new(token.clone()),
    );

    // Запоминаем пару оптимистично (эхо-вызовы не будут повторять в meanwhile).
    let (prev, prev_alive) = {
        let mut slot = state.voice_conn.lock().unwrap();
        slot.session_id = Some(session_id.clone());
        slot.token = Some(token.clone());
        let prev = slot.conn.take();
        let alive = prev
            .as_ref()
            .is_some_and(|p| {
                !matches!(
                    p.state().phase(),
                    oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed
                )
            });
        if !alive {
            slot.pump_spawned = false; // мёртвое/новое соединение — насос заводим заново
        }
        (prev, alive)
    };

    let conn = match prev {
        Some(p) if prev_alive => {
            // Живое соединение: не рвём WS, отдаём свежую пару на месте.
            let ph = p.state().phase();
            match p.replace_voice_info(info).await {
                Ok(generation) => {
                    log::info!(
                        "Voice (oto) replace_voice_info OK (phase={ph:?}, gen={})",
                        generation.get()
                    );
                    p
                }
                Err(e) => {
                    state.voice_conn.lock().unwrap().session_id = None;
                    state.voice_conn.lock().unwrap().token = None;
                    let _ = p.shutdown().await;
                    if matches!(e.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&app, &state);
                    }
                    return Err(format!("oto replace_voice_info: {e}"));
                }
            }
        }
        Some(p) => {
            // Мёртвое соединение — закрываем и подключаемся начисто.
            // Старый sender принадлежал мёртвому конну — сбрасываем.
            state.mic_sender.lock().unwrap().take();
            let _ = p.shutdown().await;
            let oto = oto::Oto::builder().build().map_err(|e| format!("oto init: {e}"))?;
            match oto.connect(info).await {
                Ok(c) => c,
                Err(e) => {
                    state.voice_conn.lock().unwrap().session_id = None;
                    state.voice_conn.lock().unwrap().token = None;
                    if matches!(e.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&app, &state);
                    }
                    return Err(format!("oto connect: {e}"));
                }
            }
        }
        None => {
            // Начисто: прежнего соединения не было — слот микрофона тоже пуст.
            state.mic_sender.lock().unwrap().take();
            let oto = oto::Oto::builder().build().map_err(|e| format!("oto init: {e}"))?;
            match oto.connect(info).await {
                Ok(c) => c,
                Err(e) => {
                    state.voice_conn.lock().unwrap().session_id = None;
                    state.voice_conn.lock().unwrap().token = None;
                    if matches!(e.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&app, &state);
                    }
                    return Err(format!("oto connect: {e}"));
                }
            }
        }
    };

    state.voice_rejoin.store(0, Ordering::SeqCst);

    let phase = conn.state().phase();
    log::info!("Voice (oto) готово: phase={phase:?} rtt={:?}", conn.state().gateway_rtt());

    state.voice_conn.lock().unwrap().conn = Some(conn.clone());
    let _ = app.emit("voice_status", json!({"status": "connected"}));

    // Насос событий: ОДИН на соединение. Повторные replace_voice_info обслуживает
    // уже живущий насос — иначе плодим подписчиков (лимит oto: 8) и дублируем события.
    let need_pump = {
        let mut slot = state.voice_conn.lock().unwrap();
        let need = !slot.pump_spawned;
        slot.pump_spawned = true;
        need
    };
    // Канал входящих Opus-кадров (уже расшифрованных oto: user_id отфильтровано криптографически).
    // user_id едет с кадром: у каждого участника — свой opus-декодер (общий
    // кодек путал состояние и давал «булькание» при смене говорящих).
    let (audio_tx, audio_rx) = tokio::sync::mpsc::channel::<(u64, Vec<u8>, u8)>(256);
    tokio::spawn(async move {
        if let Err(e) = play_audio_task(audio_rx).await {
            log::warn!("Voice playback: {e}");
        }
    });

    if need_pump {
        let pump_app = app.clone();
        let pump_state = state.clone();
        let slot_ref = state.voice_conn.clone();
        // Микрофон: подключаем (с ретраями до готовности DAVE); при replace-пути
        // sender уже жив в pacer — attach_mic_task это увидит и завершится сразу.
        if state.mic_sender.lock().unwrap().is_none() {
            tokio::spawn(attach_mic_task(app.clone(), conn.clone()));
        }
        let mut sub = conn
            .subscribe_events()
            .map_err(|e| format!("oto subscribe: {e}"))?;
        tokio::spawn(async move {
            loop {
                let event = match sub.recv().await {
                    Ok(ev) => ev,
                    Err(oto::EventReceiveError::Lagged { skipped }) => {
                        // Переполнение кольца — не смертельно, продолжаем с новыми.
                        log::warn!("Voice (oto) насос отстал: пропущено {skipped} событий");
                        continue;
                    }
                    Err(_) => break,
                };
                // Актуальное состояние слота на момент прихода события.
                let (cur_gen, cur_phase) = {
                    let slot = slot_ref.lock().unwrap();
                    match slot.conn.as_ref() {
                        Some(c) => {
                            let s = c.state();
                            (Some(s.generation()), Some(s.phase()))
                        }
                        None => (None, None),
                    }
                };
                let ev_gen = match &event {
                    oto::ConnectionEvent::StateChanged { generation, .. } => Some(*generation),
                    oto::ConnectionEvent::Failure(f) => Some(f.generation()),
                    _ => None,
                };
                if let Some(g) = ev_gen {
                    if Some(g) != cur_gen {
                        continue; // поколение уже заменено — событие устарело
                    }
                } else if matches!(event, oto::ConnectionEvent::Closed(_))
                    && cur_phase != Some(oto::ConnectionPhase::Closed)
                {
                    continue; // Closed старого соединения — в слоте уже другое/живое
                }
                match event {
                    oto::ConnectionEvent::StateChanged { phase, .. } => {
                        log::debug!("Voice (oto) phase: {phase:?}");
                        if phase == oto::ConnectionPhase::Connected {
                            let attach = slot_ref.lock().unwrap().conn.clone();
                            if let Some(c) = attach {
                                tokio::spawn(attach_mic_task(pump_app.clone(), c));
                            }
                        }
                        let (status, err) = match phase {
                            oto::ConnectionPhase::Connected => ("connected", None),
                            oto::ConnectionPhase::Resuming
                            | oto::ConnectionPhase::Reconnecting => ("reconnecting", None),
                            oto::ConnectionPhase::NeedsFreshVoiceInfo => ("needs_fresh", None),
                            oto::ConnectionPhase::Closed => ("left", None),
                            oto::ConnectionPhase::Failed => {
                                ("error", Some("voice connection failed".to_string()))
                            }
                            _ => ("connecting", None),
                        };
                        let payload = match err {
                            Some(e) => json!({"status": "error", "error": e}),
                            None => json!({"status": status}),
                        };
                        let _ = pump_app.emit("voice_status", payload);
                    }
                    oto::ConnectionEvent::Failure(f) => {
                    log::warn!(
                        "Voice (oto) failure: op={:?} kind={:?} code={:?} retry={:?}",
                        f.operation(),
                        f.kind(),
                        f.safe_code(),
                        f.retry_disposition()
                    );
                    // NeedsFreshVoiceInfo mid-session: соединение ждёт свежую пару —
                    // сами запрашиваем re-join (op 4), иначе голос висит навсегда.
                    if matches!(f.retry_disposition(), oto::RetryDisposition::NeedsFreshVoiceInfo) {
                        request_voice_rejoin(&pump_app, &pump_state);
                        let _ = pump_app.emit("voice_status", json!({"status": "joining"}));
                    } else {
                        let _ = pump_app.emit(
                            "voice_status",
                            json!({"status": "error", "error": format!("{:?}/{:?}", f.operation(), f.kind())}),
                        );
                    }
                }
                oto::ConnectionEvent::Closed(_) => {
                    clear_speaking(&pump_app, &pump_state);
                    let _ = pump_app.emit("voice_status", json!({"status": "left"}));
                    break;
                }
                oto::ConnectionEvent::AudioReceived { user_id, opus, missing, .. } => {
                    // Индикатор «говорит»: кадр пришёл — зажигаем сразу (и при deafen:
                    // обводка должна жить, даже если звук не доигрываем).
                    if pump_state.voice_speaking.mark(user_id) {
                        let _ = pump_app.emit(
                            "voice_speaking",
                            json!({"user_id": user_id.to_string(), "speaking": true}),
                        );
                    }
                    // Кадр уже расшифрован oto (автор определён по ratchet'у
                    // отправителя). По-кадровых логов нет — до 100 кадров/с.
                    // Deafen: кадры не доигрываем вовсе (без расхода декодера).
                    if pump_app
                        .state::<DiscordState>()
                        .self_deaf
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        continue;
                    }
                    let _ = audio_tx.send((user_id, opus, missing as u8)).await;
                }
                oto::ConnectionEvent::DaveStateChanged { opcode, before, after, .. } => {
                    log::debug!("Voice (oto) DAVE control {opcode}: {before:?} → {after:?}");
                }
                oto::ConnectionEvent::DaveWaitingForMembers { timeout_seconds, .. } => {
                    log::info!("Voice (oto) DAVE: ждёт участников (таймаут {timeout_seconds}s), отправка недоступна");
                }
                oto::ConnectionEvent::DaveRecovery { attempt, .. } => {
                    log::info!("Voice (oto) DAVE: recovery, попытка {attempt}");
                }
                oto::ConnectionEvent::DaveControlProcessed {
                    opcode, transition_id, epoch, ..
                } => {
                    log::debug!(
                        "Voice (oto) DAVE control processed: op={opcode} tid={transition_id:?} epoch={epoch:?}"
                    );
                }
                _ => {}
            }
        }
    }); // need_pump
    } // need_pump

    Ok(())
}
