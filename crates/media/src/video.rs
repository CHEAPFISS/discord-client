//! Декодер видео стрима (Фаза 3 — просмотр демок).
//!
//! Путь кадра: oto-депакетизатор собрал Annex-B H264 (`VideoFrameReceived`) →
//! этот модуль декодирует его через Cisco OpenH264, при необходимости
//! даунскейлит до 720p (SIMD-фильтр fast_image_resize) и сжимает в JPEG —
//! в webview уходит ~60-100 КБ на кадр вместо 8.3 МБ сырых RGBA.
//!
//! Держим ОДИН decoder на соединение стрима. H264 — контекстно-зависимый
//! кодек: SPS/PPS и ключевой кадр (IDR) задают опору для всех последующих
//! дельта-кадров, поэтому decoder нельзя пересоздавать на каждый кадр.
//!
//! ДЕКОД И КОДИРОВКА РАЗДЕЛЕНЫ: в пачке кадров, пришедшей за один тик,
//! JPEG кодируется только у ПОСЛЕДНЕГО кадра (показываем всё равно его).
//! Промежуточные кадры лишь прогоняются через декодер ради обновления опоры
//! H264 — это экономит 40-60% CPU декод-таска (ревью-находка C1).

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use openh264::decoder::{Decoder, DecoderConfig};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use turbojpeg::{Compressor, Subsamp, YuvPlanesImage};

/// Максимальное разрешение JPEG для webview. Экран демки в UI меньше 1080p,
/// а даунскейл «2/3» (1920×1080 → 1280×720) Lanczos-фильтром даёт −56%
/// пикселей на encode/IPC/decode и МЕНЬШИЕ DCT-блоки на пиксель экрана
/// (артефакты JPEG q90 на 720p на глаз хуже заметны, чем q80 на 1080p).
const JPEG_MAX_DIM: u32 = 1280;

/// Качество JPEG (1..100). После даунскейла 90 даёт чистую картинку на
/// демках с текстом; размер кадра ~70-110 КБ — IPC-каналу по плечу.
const JPEG_QUALITY: i32 = 90;

/// Декодер одного видео-потока стрима. Держит stateful-декодер OpenH264,
/// SIMD-ресайзер и переиспользуемый JPEG-компрессор.
pub struct StreamDecoder {
    decoder: Decoder,
    resizer: Resizer,
    compressor: Compressor,
    /// Переиспользуемый выходной буфер JPEG (worst-case размер).
    jpeg_buf: Vec<u8>,
    /// Кадры даунскейла (Y/U/V, 4:2:0) — пересоздаются при смене разрешения.
    y_dst: Image<'static>,
    u_dst: Image<'static>,
    v_dst: Image<'static>,
    /// Строковые буферы на случай, если stride ≠ ширина (openh264 выравнивает
    /// планы; fast_image_resize требует плотный буфер).
    y_scratch: Vec<u8>,
    u_scratch: Vec<u8>,
    v_scratch: Vec<u8>,
    /// Последние известные размеры ВХОДНОГО кадра.
    src_size: (u32, u32),
    /// Сколько микросекунд заняло последнее JPEG-кодирование (статистика).
    last_encode_us: u64,
}

/// Сверка ABI версии загруженной libopenh264 ПОСЛЕ непроверенной загрузки
/// (ревью: «unchecked soname»). Биндинги openh264-sys2 0.9 сгенерированы под
/// openh264 2.6.x: vtable интерфейсов другой версии дал бы UB, поэтому
/// сверяем версию через стабильный C-API до первого вызова декодера.
fn check_openh264_abi(api: &OpenH264API) -> Result<(), String> {
    // Фича `source` у нас выключена — вариант один; else оставлен на случай
    // её включения.
    #[allow(irrefutable_let_patterns)]
    let OpenH264API::Libloading(loader) = api
    else {
        return Err("openh264: ожидалась libloading-загрузка API".to_string());
    };
    let get_ver = loader
        .WelsGetCodecVersion
        .as_ref()
        .map_err(|e| format!("openh264: нет символа WelsGetCodecVersion: {e}"))?;
    // SAFETY: вызов без аргументов возвращает POD-структуру версии; символ
    // экспортируется всеми версиями openh264 (часть стабильного C-API).
    let v = unsafe { (*get_ver)() };
    if v.uMajor == 2 && v.uMinor == 6 {
        log::info!(
            "openh264: ABI {}.{}.{} — версия совместима с биндингами",
            v.uMajor,
            v.uMinor,
            v.uRevision
        );
        Ok(())
    } else {
        Err(format!(
            "libopenh264 {}.{}.{} — биндинги ожидают 2.6.x (несовпадение ABI = UB)",
            v.uMajor, v.uMinor, v.uRevision
        ))
    }
}

