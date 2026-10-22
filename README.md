# Discord Client — лёгкий клиент Discord на Rust + Tauri

Клиент Discord на Rust + Tauri v2: тёмный интерфейс в духе Material Design с бирюзовым
акцентом, без Electron и фронтенд-фреймворков. Бэкенд — Rust, интерфейс — TypeScript
и ванильный CSS, рендер — системный WebKitGTK.

## Возможности

- Вход по токену, сохранение сессии (файлы в `~/.config/discord-client/`, права `0600`).
- Серверы, категории, текстовые и голосовые каналы, счётчики непрочитанного.
- Чат: история с подгрузкой, вложения, embeds (включая gifv как видео), упоминания
  с цветом роли, спойлеры, ссылки, отправка сообщений.
- Участники: группировка по ролям, статусы, Rich Presence (активности).
- Голос: Opus, DAVE (E2EE), микшер по участникам, mute/deaf, обводка говорящего (VAD).
- Демонстрации экрана: просмотр чужих стримов (видео + звук), кнопка «В эфире»,
  полноэкранный просмотр.

## Стек

- **Rust**, [Tauri v2](https://tauri.app), tokio, reqwest, tokio-tungstenite.
- [oto](https://github.com/rayan6ms/oto) — голосовой транспорт Discord (форк с приёмом
  видео и поддержкой DAVE, вендорен в `vendor/oto`).
- OpenH264 + libjpeg-turbo + fast_image_resize — пайплайн видео демонстраций
  (декодирование → даунскейл 720p → JPEG → canvas).
- **TypeScript** + esbuild, CSS без фреймворков.

## Зависимости (Arch Linux)

```bash
sudo pacman -S --needed base-devel rust nodejs npm pkgconf \
  webkit2gtk-4.1 gtk3 libsoup3 \
  alsa-lib opus libjpeg-turbo openh264 \
  gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav
```

- `libjpeg-turbo` даёт `libturbojpeg` (pkg-config).
- GStreamer-плагины обязательны для видео/гифок в webview: без них WebKitWebProcess
  падает (SIGABRT) при попытке воспроизведения медиа.
- Rust можно поставить и через [rustup](https://rustup.rs).

На других дистрибутивах нужны эквивалентные пакеты: GTK3 + WebKitGTK 4.1, ALSA,
libopus, libjpeg-turbo, OpenH264, GStreamer с плагинами.

## Сборка и запуск

```bash
npm ci
npm run fe:build            # TypeScript → src/app.js
cd src-tauri
cargo build
./target/debug/discord-client
```

Release-сборка (LTO, opt-level 3):

```bash
cd src-tauri && cargo build --release
./target/release/discord-client
```

Разработка фронтенда: `npm run fe:watch` (пересборка `src/app.js` на лету) в одном
терминале и `cargo build` в другом.

Проверки: `npm run fe:check` (типы) и `cargo test` (юнит-тесты).

## Ограничения

- Используется **user-токен** Discord — это нарушает ToS Discord, аккаунт может быть
  заблокирован. Проект учебный, для личного использования; не разворачивайте его
  как публичный сервис.
- Демонстрации экрана — **только просмотр (recv-only)**: свой экран клиент не транслирует.
- Целевая платформа — Linux (Wayland/X11); другие ОС не тестировались.
- Видео декодируется на CPU (без аппаратного ускорения); перенос на WebCodecs —
  в планах (Фаза 5).
- Файлы сессии в `~/.config/discord-client/` содержат токен и client_secret — не
  публикуйте их и не коммитьте в репозиторий.

## Структура

```
src/                  фронтенд: TypeScript + CSS + index.html
  ui/                 экраны: чат, участники, голос, демонстрация, тосты
src-tauri/src/        бэкенд: API, шлюз, голос, видео, рендер кадров
src-tauri/src/voice/  подключение к голосу, просмотр стримов
vendor/oto/           форк oto (DAVE, приём RTP-видео, RTCP PLI/REMB)
```

## Лицензии

Форк oto в `vendor/oto/` — MIT OR Apache-2.0 (см. `LICENSE-MIT`, `LICENSE-APACHE`),
включая вендоренный `davey` (MIT). Сторонние компоненты — см.
`vendor/oto/THIRD_PARTY_NOTICES.md`.
