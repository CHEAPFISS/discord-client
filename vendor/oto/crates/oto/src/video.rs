//! Депакетизатор H264 из видео-RTP (Фаза 2 — просмотр демок).
//!
//! SFU шлёт видео БЕЗ DAVE-media-шифрования: после транспортного AEAD-decrypt
//! plaintext каждого RTP-пакета — сырой H264 RTP-payload (RFC 6184). Один
//! Access Unit (кадр) обычно разбит на несколько RTP-пакетов через FU-A
//! (fragmentation unit); маркер последнего пакета = 1, таймстемп (90 кГц)
//! у всех пакетов кадра общий.
//!
//! Этот модуль складывает FU-A-фрагменты одного кадра в Annex-B буфер
//! ([00 00 00 01 NAL]…) и отдаёт готовый кадр декодеру H264 (Фаза 3).
//!
//! RTX (payload type 104) — ремонтный поток: перед [2 байта orig-seq LE][оригинальный
//! payload]. На Фазе 2 RTX складываем в карту и востребуем при пропуске seq.

use std::collections::HashMap;

/// Payload type RTX-ремонтного потока (оригинальный PT — 103, H264; отличаем
/// по SSRC из оп 12, а не по константе: PT приходит из RTP-заголовка).
pub(crate) const VIDEO_PT_RTX: u8 = 104;

/// NAL-тип FU-A (fragmentation unit) внутри RTP-payload.
const NAL_TYPE_FU_A: u8 = 28;
/// NAL-тип STAP-A (single-time aggregation, несколько коротких NAL в одном пакете).
const NAL_TYPE_STAP_A: u8 = 24;

/// Сколько Access Units держим в переупорядочивающем буфере, прежде чем
/// досрочно отдадим неполный кадр (потеря пакета). ~30 кадров ≈ 1 с.
const MAX_PENDING_FRAMES: usize = 30;

/// Собираемый FU-A — один фрагментируемый NAL внутри Access Unit.
struct FuState {
    /// Восстановленный Annex-B NAL (start-code + заголовок + накопленный payload).
    annexb: Vec<u8>,
}

/// Один незавершённый Access Unit в процессе сборки.
///
/// В одном Access Unit может быть НЕСКОЛЬКО NAL: обычно STAP-A со служебными
/// (SPS/PPS, marker=0) предшествует FU-A с самим слайсом. Поэтому целые NAL
/// (`complete`) и фрагментируемый (`fu`) хранятся РАЗДЕЛЬНО — старт нового
/// FU-A не должен выбрасывать уже накопленные SPS/PPS.
struct PendingFrame {
    /// RTP-таймстемп кадра (90 кГц) — ключ переупорядочивания и A/V-sync.
    timestamp: u32,
    /// Целые NAL, пришедшие до/вместе с FU-A (STAP-A, одиночные).
    complete: Vec<u8>,
    /// Текущий фрагментируемый NAL (None, если FU-A ещё не начинался).
    fu: Option<FuState>,
    /// Сколько пакетов уже сложено (для отладки/лимитов).
    fragments: usize,
}

impl PendingFrame {
    /// Собранный Annex-B кадр: целые NAL + фрагментируемый (если был).
    fn take_annexb(mut self) -> Vec<u8> {
        if let Some(fu) = self.fu.take() {
            self.complete.extend_from_slice(&fu.annexb);
        }
        self.complete
    }
}

/// Депакетизатор одного видео-потока (по SSRC). Stateless по крипто, stateful
/// по переупорядочиванию/сборке кадра.
#[derive(Default)]
pub(crate) struct VideoDepacketizer {
    /// Незавершённые кадры по RTP-таймстемпу.
    pending: HashMap<u32, PendingFrame>,
    /// RTX-ремонт: orig_seq → сырой payload (для восстановления пропусков).
    rtx_cache: HashMap<u16, Vec<u8>>,
    /// Последний обработанный RTP-seq (для обнаружения пропусков).
    last_seq: Option<u16>,
    /// Последний отданный кадр (для дедупликации RTX-повторов).
    last_delivered_ts: Option<u32>,
}

