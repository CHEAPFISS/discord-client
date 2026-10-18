use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use nnnoiseless::DenoiseState;
use oto::{FrameSource, FrameStatus};

/// Микрофон: cpal-capture → кольцевая буфер → Opus-кадр 20 мс / 48 кГц.
/// poll_frame не спит и не блокирует: недостатке данных — Pending (oto подождёт
/// своего следующего тика, пауза заполняется тишиной).
pub(crate) struct MicSource {
    _stream: cpal::Stream,
    /// Готовые закодированные кадры, выкладываемые рабочим потоком (FIFO).
    encoded: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// Waker пасера oto: poll регистрирует себя, когда очередь пуста; рабочий
    /// поток будит пасера после выкладки кадра (контракт FrameSource).
    waker: Arc<Mutex<Option<Waker>>>,
    /// Сигнальный condvar кольца: рабочий поток ждёт на нём сэмплы; Drop будит.
    ring_cv: Arc<Condvar>,
    /// Флаг остановки рабочего потока (ставится в Drop).
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
}

pub(crate) const MIC_FRAME_SAMPLES: usize = 960; // 20 мс моно при 48 кГц
                                                 // Потолок ~80 мс: часы микрофона и пасера расходятся, а пасер после каждого
                                                 // запаздывания пере-планирует дедлайн «от текущего момента» — без жёсткого
                                                 // потолка задержка растёт бесконечно (и целиком остаётся после unmute).
pub(crate) const MIC_RING_MAX: usize = MIC_FRAME_SAMPLES * 4;

/// RMS-порог «говорю сам»: своя речь не приходит по сети, обводку зажигаем по
/// громкости кадра после шумодава (речь ≫ 0.1, остатки шума ≈ 0.001).
const SELF_VAD_RMS: f32 = 0.04;