impl StreamDecoder {
    pub fn new() -> Result<Self, String> {
        // Системная libopenh264 (Arch: собрана с ассемблером и оптимизациями
        // — встроенная сборка без nasm в debug декодировала 1080p слишком
        // медленно). Пробуем стандартные пути: имя библиотеки через ldconfig
        // не гарантировано, поэтому перебираем известные варианты.
        let mut last_err = String::from("libopenh264 не найдена");
        let mut api = None;
        for path in [
            "/usr/lib/libopenh264.so",
            "/usr/lib/libopenh264.so.8",
            "/usr/lib/libopenh264.so.2.6.0",
            "/usr/lib64/libopenh264.so",
            "libopenh264.so.8",
            "libopenh264.so",
        ] {
            // 1) Официальные бинари Cisco сверяются openh264-sys2 по SHA-256
            //    (well-known hashes) — без сюрпризов с версией.
            if let Ok(a) = OpenH264API::from_blob_path(path) {
                api = Some(a);
                break;
            }
            // 2) Дистрибутивные пакеты (Arch пересобирает openh264, SHA чужой):
            //    грузим unchecked, но версию ABI СВЕРЯЕМ сами до первого вызова —
            //    иначе vtable другой версии = UB (ревью: unchecked soname).
            // SAFETY: путь фиксирован, сверка версии ниже.
            match unsafe { OpenH264API::from_blob_path_unchecked(path) } {
                Ok(a) => match check_openh264_abi(&a) {
                    Ok(()) => {
                        api = Some(a);
                        break;
                    }
                    Err(e) => last_err = format!("{path}: {e}"),
                },
                Err(e) => last_err = format!("{path}: {e}"),
            }
        }
        let Some(api) = api else {
            return Err(format!("openh264 init: {last_err}"));
        };
        let decoder = Decoder::with_api_config(api, DecoderConfig::new())
            .map_err(|e| format!("openh264 init: {e}"))?;
        let mut compressor = Compressor::new().map_err(|e| format!("turbojpeg init: {e}"))?;
        compressor
            .set_quality(JPEG_QUALITY)
            .map_err(|e| format!("turbojpeg quality: {e}"))?;
        Ok(Self {
            decoder,
            resizer: Resizer::new(),
            compressor,
            jpeg_buf: Vec::new(),
            y_dst: Image::new(2, 2, PixelType::U8),
            u_dst: Image::new(1, 1, PixelType::U8),
            v_dst: Image::new(1, 1, PixelType::U8),
            y_scratch: Vec::new(),
            u_scratch: Vec::new(),
            v_scratch: Vec::new(),
            src_size: (0, 0),
            last_encode_us: 0,
        })
    }

    /// Время последнего JPEG-кодирования в микросекундах (статистика).
    pub fn last_encode_us(&self) -> u64 {
        self.last_encode_us
    }

    /// Прогоняет Annex-B H264-кадр через декодер БЕЗ кодирования — для
    /// промежуточных кадров пачки: контекст H264 требует обновить опору, но
    /// показывать/сжимать их незачем. Возвращает `Ok(true)`, если декодер
    /// выдал картинку (просто выбрасываем её).
    pub fn decode_frame(&mut self, annexb: &[u8]) -> Result<bool, String> {
        if annexb.is_empty() {
            return Ok(false);
        }
        let yuv = self
            .decoder
            .decode(annexb)
            .map_err(|e| format!("openh264 decode: {e}"))?;
        Ok(yuv.is_some())
    }

