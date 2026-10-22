# Discord Client

[![CI](https://github.com/CHEAPFISS/discord-client/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/CHEAPFISS/discord-client/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-00acc1.svg)](LICENSE)
![Status: experimental](https://img.shields.io/badge/status-experimental-ffb74d.svg)
![Platform: Linux](https://img.shields.io/badge/platform-Linux-4dd0e1.svg)

A lightweight Discord client for Linux, built with **Rust**, **Tauri v2** and
**TypeScript** — no Electron, no frontend framework, no bundled Chromium. The UI
is rendered by the system WebKitGTK engine; voice, screen-share viewing and
Discord Gateway handling live in Rust.

> [!WARNING]
> **This codebase was written almost entirely by AI.** AI coding agents produced
> the implementation; the human owner wrote the requirements, ran the app, tested
> voice/screen-share sessions and made product decisions. It has **not** been
> reviewed line-by-line by a professional engineer. Treat it as an experimental
> prototype, not as production software. See
> [docs/AI_DISCLOSURE.md](docs/AI_DISCLOSURE.md) for the highest-risk areas and
> what a human review should focus on.

> [!CAUTION]
> **Not affiliated with Discord Inc.** This client uses a **user token**, which
> violates Discord's Terms of Service and can lead to permanent account
> termination. The MIT license covers the code only — it does not grant
> permission to use Discord's service in violation of their terms. Read
> [docs/LEGAL.md](docs/LEGAL.md) before using or distributing this project.

## Features

- **Sign-in** with a user token or the OAuth2 flow; the session is persisted in
  `~/.config/discord-client/` with `0600` permissions.
- **Servers and channels**: guild rail, categories, text/voice channels, unread
  badges.
- **Chat**: paginated history, embeds (links, images, video, `gifv`), attachments,
  mentions with role colors, spoilers, clickable links, message sending.
- **Member list**: role groups in Discord's own order, presence, custom status,
  Rich Presence activities, bot badges, voice participants.
- **Voice**: Opus, DAVE/E2EE, per-user mixing, mute/deafen, speaking indicator
  (VAD), reconnect handling.
- **Screen-share viewing** (receive-only): H.264 decode via the system OpenH264,
  downscale to 720p, JPEG over IPC, rendered on a `<canvas>`; stream audio is
  mixed into the voice output. Includes RTCP PLI so the streamer keeps sending
  keyframes.
- **RU/EN interface** with an instant language switch (see [docs/I18N.md](docs/I18N.md)).

## Status

| Area | State |
| --- | --- |
| Text chat, guilds, channels | Works |
| Voice + DAVE/E2EE | Works; tested on Linux with real calls |
| Screen-share viewing | Works (recv-only) |
| Screen-share broadcasting | Not implemented |
| Voice messages, DMs, friends, threads | Not implemented |
| Windows/macOS | Not tested; Linux is the only target |
| Accessibility audit | Not done |
| Security review | **Not done** — see [SECURITY.md](SECURITY.md) |

## Tech stack

- **Rust** + [Tauri v2](https://tauri.app), tokio, reqwest, tokio-tungstenite.
- **[oto](https://github.com/rayan6ms/oto)** — Discord voice transport. The fork
  in [`vendor/oto`](vendor/oto) adds receive-side video, RTCP PLI/REMB and DAVE
  diagnostics. It is vendored so the build is reproducible.
- **OpenH264** (system library, dynamically loaded), **libjpeg-turbo** and
  **fast_image_resize** for the screen-share pipeline.
- **TypeScript** + esbuild, plain CSS with Material-style tokens.

## Requirements (Linux)

### Arch Linux

```bash
sudo pacman -S --needed base-devel rust nodejs npm pkgconf \
  webkit2gtk-4.1 gtk3 libsoup3 \
  alsa-lib opus libjpeg-turbo openh264 \
  gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav
```

- `libjpeg-turbo` provides `libturbojpeg` (pkg-config).
- The GStreamer plugins are required for images/GIF/video inside the WebKit
  webview. Without them `WebKitWebProcess` can abort (SIGABRT) when media is
  played.
- Rust can also be installed with [rustup](https://rustup.rs).

### Debian/Ubuntu

The app is expected to build with these packages (CI installs the build-time
subset):

```bash
sudo apt-get install -y \
  build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev librsvg2-dev libxdo-dev \
  libasound2-dev libopus-dev libturbojpeg0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav
```

At runtime the screen-share viewer also needs the system **OpenH264** shared
library (`libopenh264.so`, package `openh264` on Arch, `libopenh264-7` on
Ubuntu). It is loaded dynamically; a missing library disables video decoding but
does not prevent the app from starting.

## Build and run

```bash
npm ci
npm run check                 # type-check + i18n check + bundle
cd src-tauri
cargo build
./target/debug/discord-client
```

Release build:

```bash
cd src-tauri
cargo build --release
./target/release/discord-client
```

Development loop:

```bash
npm run fe:watch              # rebuild src/app.js on change
cd src-tauri && cargo build   # rebuild the backend
```

`src/app.js` is a generated bundle and is committed so the Tauri app can be
built without running esbuild first. Run `npm run fe:build` after editing
TypeScript and commit the result.

### Tests and checks

```bash
npm run check                 # tsc --noEmit, i18n key parity, esbuild
cd src-tauri && cargo test    # Rust unit tests (parsers, member mapping, video)
```

The video tests read `/tmp/opencode/ref.264` and `/tmp/opencode/idr.bin` when
present and skip otherwise, so they are safe in CI.

## Internationalization

The interface ships in **English** and **Russian**. The language is chosen from
the OS locale on first launch, can be switched from the sign-in screen or the
user area, and is stored in `localStorage` (`discord-client.lang`). All UI
strings live in [`src/locales/en.json`](src/locales/en.json) and
[`src/locales/ru.json`](src/locales/ru.json); `npm run i18n:check` fails if the
dictionaries diverge or use different `{placeholders}`. Adding a language is
documented in [docs/I18N.md](docs/I18N.md).

## Architecture

```
┌──────────────────────────── WebKitGTK (UI) ────────────────────────────┐
│ index.html · main.css · app.js (TypeScript bundle)                     │
│ api.ts / events.ts / state.ts / i18n.ts / ui/*.ts                       │
└───────────────▲──────────────────────────────────┬─────────────────────┘
                │ Tauri commands / events           │ invoke()
┌───────────────┴──────────────────────────────────▼─────────────────────┐
│ Rust backend (src-tauri)                                                │
│  api.rs · auth.rs · gateway.rs · members.rs · video.rs · voice/*        │
│  tokio tasks: gateway WS · voice WS/UDP · audio mixer · video decode    │
└───────────────▲──────────────────────────────────┬─────────────────────┘
                │                                   │
        Discord REST / Gateway              oto (voice + DAVE)
        Discord voice gateway/UDP           OpenH264 · libjpeg-turbo
```

A detailed module map, data flows and known weak points are in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## How we got here

The project started as a Tauri scaffold on 2026-10-16 and grew through a
hand-written Discord Gateway client, a painful hand-rolled voice implementation,
a vendored `oto` fork with DAVE, an audio mixer, a large backend refactor, a
TypeScript/Material UI rewrite, member-list and Rich Presence work, and finally a
receive-only H.264 screen-share pipeline. The full story — including dead ends
and the bugs that shaped the design — is in [docs/HISTORY.md](docs/HISTORY.md).

## Limitations and legal notes

- **User tokens violate Discord's ToS.** Accounts can be terminated. This is a
  personal/educational project; do not run it as a public service.
- **Screen sharing is receive-only.** The client can watch someone else's stream
  but cannot broadcast your screen.
- **Linux only.** Wayland and X11 are the tested environments.
- Video is decoded on the CPU (no hardware acceleration). Moving the pipeline to
  WebCodecs is a possible future step.
- Session files contain a token (and OAuth client secret); never commit or share
  them. `RUST_LOG=trace` can log authorization headers — do not use it with a
  real account.
- There is **no warranty**. See [LICENSE](LICENSE).

## Contributing

Contributions are very welcome — especially **code review by experienced Rust,
TypeScript, WebRTC or audio engineers**. Read [CONTRIBUTING.md](CONTRIBUTING.md)
and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) first.

Good places to start:

- review the DAVE/voice and RTP/RTCP code in `src-tauri/src/voice/` and
  `vendor/oto/`;
- review the screen-share pipeline (`video.rs`, `video_render.rs`,
  `ui/stream.ts`) for memory/latency issues;
- add tests around parsing and state transitions;
- translate code comments and documentation (some comments are still Russian);
- add a language to `src/locales/`.

The repository has a PR template, issue forms and a CI workflow (frontend
type-check, i18n parity, bundle freshness, `cargo test`).

## Security

Please do **not** paste tokens, session files or full logs into issues. Report
vulnerabilities privately — see [SECURITY.md](SECURITY.md) for the available
routes.

## License

MIT — see [LICENSE](LICENSE). Third-party components and their licenses are
listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The license does not
override Discord's Terms of Service; read [docs/LEGAL.md](docs/LEGAL.md).

## Acknowledgements

- [oto](https://github.com/rayan6ms/oto) by rayan6ms and contributors — the
  Discord voice transport this project builds on.
- [davey](https://github.com/discord/davey) — Discord's DAVE/E2EE protocol
  implementation, vendored through oto.
- [Tauri](https://tauri.app), [OpenH264](https://www.openh264.org/),
  [libjpeg-turbo](https://libjpeg-turbo.org/) and the Rust ecosystem.
- Discord, for the protocol documentation and the unofficial reverse-engineering
  community.
