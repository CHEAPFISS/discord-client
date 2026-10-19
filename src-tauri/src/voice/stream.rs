use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

use discord_client::{DiscordState, StreamTarget};

/// RTC-подключение к стриму (Go Live), который мы смотрим.
///
/// Стрим — ВТОРОЕ соединение параллельно с родительским голосовым: свой WS,
/// свой UDP-транспорт, своя DAVE-группа (media_session_id = rtc_server_id - 1),
/// свой SSRC-неймспейс. Родительский голосовой коннект остаётся как есть
/// (через него мы «в канале»).
///
/// Recv-only: мы НЕ шлём ни Speak, ни Video, ни медиа — только Media Sink Wants
/// (Шаг 1.4). Звук демки (Opus, SOUNDSHARE-семантика) идёт через общий mixer.

/// Подключается к стриму, если цель готова и мы ещё не подключены к ней.
/// Вызывается, когда `STREAM_CREATE` + `STREAM_SERVER_UPDATE` собрали полный
/// `StreamTarget`. Все проверки — под локом, сами подключения — вне локов.
pub(crate) fn maybe_connect_stream(app: &AppHandle) {
    let state: tauri::State<'_, DiscordState> = app.state();
    // Наша голосовая session_id в родительском канале — обязательна для stream-Identify.
    let (target, voice_session, user_id) = {
        let target = state.stream.lock().unwrap().clone();
        let voice_session = state
            .voice
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|v| v.session_id.clone());
        let user_id = state
            .session
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.user_id.clone());
        (target, voice_session, user_id)
    };
    let Some(target) = target else { return };
    if !target.ready() {
        log::debug!("Stream connect: цель ещё не собрана (пропуск)");
        return;
    }
    let Some(voice_session) = voice_session else {
        log::warn!("Stream connect: нет голосовой session_id — мы не в голосовом канале?");
        return;
    };
    let Some(user_id) = user_id else { return };

    // Дедуп: к этому же стриму уже подключены (или подключаемся).
    {
        let slot = state.stream_conn.lock().unwrap();
        let same_key = slot.session_id.as_deref() == Some(target.stream_key.as_str());
        let alive = slot.conn.as_ref().is_some_and(|c| {
            !matches!(
                c.state().phase(),
                oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed
            )
        });
        if same_key && alive {
            log::debug!("Stream: уже подключены к {}", target.stream_key);
            return;
        }
    }

    let app2 = app.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_stream_task(app2, target, voice_session, user_id).await {
            log::warn!("Stream connect: {e}");
        }
    });
}