    /// Декодирует Annex-B H264-кадр, даунскейлит до `JPEG_MAX_DIM` (если
    /// крупнее) и сжимает YUV-планы в JPEG. `Ok(None)` — декодер копит опору
    /// (SPS/PPS/первая ключевая точка) или кадр битый, это норма.
    ///
    /// Возвращает `(width, height, jpeg-копия)` — копия нужна, чтобы
    /// вызывающий держал кадр независимо от внутреннего буфера.
    pub fn decode_jpeg(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        if annexb.is_empty() {
            return Ok(None);
        }
        let yuv = self
            .decoder
            .decode(annexb)
            .map_err(|e| format!("openh264 decode: {e}"))?;
        let Some(yuv) = yuv else {
            // Кадр не дал картинки — декодер ждёт больше данных (норма для
            // накопления SPS/PPS/ключевой точки). Это не ошибка.
            return Ok(None);
        };
        let (w, h) = yuv.dimensions();
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
            // Нечётные размеры несовместимы с 4:2:0 — пропускаем кадр.
            return Ok(None);
        }
        // Лимит разрешения ДО аллокаций: битый/враждебный SPS может назвать
        // гигантские размеры — jpeg_buf и dst-образы аллоцировались бы под
        // сотни МБ на кадр (OOM/фриз) (ревью).
        const MAX_DIM: usize = 8192;
        if w > MAX_DIM || h > MAX_DIM {
            log::warn!("Stream decoder: кадр отклонён — {w}x{h} больше лимита {MAX_DIM}");
            return Ok(None);
        }
        let strides = yuv.strides();
        let (w32, h32) = (w as u32, h as u32);
        if self.src_size != (w32, h32) {
            log::info!("Stream decoder: вход {w32}x{h32}");
            self.src_size = (w32, h32);
            // Прямые присваивания полей — безопасны рядом с borrow'ом
            // self.decoder у yuv (поля не пересекаются).
            let needed =
                turbojpeg::compressed_buf_len(w, h, Subsamp::Sub2x2).map_err(|e| e.to_string())?;
            self.jpeg_buf = vec![0u8; needed];
            let (dw, dh) = scale_2_3(w, h);
            self.y_dst = Image::new(dw, dh, PixelType::U8);
            self.u_dst = Image::new(dw / 2, dh / 2, PixelType::U8);
            self.v_dst = Image::new(dw / 2, dh / 2, PixelType::U8);
        }
        let scale =
            (w32 > JPEG_MAX_DIM || h32 > JPEG_MAX_DIM) && std::env::var("DC_NO_SCALE").is_err();
        let t0 = std::time::Instant::now();
        let (out_w, out_h, y_plane, u_plane, v_plane, ys, us, vs) = if scale {
            let (dw, dh) = scale_2_3(w, h);
            // Алгоритм ресайза выбирается для замеров (DC_RESIZE_ALG):
            // box — area-averaging по умолчанию; nearest/interp — быстрые
            // альтернативы на сравнение. DC_NO_SCALE=1 — вообще без даунскейла.
            let alg = match std::env::var("DC_RESIZE_ALG").as_deref() {
                Ok("nearest") => ResizeAlg::Nearest,
                Ok("interp") => ResizeAlg::Interpolation(FilterType::Bilinear),
                _ => ResizeAlg::Convolution(FilterType::Box),
            };
            let opts = ResizeOptions::new().resize_alg(alg);
            // Плотные view: при stride == ширина берём срез напрямую, иначе
            // копируем строки в scratch-буфер.
            let tc = std::time::Instant::now();
            let y_ref = plane_view(yuv.y(), strides.0, w, h, &mut self.y_scratch)?;
            let u_ref = plane_view(yuv.u(), strides.1, w / 2, h / 2, &mut self.u_scratch)?;
            let v_ref = plane_view(yuv.v(), strides.2, w / 2, h / 2, &mut self.v_scratch)?;
            let tr = std::time::Instant::now();
            self.resizer
                .resize(&y_ref, &mut self.y_dst, Some(&opts))
                .map_err(|e| format!("resize Y: {e}"))?;
            self.resizer
                .resize(&u_ref, &mut self.u_dst, Some(&opts))
                .map_err(|e| format!("resize U: {e}"))?;
            self.resizer
                .resize(&v_ref, &mut self.v_dst, Some(&opts))
                .map_err(|e| format!("resize V: {e}"))?;
            #[cfg(debug_assertions)]
            if std::env::var("DC_VIDEO_BENCH").is_ok() {
                eprintln!(
                    "  copy={}мс conv={}мс (stride {} vs w {})",
                    tr.duration_since(tc).as_micros() / 1000,
                    std::time::Instant::now().duration_since(tr).as_micros() / 1000,
                    strides.0,
                    w
                );
            }
            (
                dw,
                dh,
                self.y_dst.buffer(),
                self.u_dst.buffer(),
                self.v_dst.buffer(),
                dw as usize,
                (dw / 2) as usize,
                (dw / 2) as usize,
            )
        } else {
            (
                w32,
                h32,
                yuv.y(),
                yuv.u(),
                yuv.v(),
                strides.0,
                strides.1,
                strides.2,
            )
        };
        let t1 = std::time::Instant::now();