impl VideoDepacketizer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Обработать один транспортно-дешифрованный RTP-payload видео-потока.
    ///
    /// * `payload_type` — PT из RTP-заголовка (103 main / 104 RTX).
    /// * `marker` — M-бит (1 = последний пакет Access Unit).
    /// * `seq` — RTP-seq (порядок/пропуски/RTX-correlate).
    /// * `timestamp` — RTP-таймстемп (90 кГц, общий у кадра).
    /// * `payload` — plaintext после транспортного AEAD (сырой H264 RTP-payload).
    ///
    /// Возвращает вектор готовых кадров `[(timestamp, annexb), …]` (обычно 0-1;
    /// >1 — когда RTX-ремонт и текущий пакет завершили кадры в одном вызове).
    pub(crate) fn push(
        &mut self,
        payload_type: u8,
        marker: bool,
        seq: u16,
        timestamp: u32,
        payload: &[u8],
    ) -> Vec<(u32, Vec<u8>)> {
        // RTX (ремонтный поток): [2 байта orig-seq LE][оригинальный payload].
        // Кладём в кэш по orig_seq, но НЕ обрабатываем как обычный пакет:
        // RTX-дубль подмешал бы фрагмент к уже собранному кадру и исказил
        // слайс («QP out of range»). Востребуем из кэша только при реальном
        // пропуске seq (см. handle_fu_a).
        if payload_type == VIDEO_PT_RTX {
            if payload.len() < 2 {
                return Vec::new();
            }
            let orig_seq = u16::from_le_bytes([payload[0], payload[1]]);
            let body = &payload[2..];
            self.rtx_cache.insert(orig_seq, body.to_vec());
            // Держим кэш компактным: ~1024 пакетов ≈ 2-3 с видеопотока —
            // хватает, чтобы догнать опоздавший RTX-ремонт.
            if self.rtx_cache.len() > 1024 {
                self.rtx_cache.clear();
            }
            return Vec::new();
        }

        if payload.is_empty() {
            return Vec::new();
        }

        // Готовые кадры: восстановленные из RTX-кэша + текущий пакет.
        // (Раньше repair «return»-ил готовый кадр сразу — текущий пакет терялся,
        // а last_seq не обновлялся → ложный пропуск на следующем пакете, п.9.)
        let mut completed: Vec<(u32, Vec<u8>)> = Vec::new();

        // Восстановление пропусков seq из RTX-кэша ДО обработки текущего
        // пакета: если между last_seq и seq выпали пакеты, достаём их из
        // кэша и обрабатываем в порядке возрастания seq — иначе данные
        // слайса сдвигаются («QP out of range» / «intra mode -1»).
        if let Some(last) = self.last_seq {
            let mut expect = last.wrapping_add(1);
            let mut guard = 0u32;
            while expect != seq && guard < 64 {
                if let Some(body) = self.rtx_cache.remove(&expect) {
                    // ts восстановленного пакета: payload его не несёт, а ключовать
                    // pending нужно ПО ЕГО собственному ts — иначе фрагменты чужого
                    // кадра склеятся с текущим (п.9). Однозначно определяем: единственный
                    // кадр с активным FU, либо вообще единственный незавершённый кадр.
                    let with_fu: Vec<u32> = self
                        .pending
                        .iter()
                        .filter(|(_, frame)| frame.fu.is_some())
                        .map(|(&ts, _)| ts)
                        .collect();
                    let repair_ts = if with_fu.len() == 1 {
                        Some(with_fu[0])
                    } else if with_fu.is_empty() && self.pending.len() == 1 {
                        self.pending.keys().next().copied()
                    } else {
                        None
                    };
                    match repair_ts {
                        Some(ts) => {
                            log::debug!(
                                "VIDEO repair: восстановлен seq={expect} ({} байт)",
                                body.len()
                            );
                            if let Some(frame) = self.process_body(false, expect, ts, &body) {
                                completed.push(frame);
                            }
                        }
                        None => {
                            log::debug!("VIDEO repair: ts для seq={expect} неоднозначен — дроп");
                        }
                    }
                } else {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static MISS: AtomicUsize = AtomicUsize::new(0);
                    if MISS.fetch_add(1, Ordering::Relaxed) < 20 {
                        log::warn!("пропуск seq={expect} не восстановлен (нет в RTX-кэше)");
                    }
                }
                expect = expect.wrapping_add(1);
                guard += 1;
            }
        }
        self.last_seq = Some(seq);

        if let Some(frame) = self.process_body(marker, seq, timestamp, payload) {
            completed.push(frame);
        }
        completed
    }

    /// Обработать один (восстановленный или свежий) payload: маршрутизация
    /// по NAL-типу. Вынесено из `push`, чтобы RTX-ремонт мог переиспользовать
    /// ту же логику.
    fn process_body(
        &mut self,
        marker: bool,
        seq: u16,
        timestamp: u32,
        payload: &[u8],
    ) -> Option<(u32, Vec<u8>)> {
        let nal_type = payload[0] & 0x1f;
        match nal_type {
            NAL_TYPE_FU_A => self.handle_fu_a(marker, seq, timestamp, payload),
            NAL_TYPE_STAP_A => self.handle_stap_a(marker, timestamp, payload),
            // Одиночный NAL (1..=23) — целиком помещается в один пакет.
            1..=23 => {
                let mut frame = Vec::with_capacity(payload.len() + 4);
                frame.extend_from_slice(&[0, 0, 0, 1]);
                frame.extend_from_slice(payload);
                if marker {
                    // Утечка pending (п.10 ревью): запись мог создать раннее
                    // продолжение кадра (STAP/FU без маркера) — забираем её
                    // буфер и удаляем, а не бросаем запись в `pending` навсегда.
                    let merged = match self.pending.remove(&timestamp) {
                        Some(prev) => {
                            let mut all = prev.take_annexb();
                            all.extend_from_slice(&frame);
                            all
                        }
                        None => frame,
                    };
                    return Some(self.finish(timestamp, merged));
                }
                // Без маркера это часть агрегированного кадра — держим.
                self.append_fragment(timestamp, frame);
                None
            }
            _ => None, // зарезервировано/не поддержано
        }
    }

    /// FU-A: [FU-indicator(1)][FU-header(1)][фрагмент NAL].
    /// FU-indicator = [F|NRI|type=28]; FU-header = [S|E|R|type].
    fn handle_fu_a(
        &mut self,
        marker: bool,
        _seq: u16,
        timestamp: u32,
        payload: &[u8],
    ) -> Option<(u32, Vec<u8>)> {
        if payload.len() < 2 {
            return None;
        }
        let fu_indicator = payload[0];
        let fu_header = payload[1];
        let start = fu_header & 0x80 != 0;
        let end = fu_header & 0x40 != 0;
        let original_nal_type = fu_header & 0x1f;
        let nri = fu_indicator & 0x60;

        // Восстановленный NAL-заголовок: [F|NRI|type] из FU-indicator+header.
        let nal_header = nri | original_nal_type;

        if start {
            // Начало нового NAL. Целые NAL (`complete`: STAP-A со SPS/PPS и
            // т.п.) НЕ трогаем — они принадлежат тому же Access Unit и должны
            // уцелеть. Сбрасываем только оборванный предыдущий фрагмент.
            let mut annexb = Vec::with_capacity(payload.len() + 8);
            annexb.extend_from_slice(&[0, 0, 0, 1, nal_header]);
            annexb.extend_from_slice(&payload[2..]);
            let frame = self.pending.entry(timestamp).or_insert(PendingFrame {
                timestamp,
                complete: Vec::new(),
                fu: None,
                fragments: 0,
            });
            // ДИАГНОСТИКА: повторный start на том же ts = начало отброшено.
            if frame.fu.is_some() {
                log::warn!("VIDEO-DIAG ДУБЛЬ start: ts={timestamp} — предыдущий фрагмент заменён");
            }
            frame.fu = Some(FuState { annexb });
            frame.fragments += 1;
        } else if let Some(frame) = self.pending.get_mut(&timestamp) {
            if let Some(fu) = frame.fu.as_mut() {
                // Серединный/конечный фрагмент — дописываем полезную нагрузку.
                fu.annexb.extend_from_slice(&payload[2..]);
                frame.fragments += 1;
            } else {
                // Пропущен start-фрагмент (потеря сети) — этот NAL не собрать.
                return None;
            }
        } else {
            // Пропущен start-фрагмент (потеря сети) — кадр не собрать.
            return None;
        }

        if end || marker {
            let frame = self.pending.remove(&timestamp)?;
            return Some(self.finish(frame.timestamp, frame.take_annexb()));
        }
        None
    }

    /// STAP-A: [STAP-indicator(1)][len:16][NAL]… — агрегация коротких NAL.
    fn handle_stap_a(
        &mut self,
        marker: bool,
        timestamp: u32,
        payload: &[u8],
    ) -> Option<(u32, Vec<u8>)> {
        let mut annexb = Vec::with_capacity(payload.len() + 8);
        let mut offset = 1;
        while offset + 2 <= payload.len() {
            let len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
            offset += 2;
            if len == 0 || offset + len > payload.len() {
                break;
            }
            annexb.extend_from_slice(&[0, 0, 0, 1]);
            annexb.extend_from_slice(&payload[offset..offset + len]);
            offset += len;
        }
        if annexb.len() <= 4 {
            return None;
        }
        if marker {
            // Утечка pending (п.10 ревью): ранний STAP-A с тем же ts (marker=0,
            // обычно SPS/PPS) создал запись — забираем его буфер, удаляем запись
            // и отдаём собранный кадр целиком.
            let merged = match self.pending.remove(&timestamp) {
                Some(prev) => {
                    let mut all = prev.take_annexb();
                    all.extend_from_slice(&annexb);
                    all
                }
                None => annexb,
            };
            return Some(self.finish(timestamp, merged));
        }
        self.append_fragment(timestamp, annexb);
        None
    }

    /// Дописать целый NAL (STAP-A/одиночный, пришедший без маркера) к
    /// ожидающему Access Unit. Целые NAL копятся в `complete` и НИКОГДА не
    /// выбрасываются стартом следующего FU-A.
    fn append_fragment(&mut self, timestamp: u32, annexb: Vec<u8>) {
        let frame = self.pending.entry(timestamp).or_insert(PendingFrame {
            timestamp,
            complete: Vec::new(),
            fu: None,
            fragments: 0,
        });
        frame.complete.extend_from_slice(&annexb);
        frame.fragments += 1;
    }

    /// Финализация кадра: защита от дублей (RTX может повторить уже отданный
    /// таймстемп) + чистка старых ожидающих кадров.
    ///
    /// Возвращает готовый Annex-B кадр. Пустой вектор = уже отдан ранее
    /// (RTX-повтор), фильтруется вызывающим.
    ///
    /// NB: подклейка SPS/PPS в IDR выполняется ПОСЛЕ расшифровки DAVE
    /// ([`VideoParameterCache`] в decrypt-таске): вставка байтов ДО decrypt
    /// сдвигает unencrypted-ranges DAVE и ломает GCM-тег (п.8 ревью).
    fn finish(&mut self, timestamp: u32, annexb: Vec<u8>) -> (u32, Vec<u8>) {
        if self.last_delivered_ts == Some(timestamp) {
            // Уже отдан (RTX-повтор) — отдаём пустой кадр, фильтруется вызывающим.
            return (timestamp, Vec::new());
        }
        self.last_delivered_ts = Some(timestamp);
        self.evict_stale(timestamp);
        (timestamp, annexb)
    }

    /// Выбросить «забытые» кадры (пришли позже или потеряны), чтобы буфер
    /// не рос бесконечно при обрывах.
    ///
    /// Держим окно ±2 с вокруг текущего RTP-таймстемпа (90 кГц): прошлое старше
    /// окна уже не собрать, «будущее» за окном — аномалия. Раньше retain
    /// пропускал ЛЮБЫЕ прошедшие ts — записи копились вечно (п.10 ревью).
    fn evict_stale(&mut self, current: u32) {
        const WINDOW: u32 = 180_000; // 2 с при 90 кГц
        self.pending.retain(|&ts, _| {
            ts == current || current.wrapping_sub(ts) < WINDOW || ts.wrapping_sub(current) < WINDOW
        });
        if self.pending.len() > MAX_PENDING_FRAMES {
            // Экстренная чистка при аномальном скоплении — оставляем текущий.
            self.pending.retain(|&ts, _| ts == current);
        }
    }
}

