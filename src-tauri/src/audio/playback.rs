use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use opus::Decoder;
use tokio::sync::mpsc;

/// Воспроизведение входящего голоса: Opus-decode (per-user) → cpal output.
/// Кадр Opus: 20 мс, 48 кГц, стерео, float.
///
/// МИКШЕР — в колбэке вывода: у каждого участника СВОЯ очередь воспроизведения
/// (свой prebuffer 96 мс), суммирование голосов происходит при выборке.
/// Ранее «микширование» было сериальным: общий буфер, кадры двоих игрались
/// по очереди (40 мс речи на каждые 20 мс времени) → очередь росла и голоса
/// «лагали». Попытка выравнивать arrival-time сектора тоже не работает: кадры
/// приходят с разлётом ±10 мс и не совпадают в one общем окне.
///
/// Дрейф часов: consumed — колбэк (локальный DAC), produced — декод (RTP).
/// При N одновременно говорящих produced ≈ N×consumed, отрицательный сдвиг
/// drift'а при миксе — норма; смысл метрики — при одиночном говорящем.
pub(crate) async fn play_audio_task(rx: mpsc::Receiver<(u64, Vec<u8>, u8)>) -> Result<(), String> {
    let handle =
        tokio::task::spawn_blocking(move || -> Result<(), String> { play_audio_blocking(rx) });
    handle
        .await
        .map_err(|e| format!("join playback task: {e}"))?
}

/// Очередь воспроизведения одного участника.
struct UserPlayback {
    /// Интерлевированные декодированные сэмплы (L,R,L,R…) этого участника.
    samples: VecDeque<f32>,
    /// Свой prebuffer: поток участника начинает играть после накопления.
    started: bool,
    /// Диагностика: всего залито в очередь (enqueue) / всего сыграно (колбэк),
    /// в сэмплах. Расхождение за окно = лаг или дублирование.
    enq_total: u64,
    played_total: u64,
}

