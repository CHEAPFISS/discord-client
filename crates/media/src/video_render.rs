//! Рендер видео стрима (Фаза 3 — просмотр демок).
//!
//! Отделяем ДЕКОДИРОВАНИЕ (блокирующее, CPU-bound, OpenH264) от НАСОСА
//! событий oto: насос не должен ждать декодер, иначе подзадерживается звук
//! и фазы. Поэтому VideoFrameReceived складывается в очередь, а отдельный
//! таск декодирует кадры и шлёт готовый JPEG в webview через бинарный
//! Tauri Channel (ArrayBuffer на фронте, без JSON-оверхеда).
//!
//! Формат канала: [u32 width LE][u32 height LE][jpeg…]. JPEG экономит IPC в
//! 30+ раз против сырых RGBA (720p: ~70-110 КБ против 8.3 МБ) — фронт
//! декодирует его асинхронно (createImageBitmap) и не дёргает UI-поток.
//!
//! Политика очереди (ревью C1/C2): кадры НЕ выбрасываются поодиночке — это
//! рвёт цепочку H264. При переполнении выкидываем всё ДО последнего IDR в
//! очереди (от него цепочка восстановима); если IDR нет — чистим всё и
//! ждём свежий ключевой кадр (один PLI, повторы не чаще 2 с).

use std::sync::{Arc, Mutex};

use crate::video::StreamDecoder;

/// Бинарный канал кадров: фронт создаёт его командой `stream_video_attach`.
/// Слот ядра — сюда попадает адаптер Tauri-слоя (см. `TauriVideoSink` в
/// src-tauri), здесь кадры уходят только через [`client_core::VideoFrameSink`].
pub type VideoChannelSlot = client_core::VideoChannelSlot;

/// Вместимость очереди кадров между насосом oto и декодер-таском. Достаточно
/// на бёрсты депакетизатора (кадры приходят пачками после marker-бита) и на
/// запас «догоняющего» декодера, не разрастаясь по памяти.
const FRAME_QUEUE_CAP: usize = 8;

/// Повтор PLI, если ждём IDR и он не приходит (SFU молчит / потерян ключ).
const PLI_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Очередь сырых Annex-B кадров от насоса к декодер-таску.
pub struct VideoIngress {
    frames: std::collections::VecDeque<IngressFrame>,
    /// Переполнение вытеснило кадры и IDR в очереди нет — ждём ключевой.
    skip_until_idr: bool,
}

/// Кадр в очереди + признак IDR (NAL type 5) — считается ОДИН раз в
/// push_frame, а не сканируется каждый тик (ревью m1).
struct IngressFrame {
    data: Vec<u8>,
    is_idr: bool,
}

/// Очередь кадров (создаётся при старте соединения стрима).
pub type VideoFrameQueue = Arc<Mutex<VideoIngress>>;

/// Создаёт пустую очередь кадров (вызывается при старте соединения стрима).
pub fn new_frame_queue() -> VideoFrameQueue {
    Arc::new(Mutex::new(VideoIngress {
        frames: std::collections::VecDeque::with_capacity(FRAME_QUEUE_CAP),
        skip_until_idr: false,
    }))
}

/// Кладёт Annex-B кадр в очередь (из насоса oto). Не блокируется — очередь
/// за коротким мьютексом. При переполнении отсекает всё до последнего IDR:
/// цепочка H264 остаётся валидной, декодер возобновит картинку сразу с него.
pub fn push_frame(queue: &VideoFrameQueue, frame: Vec<u8>) {
    let is_idr = has_idr(&frame);
    let mut q = queue.lock().unwrap();
    if q.frames.len() >= FRAME_QUEUE_CAP {
        match q.frames.iter().rposition(|f| f.is_idr) {
            Some(pos) => {
                // Оставляем от последнего IDR (включая его) — свежайшая опора.
                q.frames.drain(..pos);
                q.skip_until_idr = false;
            }
            None => {
                q.frames.clear();
                q.skip_until_idr = true;
            }
        }
    }
    q.frames.push_back(IngressFrame {
        data: frame,
        is_idr,
    });
}