/// Кэш parameter sets (SPS/PPS) по видео-SSRC — применяется ПОСЛЕ расшифровки DAVE.
///
/// SFU шлёт SPS/PPS STAP-A'ом перед каждым IDR (marker=0); если этот пакет
/// потерян/пропущен, декодер навсегда останется без опоры (`dsNoParamSets`).
/// Храним последние параметры каждого потока и вклеиваем их в IDR, в котором
/// их не хватает.
///
/// ВАЖНО: работает только с РАСШИФРОВАННЫМ кадром: вставка байтов до decrypt
/// сдвигает unencrypted-ranges и портит GCM-тег (п.8 ревью).
pub(crate) struct VideoParameterCache {
    per_ssrc: HashMap<u32, VideoParameters>,
}

#[derive(Default)]
struct VideoParameters {
    /// Последние SPS (NAL 7) с start-code'ом.
    sps: Vec<u8>,
    /// Последний PPS (NAL 8) с start-code'ом.
    pps: Vec<u8>,
}

impl VideoParameterCache {
    pub(crate) fn new() -> Self {
        Self {
            per_ssrc: HashMap::new(),
        }
    }

    /// Закэшировать SPS/PPS из кадра БЕЗ его модификации — можно вызывать и на
    /// ЗАШИФРОВАННОМ кадре: DAVE шифрует только VCL-NAL (слайсы), SPS/PPS в нём
    /// открыты. Это принципиально: кадр «только параметры» (SFU шлёт SPS/PPS
    /// отдельным AU) decrypt-таск дропает — без кэша здесь декодер навсегда
    /// оставался бы без опоры (dsNoParamSets = «одна картинка и вечный стоп»).
    pub(crate) fn cache(&mut self, ssrc: u32, annexb: &[u8]) {
        let params = self.per_ssrc.entry(ssrc).or_default();
        for (start, end, nal_type) in Self::spans(annexb) {
            match nal_type {
                7 => {
                    params.sps.clear();
                    params.sps.extend_from_slice(&annexb[start..end]);
                }
                8 => {
                    params.pps.clear();
                    params.pps.extend_from_slice(&annexb[start..end]);
                }
                _ => {}
            }
        }
    }