fn play_audio_blocking(mut rx: mpsc::Receiver<(u64, Vec<u8>, u8)>) -> Result<(), String> {
    // Декодер на КАЖДОГО участника: libopus хранит PLC/фазовое состояние,
    // общим кодеком кадры разных говорящих давали «булькание» при сменах.
    let mut decoders: HashMap<u64, Decoder> = HashMap::new();
    // Счётчики дрейфа часов: произведено/потреблено (см. отчёт раз в 10 с).
    let produced = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let consumed = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let consumed_cb = consumed.clone();

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(crate::i18n::t(crate::i18n::Key::NoOutputAudio))?;

    let mut chosen = None;
    'ranges: for range in device
        .supported_output_configs()
        .map_err(|e| e.to_string())?
    {
        if range.sample_format() == cpal::SampleFormat::F32
            && range.channels() == 2
            && range.min_sample_rate() <= 48000
            && 48000 <= range.max_sample_rate()
        {
            chosen = Some(range.with_sample_rate(48000).config());
            break 'ranges;
        }
    }
    let config = chosen.unwrap_or_else(|| {
        device
            .default_output_config()
            .ok()
            .map(|d| d.config())
            .unwrap_or_else(|| panic!("{}", crate::i18n::t(crate::i18n::Key::NoOutputConfig)))
    });
    if config.sample_rate != 48000 || config.channels != 2 {
        log::warn!(
            "аудио-устройство не 48 кГц стерео: {:?} — будет ресемпл на стороне ОС",
            config
        );
    }
    let config: cpal::StreamConfig = config;

    // Очереди воспроизведения участников: у каждой своя, суммирование в колбэке.
    let out_map: Arc<Mutex<HashMap<u64, UserPlayback>>> = Arc::new(Mutex::new(HashMap::new()));
    let map_clone = out_map.clone();
    let underruns = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // Де-джиттер: порог действует у КАЖДОГО участника при старте ЕГО потока —
    // ждём ~96 мс запаса (иначе всплески задержки сети истощали буфер
    // мгновенно и получались «дробления»). Дальше порог НЕ применяется:
    // ранее он действовал на каждом тике вывода — каждое проседание очереди
    // ниже 96 мс глухо обнуляло выход (в очереди ещё лежал настоящий звук!)
    // и давало периодические «прерывания с бульканием». В потоке —
    // естественный дренаж до пустой очереди; такие underrun'ы считаем.
    const PREBUFFER_SAMPLES: usize = 48000 * 2 * 96 / 1000; // 48к × 2 канала × 96 мс

    let stream = device
        .build_output_stream(
            config,
            {
                let underruns = underruns.clone();
                let consumed_cb = consumed_cb.clone();
                let map_clone = map_clone.clone();
                move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    consumed_cb
                        .fetch_add(output.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    let mut users = map_clone.lock().unwrap();
                    // Свой prebuffer у каждого участника: его поток начинает
                    // играть только после накопления запаса; прошёл порог —
                    // играет непрерывно (потери масштаба канала не глушим).
                    for up in users.values_mut() {
                        if !up.started && up.samples.len() >= PREBUFFER_SAMPLES {
                            up.started = true;
                        }
                    }
                    let any_started = users.values().any(|up| up.started);
                    // МИКС: сумма потоков начавших участников. Выходной буфер
                    // cpal не гарантированно обнулён — обнуляем явно, иначе
                    // мусор/NaN в незанятых сэмплах даёт звуковой «взрыв».
                    for sample in output.iter_mut() {
                        *sample = 0.0;
                    }
                    let mut played_any = false;
                    for up in users.values_mut() {
                        if !up.started {
                            continue;
                        }
                        let take = output.len().min(up.samples.len());
                        if take > 0 {
                            played_any = true;
                            up.played_total += take as u64;
                            for (o, s) in output.iter_mut().zip(up.samples.drain(0..take)) {
                                *o += s;
                            }
                        }
                    }
                    for sample in output.iter_mut() {
                        *sample = sample.clamp(-1.0, 1.0);
                    }
                    if !played_any && any_started {
                        // Все начавшие участницы на тике сухие (очереди
                        // опустели: сеть/загрузка) — посчитаем underrun-тик.
                        underruns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            move |err| log::warn!("cpal stream error: {err}"),
            None,
        )
        .map_err(|e| format!("cpal open stream: {e}"))?;
    stream.play().map_err(|e| format!("cpal play: {e}"))?;

    let mut pcm = vec![0.0f32; 5760 * 2]; // максимум кадра Opus (120 мс) × 2 канала
    let silent_slot = vec![0.0f32; 960 * 2]; // 20 мс стерео — для дыры без FEC
                                             // Декодированное уходит в ОЧЕРЕДЬ УЧАСТНИКА (суммирование — в колбэке).
                                             // Кап 2 с на участника — аварийный (от операции в двух секундах от
                                             // молчащего колонка убегать нечем: гнать с головы).
    let enqueue = |uid: u64, samples: &[f32]| {
        let mut users = out_map.lock().unwrap();
        let up = users.entry(uid).or_insert_with(|| UserPlayback {
            samples: VecDeque::new(),
            started: false,
            enq_total: 0,
            played_total: 0,
        });
        up.samples.extend(samples.iter().copied());
        up.enq_total += samples.len() as u64;
        while up.samples.len() > 48000 * 2 * 2 {
            up.samples.pop_front();
        }
        produced.fetch_add(samples.len() as u64, std::sync::atomic::Ordering::Relaxed);
    };
    // Раз в 10 с: глубина джиттер-очереди, underrun'ы и дрейф часов (окно):
    // положительный drift = потребление обгоняет производство — буфер убывает.
    let mut last_report = Instant::now();
    let mut last_cons: u64 = 0;
    let mut last_prod: u64 = 0;
    // Снапшот per-user счётчиков для приростов в отчёте (окно 10 с).
    let mut last_per_user: HashMap<u64, (u64, u64)> = HashMap::new();
    while let Some((user_id, opus, missing)) = rx.blocking_recv() {
        // Скрытие потерь (семантика libopus): FEC-данные лежат в ТЕКУЩЕМ пакете
        // и описывают ПОТЕРЯННЫЙ предыдущий. Один декод текущего пакета с
        // fec=true вернёт восстановленный предыдущий кадр; более старые
        // пропуски — тишина. (Раньше повторно декодировали старый пакет —
        // дублировали ~20 мс и давали «пузыри» при missing≥2.)
        let decoder = decoders.entry(user_id).or_insert_with(|| {
            Decoder::new(48000, opus::Channels::Stereo).expect("opus decoder per-user")
        });
        // FEC-концеляция убрана: libopus восстанавливает потерянный кадр
        // декодированием *текущего* пакета с fec=true, но затем его own
        // аудио уже не достать без мусора в состоянии декодера. Декодировать
        // текущий пакет дважды (fec=true, потом ещё раз) — двойное
        // воспроизведение каждого кадра после потери — растягивает звук.
        // Потери были 2 за 4 минуты — тишина на их месте незаметна.
        if missing > 0 {
            for _ in 0..missing {
                enqueue(user_id, &silent_slot);
            }
        }
        let len = match decoder.decode_float(&opus, &mut pcm, false) {
            Ok(len) => len,
            Err(e) => {
                log::warn!("opus decode: {e}");
                continue;
            }
        };
        if len == 0 {
            continue;
        }
        enqueue(user_id, &pcm[..len * 2]);
        if last_report.elapsed() >= Duration::from_secs(10) {
            last_report = Instant::now();
            let und = underruns.load(std::sync::atomic::Ordering::Relaxed);
            let cons = consumed.load(std::sync::atomic::Ordering::Relaxed);
            let prod = produced.load(std::sync::atomic::Ordering::Relaxed);
            let d_cons = cons.saturating_sub(last_cons);
            let d_prod = prod.saturating_sub(last_prod);
            last_cons = cons;
            last_prod = prod;
            let drift_ppm = if d_cons > 0 {
                (d_cons as f64 - d_prod as f64) / d_cons as f64 * 1e6
            } else {
                0.0
            };
            // Per-user диагностика микса: глубина, приток/плий за окно.
            // Расхождение притока и плея = лаг или дубли (см. «жуть»-репродукцию).
            {
                let users = out_map.lock().unwrap();
                let mut line = String::from("Mix |");
                for (uid, up) in users.iter() {
                    let (le, lp) = last_per_user.get(uid).copied().unwrap_or((0, 0));
                    let denq = up.enq_total.saturating_sub(le) as f64 / 10.0 / 1000.0;
                    let dplay = up.played_total.saturating_sub(lp) as f64 / 10.0 / 1000.0;
                    last_per_user.insert(*uid, (up.enq_total, up.played_total));
                    let depth = up.samples.len() as f64 * 1000.0 / 48000.0 / 2.0;
                    line.push_str(&format!(
                        " {:.0}мс {:>5.1}/{:>5.1}к{} ({uid}) |",
                        depth,
                        denq,
                        dplay,
                        if up.started { "▶" } else { "…" }
                    ));
                }
                if users.is_empty() {
                    line.push_str(" тишина");
                }
                let max_ms = users
                    .values()
                    .map(|up| up.samples.len() as f64 * 1000.0 / 48000.0 / 2.0)
                    .fold(0.0f64, f64::max);
                log::debug!(
                    "Playback: глубина {max_ms:.0} мс, underrun {und}, drift {drift_ppm:+.0} ppm"
                );
                log::debug!("{line}");
            }
        }
    }
    Ok(())
}