/// Есть ли в Annex-B кадре IDR (NAL type 5) — точка возобновления H264.
fn has_idr(annexb: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 3 < annexb.len() {
        if annexb[i] == 0 && annexb[i + 1] == 0 {
            let next = if annexb[i + 2] == 1 {
                i + 3
            } else if i + 4 < annexb.len() && annexb[i + 2] == 0 && annexb[i + 3] == 1 {
                i + 4
            } else {
                i += 1;
                continue;
            };
            if next < annexb.len() && (annexb[next] & 0x1f) == 5 {
                return true;
            }
            i = next;
        } else {
            i += 1;
        }
    }
    false
}

/// Таск-декодер: раз ~15 мс забирает накопленные кадры из очереди, декодирует
/// их в порядке H264 (промежуточные — только ради опоры), последний кадр
/// сжимает в JPEG и шлёт фронту бинарным каналом: [u32 w LE][u32 h LE][jpeg].
///
/// Условие выхода — `stop` (watch): sender дропается или шлёт `true` при
/// реконнекте/дисконнекте стрима — иначе таск с декодером тек на каждую
/// сессию (п.6 ревью).
pub async fn decode_and_draw_task(
    queue: VideoFrameQueue,
    video_channel: VideoChannelSlot,
    mut stop: tokio::sync::watch::Receiver<bool>,
    keyframe_tx: tokio::sync::mpsc::Sender<()>,
) {
    let Ok(mut decoder) = StreamDecoder::new() else {
        log::error!("Stream video: не удалось создать декодер OpenH264");
        return;
    };
    log::info!("Stream video: декодер запущен");
    // Троттлинг PLI: один запрос при входе в ожидание IDR, повторы — не
    // чаще PLI_RETRY_INTERVAL (шторм PLI не решает ничего, ревью M2).
    let mut last_keyframe_request: Option<std::time::Instant> = None;
    let request_keyframe = |last: &mut Option<std::time::Instant>| {
        let now = std::time::Instant::now();
        let due = last
            .map(|t| now.duration_since(t) >= PLI_RETRY_INTERVAL)
            .unwrap_or(true);
        if due {
            *last = Some(now);
            if keyframe_tx.try_send(()).is_ok() {
                log::debug!("Stream video: запрошен ключевой кадр (PLI)");
            }
        }
    };
    // Статистика окна (все счётчики сбрасываются раз в 5 с — ревью M1).
    let mut stat_started = std::time::Instant::now();
    let mut stat_decoded: u64 = 0;
    let mut stat_jpeg_bytes: u64 = 0;
    let mut stat_encode_us: u64 = 0;
    let mut stat_ticks: u64 = 0;
    let mut stat_skip_ticks: u64 = 0;
    let mut decode_errors: u64 = 0;
    loop {
        // ~15 мс тика хватает для 30 fps; select на stop — корректный выход
        // (changed() вернёт Err при дропе sender'а).
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    log::info!("Stream video: декодер остановлен (стоп-сигнал)");
                    break;
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(15)) => {}
        }
        stat_ticks += 1;
        // Забираем кадры ПОД локом, а декодируем ВНЕ его: раньше лок держался
        // на время decode всех кадров — насос oto блокировался на push_frame,
        // broadcast-события отставали и терялись через Lagged (включая звук
        // демки — AudioReceived), п.4 ревью.
        let frames: Vec<IngressFrame> = {
            let mut q = queue.lock().unwrap();
            if q.skip_until_idr {
                // Ждём IDR: всё до него — мусор без опоры. IDR оставляем
                // первым кадром пачки, если он уже пришёл.
                match q.frames.iter().position(|f| f.is_idr) {
                    Some(pos) => {
                        q.frames.drain(..pos);
                        q.skip_until_idr = false;
                    }
                    None => {
                        stat_skip_ticks += 1;
                        request_keyframe(&mut last_keyframe_request);
                    }
                }
            }
            q.frames.drain(..).collect()
        };
        if frames.is_empty() {
            continue;
        }
        // Контекст H264: промежуточные кадры прогоняем через декодер ради
        // опоры БЕЗ кодирования; JPEG — только у последнего (ревью C1).
        // Декод+encode — CPU-bound (десятки мс на 1080p): в spawn_blocking,
        // иначе блокирует воркер токио (на нём же Gateway, heartbeat и голос).
        let dec = decoder;
        let blocking = tokio::task::spawn_blocking(move || {
            let mut latest: Option<(u32, u32, Vec<u8>)> = None;
            let mut decoded: u64 = 0;
            let mut jpeg_bytes: u64 = 0;
            let mut encode_us: u64 = 0;
            let mut errors: u64 = 0;
            let mut need_pli = false;
            let n = frames.len();
            let mut dec = dec;
            for (i, frame) in frames.into_iter().enumerate() {
                let is_last = i + 1 == n;
                let res = if is_last {
                    dec.decode_jpeg(&frame.data)
                } else {
                    dec.decode_frame(&frame.data)
                        .map(|_| None as Option<(u32, u32, Vec<u8>)>)
                };
                match res {
                    Ok(Some(jpeg)) => {
                        decoded += 1;
                        jpeg_bytes += jpeg.2.len() as u64;
                        encode_us += dec.last_encode_us();
                        latest = Some(jpeg);
                    }
                    Ok(None) => {} // декодер копит опору (SPS/PPS) — норма
                    Err(e) => {
                        // Декодер потерял опору (потеря пакета/сброс) — просим у
                        // отправителя свежий IDR (RTCP PLI). Без этого картинка
                        // стоит до следующего редкого ключевого кадра.
                        need_pli = true;
                        errors += 1;
                        if errors <= 2 {
                            log::warn!(
                                "Stream video decode FAIL: len={} err={e}",
                                frame.data.len()
                            );
                        }
                    }
                }
            }
            (
                dec, latest, decoded, jpeg_bytes, encode_us, errors, need_pli,
            )
        })
        .await;
        let (dec, latest, batch_decoded, batch_jpeg, batch_encode, batch_errors, need_pli) =
            match blocking {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Stream video: decode-задача упала: {e}");
                    break;
                }
            };
        decoder = dec;
        stat_decoded += batch_decoded;
        stat_jpeg_bytes += batch_jpeg;
        stat_encode_us += batch_encode;
        decode_errors += batch_errors;
        if need_pli {
            request_keyframe(&mut last_keyframe_request);
        }
        // Статистика: раз в 5 с — fps, размер/время JPEG, честные оконные
        // счётчики ошибок и skip-тиков.
        if stat_started.elapsed() >= std::time::Duration::from_secs(5) {
            let secs = stat_started.elapsed().as_secs_f64();
            let avg_kb = stat_jpeg_bytes
                .checked_div(stat_decoded)
                .map_or(0, |bytes| bytes / 1024);
            let avg_enc_ms = stat_encode_us
                .checked_div(stat_decoded)
                .map_or(0, |us| us / 1000);
            log::info!(
                "Stream video: {:.1} fps, jpeg ~{} КБ (encode ~{} мс), \
                 декод-ошибок {}, skip-тиков {} из {}",
                stat_decoded as f64 / secs,
                avg_kb,
                avg_enc_ms,
                decode_errors,
                stat_skip_ticks,
                stat_ticks
            );
            stat_started = std::time::Instant::now();
            stat_decoded = 0;
            stat_jpeg_bytes = 0;
            stat_encode_us = 0;
            stat_ticks = 0;
            stat_skip_ticks = 0;
            decode_errors = 0;
        }
        let Some((width, height, jpeg)) = latest else {
            continue;
        };
        // Бинарный IPC: [u32 w][u32 h][jpeg…].
        let sink = video_channel.lock().unwrap().clone();
        if let Some(sink) = sink {
            let mut payload = Vec::with_capacity(8 + jpeg.len());
            payload.extend_from_slice(&width.to_le_bytes());
            payload.extend_from_slice(&height.to_le_bytes());
            payload.extend_from_slice(&jpeg);
            if let Err(e) = sink.send_frame(payload) {
                // Фронт не attached / закрыл страницу — канал пере-attach'ится
                // при открытии оверлея.
                log::debug!("Stream video: канал кадров закрыт: {e}");
            }
        }
    }
}
