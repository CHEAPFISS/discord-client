# Architecture

This document describes how the client is put together, the main data flows, and
the parts that deserve the most scrutiny. For the project's history and AI
disclosure, see [HISTORY.md](HISTORY.md) and [AI_DISCLOSURE.md](AI_DISCLOSURE.md).

## Bird's-eye view

```
┌────────────────────────────── WebKitGTK ──────────────────────────────┐
│ index.html + main.css + app.js (bundled from TypeScript)              │
│                                                                       │
│  api.ts ── invoke() ──┐          events.ts ── listen() ──┐            │
│  state.ts             │                                 │            │
│  i18n.ts              │                                 │            │
│  ui/*.ts (render)     │                                 │            │
└───────────────────────┼─────────────────────────────────┼────────────┘
                        │ Tauri IPC                       │ Tauri events
┌───────────────────────▼─────────────────────────────────▼────────────┐
│ Rust backend (src-tauri)                                              │
│                                                                       │
│  auth.rs ── REST ──► discord.com/api/v10                              │
│  gateway.rs ── WS ──► gateway.discord.gg            ── emits events   │
│  voice/mod.rs ──► voice WS + UDP (oto) ──► audio/* + voice/stream.rs  │
│  video.rs / video_render.rs ──► OpenH264 + turbojpeg ──► Channel      │
│  members.rs, api.rs, i18n.rs, lib.rs (shared state/types)             │
└───────────────────────────────────────────────────────────────────────┘
```

The app is a single Tauri window. Rust owns all network I/O, credentials and
media processing. The webview is a rendering layer: it holds UI state, receives
events, and sends commands. There is no local HTTP server and no Node.js at
runtime.

## Runtime model

- **Main thread**: Tauri/GTK event loop and the webview.
- **Tokio runtime**: Gateway WebSocket, REST calls, voice connection, audio
  send/receive tasks, member sync, video decode. Most tasks are spawned with
  `tokio::spawn` or `tauri::async_runtime::spawn`.
- **ALSA callbacks** (`cpal`): capture and playback callbacks run on audio
  threads; they exchange data with async tasks through ring buffers and channels.
- **Decoder task**: one task per active stream, decodes H.264, resizes, encodes
  JPEG and pushes frames into a bounded queue; a Tauri `Channel` carries binary
  frames to the webview.
- **Webview**: `createImageBitmap` + `canvas.drawImage` for each JPEG frame.

`DiscordState` (in `lib.rs`) is the shared backend state: token, user, guilds,
channels, members, presences, voice handles, stream handles and caches. It is
managed by Tauri and accessed via `State<'_, DiscordState>`; mutable parts use
`Mutex`/`RwLock`/atomics. Because these locks are taken from multiple async tasks,
lock ordering is a review target.

## Frontend modules

| File | Responsibility |
| --- | --- |
| `app.ts` | bootstrap: i18n, event subscriptions, initial gateway status, local handlers, session restore |
| `api.ts` | typed wrappers for every `#[tauri::command]`, shared DTO types |
| `events.ts` | typed `listen()` wrappers for backend events |
| `state.ts` | the single mutable client state object (current guild/channel, members, unread, voice flags) |
| `i18n.ts` | RU/EN dictionaries, `t()`, `data-i18n*` DOM translation, persisted language |
| `login.ts` | sign-in screen, token login, OAuth flow, session restore |
| `dom.ts` | `el()` helper |
| `ui/servers.ts` | guild rail |
| `ui/channels.ts` | channel list, categories, unread badges, joining voice |
| `ui/chat.ts` | message history, embeds/attachments, mentions, sending |
| `ui/members.ts` | role groups, presence, activities, voice participants, stream click |
| `ui/voice.ts` | voice panel, mute/deafen/leave, `voice_status` handling |
| `ui/stream.ts` | stream viewer, binary frame channel, canvas drawing, PLI-agnostic UI |
| `ui/status.ts` | gateway status pill |
| `ui/toast.ts` | error toast |
| `ui/unread.ts` | unread badge updates |