    /// Подклеить последние известные SPS/PPS к кадру, в котором их не хватает.
    ///
    /// Патчим ЛЮБОЙ кадр без параметров (не только IDR): после сброса декодера
    /// (битый слайс, потеря пакета) openh264 теряет опору — подклейка в каждом
    /// кадре даёт ей восстановиться сразу, не дожидаясь следующего GOP.
    pub(crate) fn absorb(&mut self, ssrc: u32, annexb: Vec<u8>) -> Vec<u8> {
        let params = self.per_ssrc.entry(ssrc).or_default();
        let mut has_sps = false;
        let mut has_pps = false;
        for (start, end, nal_type) in Self::spans(&annexb) {
            match nal_type {
                7 => {
                    has_sps = true;
                    params.sps.clear();
                    params.sps.extend_from_slice(&annexb[start..end]);
                }
                8 => {
                    has_pps = true;
                    params.pps.clear();
                    params.pps.extend_from_slice(&annexb[start..end]);
                }
                _ => {}
            }
        }

        if (!has_sps || !has_pps) && (!params.sps.is_empty() || !params.pps.is_empty()) {
            let mut patched =
                Vec::with_capacity(annexb.len() + params.sps.len() + params.pps.len());
            if !has_sps {
                patched.extend_from_slice(&params.sps);
            }
            if !has_pps {
                patched.extend_from_slice(&params.pps);
            }
            patched.extend_from_slice(&annexb);
            return patched;
        }
        annexb
    }