async fn connect_stream_task(
    app: AppHandle,
    target: StreamTarget,
    voice_session: String,
    user_id: String,
) -> Result<(), String> {
    let state: tauri::State<'_, DiscordState> = app.state();
    let rtc_server = target
        .rtc_server_id
        .as_deref()
        .ok_or("stream: нет rtc_server_id")?
        .parse::<u64>()
        .map_err(|e| format!("rtc_server_id: {e}"))?;
    let rtc_channel = target
        .rtc_channel_id
        .as_deref()
        .ok_or("stream: нет rtc_channel_id")?
        .parse::<u64>()
        .map_err(|e| format!("rtc_channel_id: {e}"))?;
    let user = user_id
        .parse::<u64>()
        .map_err(|e| format!("user_id: {e}"))?;
    let endpoint = target
        .endpoint
        .as_deref()
        .ok_or("stream: нет endpoint")?
        .trim()
        .trim_start_matches("wss://")
        .to_string();
    let token = target.token.clone().ok_or("stream: нет token")?;

    log::info!(
        "Stream connect (oto): key={} rtc_server={rtc_server} rtc_channel={rtc_channel} endpoint={endpoint}",
        target.stream_key
    );

    // Своя DAVE-группа стрима: new_stream помечает коннект видео-режимом,
    // oto вычислит group_id = rtc_server - 1 (media_session_id).
    let info = oto::VoiceConnectInfo::new_stream(
        rtc_server,
        user,
        rtc_channel,
        voice_session,
        endpoint,
        oto::VoiceToken::new(token.clone()),
    );

    let oto_conn = oto::Oto::builder().build().map_err(|e| format!("oto init: {e}"))?;
    let conn = match oto_conn.connect(info).await {
        Ok(c) => c,
        Err(e) => {
            *state.stream.lock().unwrap() = None;
            let _ = app.emit(
                "stream_update",
                json!({"watching": false, "reason": format!("{:?}/{:?}", e.operation(), e.kind())}),
            );
            return Err(format!("oto connect: {e}"));
        }
    };

    log::info!(
        "Stream (oto) готово: phase={:?} rtt={:?}",
        conn.state().phase(),
        conn.state().gateway_rtt()
    );

    // Запоминаем соединение; старое (мёртвое) — сбрасываем.
    // take() + shutdown() ПОСЛЕ снятия лока: MutexGuard не должен пересекать await.
    let old = {
        let mut slot = state.stream_conn.lock().unwrap();
        let old = slot.conn.take();
        slot.conn = Some(conn.clone());
        slot.session_id = Some(target.stream_key.clone());
        slot.token = Some(token);
        slot.pump_spawned = true;
        old
    };
    if let Some(old) = old {
        let _ = old.shutdown().await;
    }

    let _ = app.emit(
        "stream_update",
        json!({"watching": true, "stream_key": target.stream_key, "owner_id": target.owner_id}),
    );

    // Насос событий стрима: фазы логируем, звук демки — в общий mixer.
    let pump_app = app.clone();
    let pump_state: DiscordState = (*state).clone();
    let stream_key = target.stream_key.clone();
    let mut sub = conn
        .subscribe_events()
        .map_err(|e| format!("oto subscribe: {e}"))?;
    tokio::spawn(async move {
        loop {
            let event = match sub.recv().await {
                Ok(ev) => ev,
                Err(oto::EventReceiveError::Lagged { skipped }) => {
                    log::warn!("Stream (oto) насос отстал: пропущено {skipped} событий");
                    continue;
                }
                Err(_) => break,
            };
            match event {
                oto::ConnectionEvent::StateChanged { phase, .. } => {
                    log::info!("Stream (oto) phase: {phase:?}");
                    if matches!(
                        phase,
                        oto::ConnectionPhase::Failed | oto::ConnectionPhase::Closed
                    ) {
                        break;
                    }
                }
                oto::ConnectionEvent::AudioReceived { user_id, opus, missing, .. } => {
                    // Звук демки — в общий mixer (ключ — id стримера: свой декодер).
                    // Deafen гасит и стрим-звук, как и голос.
                    if pump_state
                        .self_deaf
                        .load(Ordering::Relaxed)
                    {
                        continue;
                    }
                    let tx = pump_state.playback_tx.lock().unwrap().clone();
                    if let Some(tx) = tx {
                        // ОТДЕЛЬНЫЙ слот микшера для звука демки: снежные id
                        // Discord < 2^63 — ставим бит 63. Иначе звук демки и
                        // голос стримера (родительский коннект) попадали бы в
                        // ОДНУ per-user очередь и играли вперемешку по кадру
                        // («дробление»). Теперь у каждого свой джиттер-буфер,
                        // а микшер их суммирует.
                        let mix_uid = user_id | (1u64 << 63);
                        let _ = tx.send((mix_uid, opus, missing as u8)).await;
                    }
                }
                oto::ConnectionEvent::VideoStateReceived {
                    user_id,
                    audio_ssrc,
                    video_ssrc,
                    rtx_ssrc,
                } => {
                    // SSRC→user маппинг для видео (приём — Фаза 2/3). Пока логируем.
                    log::info!(
                        "Stream (oto) Video state: user={user_id} audio_ssrc={audio_ssrc} video_ssrc={video_ssrc} rtx_ssrc={rtx_ssrc}"
                    );
                }
                oto::ConnectionEvent::DaveStateChanged { opcode, before, after, .. } => {
                    log::debug!("Stream (oto) DAVE control {opcode}: {before:?} → {after:?}");
                }
                oto::ConnectionEvent::DaveWaitingForMembers { timeout_seconds, .. } => {
                    log::info!(
                        "Stream (oto) DAVE: ждёт участников (таймаут {timeout_seconds}s)"
                    );
                }
                oto::ConnectionEvent::Failure(f) => {
                    log::warn!(
                        "Stream (oto) failed: {:?}/{:?}",
                        f.operation(),
                        f.kind()
                    );
                    break;
                }
                oto::ConnectionEvent::Closed(_) => break,
                _ => {}
            }
        }
        log::info!("Stream (oto) насос завершён: {stream_key}");
        // Слот чистим только если он всё ещё про этот стрим (мог прийти новый).
        let mut slot = pump_state.stream_conn.lock().unwrap();
        if slot.session_id.as_deref() == Some(stream_key.as_str()) {
            slot.conn = None;
            slot.session_id = None;
            slot.token = None;
            slot.pump_spawned = false;
        }
        let _ = pump_app.emit("stream_update", json!({"watching": false, "stream_key": stream_key}));
    });

    Ok(())
}

/// Полный офф соединения стрима: шлём Delete Stream (op 19) и рвём RTC.
pub(crate) async fn disconnect_stream(app: &AppHandle) {
    let state: tauri::State<'_, DiscordState> = app.state();
    let conn = {
        let mut slot = state.stream_conn.lock().unwrap();
        slot.conn.take()
    };
    if let Some(c) = conn {
        let _ = c.shutdown().await;
    }
    {
        let mut slot = state.stream_conn.lock().unwrap();
        slot.session_id = None;
        slot.token = None;
        slot.pump_spawned = false;
    }
}