`src/app.js` is the esbuild bundle and is committed so `cargo build` works
without Node. CI runs `npm run fe:build` and fails if the committed bundle is
stale.

## Backend modules

| File | Responsibility |
| --- | --- |
| `main.rs` | process entry, logging, rustls provider, Tauri builder, command registration |
| `lib.rs` | `DiscordState`, shared DTOs (`ChatMessage`, `Member`, `PresenceInfo`, `VoiceStateInfo`), config paths, parsers, tests |
| `api.rs` | REST client (guilds, channels, roles, messages, attachments), message sending, `open_url` |
| `auth.rs` | token login, OAuth2 (authorize URL, local callback, code exchange), logout, `get_me` |
| `gateway.rs` | Gateway WS loop, identify/resume, heartbeat, event dispatch, member-list sync requests, voice state/server events |
| `members.rs` | op 37 member-list assembly, role groups, member cache |
| `voice/mod.rs` | join/leave, mute/deaf, voice status events, stream watch/unwatch, reconnect policy |
| `voice/connect.rs` | voice WS + UDP setup, DAVE negotiation, oto connection, pacer |
| `voice/stream.rs` | stream subscription, RTP video receive, PLI/REMB, decoder task lifecycle |
| `audio/mod.rs` | audio module root |
| `audio/mic.rs` | microphone capture, RNNoise, VAD, Opus encoding |
| `audio/playback.rs` | per-user decode buffers, mixer, ALSA output |
| `video.rs` | H.264 decode via OpenH264, downscale, JPEG encode, queue management |
| `video_render.rs` | bounded frame queue and Tauri `Channel` delivery |
| `i18n.rs` | backend language flag and the few localized error strings |
| `vendor/oto` | Discord voice transport fork: voice WS/UDP, DAVE, RTP, receive-side video |

## Data flows

### Login and session

1. `login_with_token` validates the token with `GET /users/@me`, stores it and
   the user in `DiscordState`, and persists the session.
2. OAuth (`start_login`) opens the browser with the configured `client_id`,
   waits for the localhost callback, exchanges the code and stores the result.
3. The UI calls `get_me` on startup; a saved session skips the login screen.
4. `logout` stops the Gateway and voice connection, clears state and files.
5. Config lives in `~/.config/discord-client/` (`config.json` with OAuth
   credentials, session/token files). Permissions are tightened to `0600`.

### Gateway

`connect_gateway` starts the WS task. It identifies, receives `READY`, then
dispatches events:

- `MESSAGE_CREATE` → `message_create` event → chat;
- `PRESENCE_UPDATE`, `GUILD_MEMBER_LIST_UPDATE` → update presence/member cache
  and emit events;
- `VOICE_STATE_UPDATE` / `VOICE_SERVER_UPDATE` → voice state machine;
- `STREAM_CREATE` / `STREAM_SERVER_UPDATE` / `STREAM_DELETE` → stream events.

The loop sends heartbeats, handles reconnects with backoff, and uses a RAII guard
to avoid a stuck "gateway running" flag. `get_gateway_status` lets a reloaded
webview recover the current status.

### Voice

1. `join_voice_channel` sends op 4 (voice state update) and waits for the
   gateway's `VOICE_STATE_UPDATE` + `VOICE_SERVER_UPDATE`.
2. `voice/connect.rs` opens the voice WS, performs identify, UDP IP discovery,
   select-protocol, and DAVE setup through oto.
3. `audio/mic.rs` captures 20 ms frames, optionally denoises, applies VAD,
   encodes Opus and feeds the pacer.
4. `audio/playback.rs` receives Opus per user, decodes, buffers, mixes and writes
   to ALSA.
5. `voice_status` events keep the UI in sync (`joining`, `connected`,
   `reconnecting`, `error`, `left`).
6. A speaking watcher emits `voice_speaking` based on incoming audio frames,
   because oto does not expose the incoming SPEAKING event.