        let image = YuvPlanesImage {
            y_plane,
            u_plane,
            v_plane,
            width: out_w as usize,
            height: out_h as usize,
            y_stride: ys,
            u_stride: us,
            v_stride: vs,
            subsamp: Subsamp::Sub2x2,
        };
        // Поля borrow'им раздельно, чтобы компилятор не видел конфликт с
        // yuv (self.decoder) и буферами даунскейла (self.*_dst).
        let Self {
            ref mut compressor,
            ref mut jpeg_buf,
            ..
        } = *self;
        let jpeg_len = compressor
            .compress_yuv_planes_to_slice(&image, jpeg_buf)
            .map_err(|e| format!("turbojpeg compress: {e}"))?;
        let t2 = std::time::Instant::now();
        // last_encode_us — чистое JPEG-кодирование; resize отдельно (бенч).
        self.last_encode_us = t2.duration_since(t1).as_micros() as u64;
        #[cfg(debug_assertions)]
        if std::env::var("DC_VIDEO_BENCH").is_ok() {
            eprintln!(
                "  resize={}мс jpeg={}мс",
                t1.duration_since(t0).as_micros() / 1000,
                t2.duration_since(t1).as_micros() / 1000
            );
        }
        // Отдаём копию сжатого кадра — внутренний буфер переиспользуется.
        Ok(Some((out_w, out_h, jpeg_buf[..jpeg_len].to_vec())))
    }
}

/// Целевой размер даунскейла: ровно 2/3, всегда чётный (4:2:0).
fn scale_2_3(w: usize, h: usize) -> (u32, u32) {
    let mut dw = (w * 2 / 3) & !1;
    let mut dh = (h * 2 / 3) & !1;
    if dw < 2 {
        dw = 2;
    }
    if dh < 2 {
        dh = 2;
    }
    (dw as u32, dh as u32)
}

/// Возвращает плотный (stride == width) view плана: напрямую, если openh264
/// не выравнивал, либо копирует строки в scratch и отдаёт view по нему.
fn plane_view<'a>(
    src: &'a [u8],
    stride: usize,
    w: usize,
    h: usize,
    scratch: &'a mut Vec<u8>,
) -> Result<ImageRef<'a>, String> {
    if stride == w {
        ImageRef::new(w as u32, h as u32, src, PixelType::U8).map_err(|e| e.to_string())
    } else {
        scratch.clear();
        scratch.reserve(w * h);
        for row in 0..h {
            scratch.extend_from_slice(&src[row * stride..row * stride + w]);
        }
        ImageRef::new(w as u32, h as u32, scratch, PixelType::U8).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::StreamDecoder;

    /// КОНТРОЛЬНЫЙ ТЕСТ: наш декодер обязан декодировать рабочий H264-файл
    /// (эталон x264) и отвергать файл с битым слайсом. Если эталон НЕ
    /// декодируется — сломана инициализация/конфигурация, а не данные.
    #[test]
    fn decode_reference_and_dump() {
        for path in ["/tmp/opencode/ref.264", "/tmp/opencode/idr.bin"] {
            let Ok(data) = std::fs::read(path) else {
                eprintln!("SKIP {path}: файла нет");
                continue;
            };
            let mut dec = StreamDecoder::new().expect("decoder init");
            match dec.decode_jpeg(&data) {
                Ok(Some((w, h, jpeg))) => {
                    eprintln!(
                        "RESULT {path}: DECODED {w}x{h} jpeg={}КБ",
                        jpeg.len() / 1024
                    )
                }
                Ok(None) => eprintln!("RESULT {path}: NO FRAME (None)"),
                Err(e) => eprintln!("RESULT {path}: ERROR {e}"),
            }
        }
    }

    /// Бенч: время decode+encode на 1080p-IDR (прогон 10 раз).
    #[test]
    fn bench_decode_1080p_debug() {
        let Ok(data) = std::fs::read("/tmp/opencode/idr.bin") else {
            eprintln!("SKIP: файла нет");
            return;
        };
        let mut dec = StreamDecoder::new().expect("decoder init");
        for i in 0..10 {
            let t0 = std::time::Instant::now();
            let res = dec.decode_jpeg(&data);
            let dt = t0.elapsed();
            let status = match &res {
                Ok(Some((w, h, jpeg))) => format!(
                    "FRAME {w}x{h} jpeg={}КБ encode={}мс",
                    jpeg.len() / 1024,
                    dec.last_encode_us() / 1000
                ),
                Ok(None) => "None".into(),
                Err(e) => format!("ERR {e}"),
            };
            eprintln!("BENCH decode_jpeg #{i}: {:?} {status}", dt);
        }
    }
}
