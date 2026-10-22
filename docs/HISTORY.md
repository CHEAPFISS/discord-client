# How we got here

This is the short history of the project: what was built, in what order, and
which problems shaped the design. It is also a record of how much was written by
AI — see [AI_DISCLOSURE.md](AI_DISCLOSURE.md).

The repository history was rewritten once, on 2026-10-22, to remove a 336 MB
`src-tauri/target` directory from every commit. Old commit hashes mentioned in
notes and chats no longer resolve; dates and messages are preserved.

## Timeline

| Date (2026) | Milestone |
| --- | --- |
| Oct 16 | Tauri v2 scaffold, OAuth2, REST, Gateway, live messages |
| Oct 17 | User-token auth, hand-written voice protocol, DAVE, audio mixer |
| Oct 18 | TypeScript + Material UI refactor, member list, Rich Presence, rich chat, review fixes |
| Oct 18–19 | Screen-share receive milestone: signaling, RTP video, H.264, JPEG, canvas |
| Oct 19 | PLI/REMB, keyframe recovery, performance tuning, stream UI |
| Oct 22 | Vendor oto, publish privately, RU/EN i18n, docs, CI, license |

## Day 1 — a Tauri shell that could talk to Discord (Oct 16)

The first commit created a Tauri v2 application with a Rust backend and a plain
JavaScript frontend. The initial goal was modest: log in, list guilds and
channels, read and send messages.

That same day the project gained:

- **OAuth2 login** with session persistence and real REST calls for guilds and
  channels;
- a **Gateway WebSocket**: identify, heartbeat, `READY`, `MESSAGE_CREATE`, and
  paginated message history;
- a working message composer.

Two early fixes set the tone for the rest of the project:

- user tokens must be sent as a **raw `Authorization` header**, not as a Bearer
  token (the official client does not use the OAuth scheme for user tokens);
- rustls panicked on the first WebSocket connection because both `ring` and
  `aws-lc-rs` were present in the dependency tree; the fix was to install the
  `ring` crypto provider explicitly at startup.

## Day 2 — voice was the hard part (Oct 17)

Voice started as a "skeleton": send op 4 (voice state update), receive
`VOICE_STATE_UPDATE` and `VOICE_SERVER_UPDATE`, show the status in the UI. Then
the real protocol work began:

1. Connect to the voice WebSocket, send op 2 (identify).
2. UDP IP discovery, then op 1 (select protocol).
3. Parse the voice ready payload, set up encryption and RTP.
4. Send and receive Opus, mix incoming users, play through ALSA via `cpal`.