Reconnects, channel switches and identity changes all interact here: the code
must cancel old tasks, reset mute/deaf flags, and avoid connecting twice.

### Screen-share viewing

1. `watch_stream(stream_key)` is called from `ui/stream.ts`; it subscribes to the
   stream and creates a decoder task.
2. `voice/stream.rs` receives RTP video, decrypts it through DAVE, reassembles
   H.264 access units and sends them to `video.rs`.
3. `video.rs` decodes with the system OpenH264. It requests a keyframe via RTCP
   PLI when decoding cannot start (late join/loss).
4. Frames are downscaled to 720p, encoded as JPEG with libjpeg-turbo, and pushed
   into a bounded queue (`video_render.rs`).
5. A Tauri `Channel<InvokeResponseBody>` delivers `ArrayBuffer` frames to the
   webview; `ui/stream.ts` uses `createImageBitmap` and draws to canvas, dropping
   a frame if the previous one is still being processed.
6. `unwatch_stream` tears down the pump, decoder and queue.

### Members and presence

The backend requests the op 37 member list in ranges. Discord sends group
markers (`role:<id>`, `online`, `offline`) and members in display order.
`members.rs` preserves that order, merges presence data, and returns
`MemberGroup[]`. The frontend renders groups as-is and splits online/offline
groups by live presence. This avoids reimplementing Discord's sorting rules.

### i18n

The frontend owns all UI text. `initI18n()` picks a stored language or the system
locale, applies `data-i18n*` attributes, and calls the backend `set_language`
command. Language changes are broadcast through a DOM event; dynamic widgets
re-render from state. The backend only localizes a handful of error strings it
returns directly.

## IPC contract

- Commands are `#[tauri::command]` functions; frontend wrappers live in `api.ts`.
  Names and argument shapes must match exactly (Tauri serializes with camelCase
  by default for arguments; the existing wrappers show the convention).
- Events are emitted with `app.emit(name, json!(...))`; payload types live in
  `events.ts`. There is no generated schema, so type parity is maintained by
  hand and checked only by TypeScript at call sites.
- Binary video frames use a Tauri `Channel`, not JSON, to avoid base64/JSON
  overhead.

## Security model

- The token is a bearer credential with full account access. It is stored
  locally and sent only to Discord endpoints.
- All traffic uses TLS (rustls; the crypto provider is fixed at startup).
- The webview uses a CSP that allows Discord CDN images/media but not arbitrary
  scripts.
- `open_url` only accepts `http(s)` URLs and shells out to `xdg-open`.
- There is no sandbox around the Rust backend and no privilege separation.
- Logs at `RUST_LOG=trace` can contain authorization headers and message
  content; do not share them.

## Performance notes

- The video hot path is sensitive to copies and allocations. `Cargo.toml` sets
  high optimization for dependencies and build scripts even in debug builds.
- The audio contract expects a frame roughly every 2 ms; unoptimized DSP code
  causes underruns and disconnects.
- JPEG-over-IPC is a pragmatic choice: it avoids WebCodecs availability issues
  and keeps the webview simple, at the cost of CPU encode/decode and bandwidth.
  WebCodecs is a possible future replacement.
- The frame queue is bounded; the decoder drops frames rather than growing
  latency without limit.

## Testing

- `cargo test`: parser tests for messages, presences and members; i18n language
  tag parsing; optional video decode tests (skipped without sample files).
- `npm run fe:check`: TypeScript type checking.
- `npm run i18n:check`: dictionary key and placeholder parity.
- CI runs all of the above plus a bundle-freshness check.

There are **no integration tests** for Gateway, voice or video. Those require a
real Discord account and a second peer, which is why live testing and human
review matter so much here.

## Known weak points

See [AI_DISCLOSURE.md](AI_DISCLOSURE.md) for the prioritized list. In short:
DAVE/voice state handling, RTP/RTCP, async task lifecycle, the audio deadline,
the video hot path, and hand-maintained Rust↔TypeScript types.