    /// Границы NAL внутри Annex-B: (start, end, nal_type) — start включает
    /// start-code, end идёт до следующего start-code (или конца кадра).
    fn spans(annexb: &[u8]) -> Vec<(usize, usize, u8)> {
        let mut scs: Vec<(usize, usize)> = Vec::new(); // (start_code_off, hdr_off)
        let mut i = 0usize;
        while i + 3 < annexb.len() {
            if annexb[i] == 0 && annexb[i + 1] == 0 {
                let (hdr, skip) = if annexb[i + 2] == 1 {
                    (i + 3, 3)
                } else if i + 4 < annexb.len() && annexb[i + 2] == 0 && annexb[i + 3] == 1 {
                    (i + 4, 4)
                } else {
                    i += 1;
                    continue;
                };
                if hdr < annexb.len() {
                    scs.push((i, hdr));
                }
                i = hdr + skip;
            } else {
                i += 1;
            }
        }
        let mut ranges = Vec::with_capacity(scs.len());
        for n in 0..scs.len() {
            let (sc, hdr) = scs[n];
            let end = if n + 1 < scs.len() {
                scs[n + 1].0
            } else {
                annexb.len()
            };
            ranges.push((sc, end, annexb[hdr] & 0x1f));
        }
        ranges
    }
}