The hand-written implementation hit every classic problem: UDP discovery
payload layout, RTP headers, sequence numbers, jitter, latency, mute/deaf state
after channel switches, and reconnects. The project eventually adopted
**[oto](https://github.com/rayan6ms/oto)** and vendored a fork because Discord
now requires **DAVE** (Discord's end-to-end encryption protocol): without it the
voice gateway closes with code **4017 "E2EE/DAVE protocol required"**.

The audio pipeline grew into:

- an **Opus encoder/decoder** with a paced sender (the `FrameSourceContract`
  expects a frame within ~2 ms);
- a **per-user mixer** so several people can speak at once;
- **RNNoise** (`nnnoiseless`) noise suppression on the mic;
- RMS-based **VAD** for the green "speaking" ring;
- mute/deafen handling that survives channel switches;
- reconnect logic that does not leave zombie voice connections.

A lot of the tuning was about the debug profile: unoptimized DSP code missed
the 2 ms frame deadline, so `Cargo.toml` sets `opt-level` overrides for
`nnnoiseless`, `fast_image_resize`, all dependencies and build scripts. That
kind of fix only shows up when you actually sit in a call.

## Day 3 — from prototype to a real UI (Oct 18)

By this point `main.rs` was **2052 lines** and the frontend was a 435-line
`app.js`. The project was working but hard to change. A deliberate refactor
followed:

- backend split into `api.rs`, `auth.rs`, `gateway.rs`, `members.rs`,
  `voice/`, `audio/`, `video.rs`, `video_render.rs`;
- frontend moved to **TypeScript + esbuild**, with typed `invoke()` wrappers and
  typed event payloads;
- a **Material Design dark theme** with teal `#00acc1` / `#4dd0e1` accents, all
  colors as CSS variables;
- member list grouped by Discord's own role/online/offline markers from the
  **op 37 member list sync**, with presence and Rich Presence activities;
- rich chat: images, GIFs (including `gifv` as video), embeds, spoilers,
  mentions with role colors, clickable links;
- fixes for duplicate history on fast channel switching, GIF flicker, and
  mention styling.

An automated review pass found real issues — a stuck Gateway "running" flag, a
zombie voice connection race, categories rendered as channels, mute state not
resetting on channel switch, config file permissions, and missing tests. Those
were fixed and covered by unit tests. The review was still AI-assisted; it is
not a substitute for a human review.

A runtime incident also shaped the dependency list: `WebKitWebProcess` aborted
(SIGABRT) when playing video because the system lacked GStreamer plugins. The
README and CI now list them.

## The screen-share milestone (Oct 18–19)

Screen-share **viewing** was the largest single feature. It went through phases:

1. **Signaling** — handle `STREAM_CREATE`, `STREAM_SERVER_UPDATE`,
   `STREAM_DELETE`, show a "LIVE" badge and a viewer screen.
2. **Transport** — a second oto connection subscribes to the stream; receive RTP
   video packets, decrypt them through DAVE, handle FU-A fragmentation and
   sequence gaps.
3. **Decode** — H.264 via the system OpenH264 (dynamically loaded, built with
   assembly on Arch). The original in-tree build without nasm decoded 1080p in
   60–120 ms per frame and the queue collapsed; the system library fixed it.
4. **Render** — downscale to 720p, encode JPEG with libjpeg-turbo, send frames
   over a Tauri channel, draw them on a `<canvas>` with `createImageBitmap`.

The classic bug: the stream showed one frame and froze. The cause was missing
**RTCP PLI** (Picture Loss Indication): the sender only sends a keyframe when
asked, and the viewer never asked. After adding PLI (and REMB handling in the
oto fork), streams recover from loss and late joins. Further tuning reduced
copies, moved resize/encode work off the hot path where possible, and added
frame dropping when a frame is still being processed.

The project deliberately stopped at **receive-only**. Broadcasting would require
screen capture (PipeWire/portal), H.264 encoding, RTP packetization and DAVE
encryption on the send side — a separate project. WebCodecs was considered as a
future decoder path but not implemented.

## Publication (Oct 22)

Before the first push:

- the Git history was rewritten to remove `src-tauri/target` (repo size dropped
  from 336 MB to a couple of megabytes);
- `vendor/oto` was committed with its licenses, so the build no longer depends
  on a nested Git checkout;
- the app got a **RU/EN i18n layer** with a live language switch, JSON
  dictionaries and a CI parity check;
- the repository gained this documentation set, MIT license, issue/PR templates,
  security policy and a GitHub Actions workflow (frontend type-check, i18n
  parity, bundle freshness, `cargo test`);
- GitHub metadata, topics and labels were set up for contributors.

The repository is private at the time of writing:
https://github.com/CHEAPFISS/discord-client

## Lessons

- **Voice is a protocol problem, not a UI problem.** Most of the difficulty was
  in state machines, timing and reconnects, not in drawing buttons.
- **AI can build a working prototype fast, but it cannot certify it.** The code
  passed live tests while still containing race conditions and protocol gaps
  that only showed up under specific conditions.
- **Observability matters.** Debug logs, a status pill, frame counters and
  timing measurements were essential to find the PLI bug and the audio deadline
  misses.
- **Dependencies are part of the product.** WebKitGTK, GStreamer, OpenH264,
  libopus and ALSA behavior directly affects whether the app runs.
- **Formatting and comments drift.** The codebase still has Russian comments and
  mixed formatting; translating and normalizing it is a good contributor task.

## What is next

- A real human review of the voice/DAVE and video paths (top priority).
- More tests around reconnects, member-list state and i18n.
- Screen-share broadcasting, if someone wants to tackle capture + encode.
- WebCodecs or hardware decoding to reduce CPU usage.
- Translating remaining code comments to English.
- Packaging (Flatpak/AppImage) once the legal and runtime dependency questions
  are settled.