impl MicSource {
    pub(crate) fn new(on_loud: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("нет входного аудиоустройства (микрофон)")?;

        let mut chosen = None;
        'cfg: for range in device
            .supported_input_configs()
            .map_err(|e| e.to_string())?
        {
            if range.sample_format() == cpal::SampleFormat::F32 && range.channels() == 1 {
                if range.min_sample_rate().0 <= 48000 && 48000 <= range.max_sample_rate().0 {
                    chosen = Some(range.with_sample_rate(cpal::SampleRate(48000)));
                    break 'cfg;
                }
            }
        }
        let config = chosen
            .ok_or("нет подходящей конфигурации микрофона (f32, моно, 48 кГц)")?
            .config();

        let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let ring_cv = Arc::new(Condvar::new());
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let encoded: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
        let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
        // Шумодав можно выключить для A/B-теста качества: DSC_DENOISE=0.
        let denoise_enabled = std::env::var("DSC_DENOISE")
            .map(|v| !matches!(v.as_str(), "0" | "false" | "off"))
            .unwrap_or(true);

        // Рабочий поток: вся тяжёлая работа (RNNoise + opus) — вне poll. Контракт
        // FrameSource ограничивает poll двумя мс CPU на кадр; денойзер (пара
        // прогона по 480 сэмплов) вместе с кодированием туда не влезал — oto
        // ронял отправку FrameSourceContract'ом каждые ~2 минуты. Теперь poll
        // только отдаёт готовый кадр из очереди (микросекунды).
        {
            let ring = ring.clone();
            let ring_cv = ring_cv.clone();
            let stop_flag = stop_flag.clone();
            let encoded = encoded.clone();
            let waker = waker.clone();
            std::thread::Builder::new()
                .name("mic-encode".into())
                .spawn(move || {
                    let mut encoder =
                        opus::Encoder::new(48000, opus::Channels::Stereo, opus::CodingMode::Audio)
                            .expect("opus encoder");
                    // 64 kbps: естественный потолочный битрейт Discord-голоса.
                    encoder.set_bitrate(64_000).expect("opus bitrate");
                    let mut denoiser = DenoiseState::new();
                    // Первый выход модели — fade-in нулевой истории: прогреваем
                    // состояние, но не пишем его в кадр.
                    let mut denoise_warmed = false;
                    let mut stereo = Vec::with_capacity(MIC_FRAME_SAMPLES * 2);
                    let mut mono = Vec::with_capacity(MIC_FRAME_SAMPLES);
                    let mut encoded_i: u64 = 0;

                    'worker: loop {
                        // 1) Собрать 20-мс кадр. Кольцо наполняет cpal-колбэк.
                        let mut guard = ring.lock().unwrap();
                        while guard.len() < MIC_FRAME_SAMPLES {
                            if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                break 'worker;
                            }
                            guard = ring_cv.wait(guard).unwrap();
                        }
                        mono.clear();
                        mono.extend(guard.drain(..MIC_FRAME_SAMPLES));
                        let ring_left = guard.len();
                        drop(guard);

                        encoded_i += 1;
                        if encoded_i % 250 == 0 {
                            log::debug!(
                                "Микрофон: закодировано {encoded_i} кадров, в кольце {ring_left} сэмплов"
                            );
                        }

                        // 2) RNNoise: вход/выход в масштабе i16 (±32768), а не ±1 —
                        // так требует документация модели. 960 сэмплов = 2 прогона.
                        // DSC_DENOISE=0 — прямой путь (A/B-тест качества модели).
                        if denoise_enabled {
                            let warm = denoise_warmed;
                            let mut dn_in = [0.0f32; DenoiseState::FRAME_SIZE];
                            let mut dn_out = [0.0f32; DenoiseState::FRAME_SIZE];
                            for chunk in
                                mono.chunks_exact_mut(DenoiseState::FRAME_SIZE)
                            {
                                for (d, s) in dn_in.iter_mut().zip(chunk.iter()) {
                                    *d = s * 32768.0;
                                }
                                denoiser.process_frame(&mut dn_out, &dn_in);
                                if warm {
                                    for (s, d) in chunk.iter_mut().zip(dn_out.iter()) {
                                        *s = d / 32768.0;
                                    }
                                }
                            }
                            denoise_warmed = true;
                        }

                        // 2.5) Обводка «говорю сам»: RMS кадра (после шумодава)
                        // выше порога — зажигаем наружу; watcher сам погасит.
                        if !mono.is_empty() {
                            let sum: f32 = mono.iter().map(|s| s * s).sum();
                            if (sum / mono.len() as f32).sqrt() > SELF_VAD_RMS {
                                on_loud();
                            }
                        }

                        // 3) opus 64 kbps stereo (дублируем моно в L/R, как раньше).
                        stereo.clear();
                        for s in &mono {
                            stereo.push(*s);
                            stereo.push(*s);
                        }
                        let mut enc = [0u8; 4096];
                        let len = match encoder.encode_float(&stereo, &mut enc) {
                            Ok(l) => l,
                            Err(e) => {
                                log::warn!("opus encode mic: {e}");
                                continue;
                            }
                        };
                        if len == 0 {
                            continue;
                        }

                        // 4) Выложить кадр: кап ~160 мс (8 кадров). Старейшие
                        // дропаются — для голоса свежесть важнее полноты.
                        {
                            let mut queue = encoded.lock().unwrap();
                            while queue.len() >= 8 {
                                queue.pop_front();
                            }
                            queue.push_back(enc[..len].to_vec());
                        }
                        if encoded_i == 1 {
                            log::info!("Микрофон: первый кадр {len} байт уходит в oto");
                        }
                        let wake = waker.lock().unwrap().take();
                        if let Some(w) = wake {
                            w.wake();
                        }
                    }
                })
                .map_err(|e| format!("поток кодирования микрофона: {e}"))?;
        }

        let stream = device
            .build_input_stream(
                &config,
                {
                    let ring_cb = ring.clone();
                    let ring_cv_cb = ring_cv.clone();
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mut ring = ring_cb.lock().unwrap();
                        for &s in data {
                            ring.push_back(s);
                            while ring.len() > MIC_RING_MAX {
                                ring.pop_front();
                            }
                        }
                        // Будим рабочий поток — он сам решит, накопился ли кадр,
                        // и затем разбудит пасер oto (контракт FrameSource).
                        ring_cv_cb.notify_all();
                    }
                },
                move |err| log::warn!("cpal input stream error: {err}"),
                None,
            )
            .map_err(|e| format!("cpal open input stream: {e}"))?;
        stream.play().map_err(|e| format!("cpal input play: {e}"))?;

        Ok(Self {
            _stream: stream,
            encoded,
            waker,
            ring_cv,
            stop_flag,
        })
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        // Рабочий поток ждёт на condvar кольца; будим его, чтобы он увидел
        // флаг остановки и корректно завершился.
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.ring_cv.notify_all();
    }
}

impl FrameSource for MicSource {
    fn poll_frame(&mut self, cx: &mut Context<'_>, output: &mut [u8]) -> Poll<FrameStatus> {
        let mut queue = self.encoded.lock().unwrap();
        while let Some(frame) = queue.pop_front() {
            let len = frame.len();
            if len < 3 || len > output.len() {
                // Дефектный кадр не должен случаться, но не уроним отправку из-за него.
                continue;
            }
            output[..len].copy_from_slice(&frame[..len]);
            return Poll::Ready(FrameStatus::Frame { len });
        }
        // Очередь пуста: регистрируем waker — рабочий поток будит после выкладки.
        *self.waker.lock().unwrap() = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Микрофон: cpal::Stream не Send, остальные поля — Arc с полной внутренней
/// синхронизацией (Mutex/AtomicBool/Condvar). Кадры кодирует один отдельный
/// поток; poll трогает только очередь и waker — Send безопасен вручную.
unsafe impl Send for MicSource {}
