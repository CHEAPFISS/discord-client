# AI disclosure and review guide

## Summary

Almost all implementation code in this repository was written by **AI coding
agents**. The human owner defined the requirements, made design decisions, ran
the application, tested real voice calls and screen shares, reported bugs, and
pushed back when the result was wrong. The AI wrote the code and most of the
prose; the human verified behaviour, not every line of code.

This was an experiment in "vibe coding" a non-trivial native application:
Discord Gateway, voice with DAVE/E2EE, audio mixing, H.264 screen-share
receiving. It works in live testing, but **it has not received a professional
code review**. That is the main thing this repository needs from contributors.

## What was AI-generated

- The Rust backend: REST client, OAuth/token auth, Gateway WebSocket loop,
  voice connection and audio pipeline, member-list sync, video decode/encode.
- The TypeScript frontend, CSS, esbuild setup and most of the HTML.
- The `oto` fork changes that add receive-side video and diagnostics.
- Tests, documentation, issue/PR templates, CI workflow and this disclosure.

## What the human did

- Chose the product scope and UI direction (Material dark, teal accent).
- Ran every milestone in a real Discord account and a real voice channel.
- Tested with another person: voice quality, mute/deafen, reconnects, screen
  shares, keyframe recovery.
- Reported concrete bugs (duplicate history on fast channel switches, GIF
  flicker, member grouping, stream freezing) and verified fixes.
- Decided to publish, chose the license, and requested this disclosure.
- Reviewed high-level behaviour and architecture, not line-level correctness.

## Why it matters

AI-generated code tends to look plausible while hiding subtle problems:

- error handling that compiles but swallows failures;
- `unwrap`/`expect` on paths that can fail at runtime;
- lock ordering, race conditions and task leaks in async code;
- protocol details that happen to work against one server version;
- cryptography code that is easy to misuse;
- frontend state duplicated between Rust and TypeScript;
- comments (mostly Russian) that no longer match the code.

Live testing covered the happy paths on one Linux machine. It did **not** cover
adversarial inputs, long-running sessions, low-memory conditions, unusual
network topologies or protocol changes.

## Highest-risk areas

Review these first. Paths are relative to the repository root.

| Priority | Area | Why it is risky | Where |
| --- | --- | --- | --- |
| 1 | DAVE / E2EE voice | Wrong crypto state handling can leak or break audio; protocol is security-sensitive | `vendor/oto/` (DAVE setup), `src-tauri/src/voice/connect.rs` |
| 2 | RTP/RTCP video receive | Packet reassembly, FU-A depacketization, PLI/REMB, sequence gaps | `vendor/oto/` video path, `src-tauri/src/video.rs` |
| 3 | Async lifecycle and reconnects | Gateway/voice task leaks, stale events, lock ordering, zombie connections | `gateway.rs`, `voice/mod.rs`, `lib.rs` |
| 4 | Audio mixer and mic pipeline | Underruns, frame overruns, per-user buffering, mute/deaf state, VAD | `src-tauri/src/audio/`, `voice/stream.rs` |
| 5 | Video hot path | 1080p RGBA copies, JPEG encode on the async runtime, frame dropping, canvas sizing | `video.rs`, `video_render.rs`, `ui/stream.ts` |
| 6 | Token/session handling | Tokens in files, logs, IPC; OAuth secret storage; no leak on logout | `auth.rs`, `lib.rs`, `main.rs` |
| 7 | Type parity Rust ↔ TypeScript | Hand-maintained serde/TS types can drift silently | `api.rs` + `api.ts`, `events.rs` + `events.ts` |
| 8 | Error handling | Many errors are `String`; some paths log and continue | whole backend |
| 9 | i18n coverage | Missing keys fall back to English or the raw key | `src/locales/`, `i18n.ts` |

There are no `unsafe` blocks in the project's own Rust code, but the vendored
`oto`/`davey` code and C libraries (OpenH264, turbojpeg, Opus, ALSA) are outside
that guarantee.

## Suggested review plan

1. **Build and run it.** Follow [../CONTRIBUTING.md](../CONTRIBUTING.md). Run
   `npm run check` and `cargo test`.
2. **Read the architecture.** [ARCHITECTURE.md](ARCHITECTURE.md) maps modules and
   data flows; use it to navigate instead of reading top-to-bottom.
3. **Start with authentication and state.** Trace a token from login to REST and
   Gateway, then logout. Check file permissions, log leakage and error paths.
4. **Review the voice path.** Follow `join_voice` → Discord voice state →
   `connect_voice` → oto → audio tasks. Ask: what stops each task? What happens
   on reconnect, channel switch, identity change?
5. **Review the video path.** Follow `STREAM_CREATE` → watch → RTP → DAVE decrypt
   → H.264 → JPEG → IPC → canvas. Ask: who drops frames, who frees buffers, what
   happens on a lost keyframe or a stalled stream?
6. **Check type parity.** Compare every `#[tauri::command]` signature and event
   payload with `api.ts`/`events.ts`.
7. **Test failure modes.** Invalid token, offline network, gateway disconnect,
   voice reconnect, stream end, window resize, language switch mid-call.
8. **Look for security issues.** Token exposure, URL handling (`open_url`),
   CSP, IPC input validation, dependency advisories.

## What "reviewed" means here

A review is useful even if it is partial. If you review an area, say which
commit and which files, and open issues for what you find. Please do not label
the project "audited" or "production-ready" without a full review; the README
will keep the experimental label until that happens.

## AI-assisted contributions

Contributors may use AI tools. The rules are:

- **Disclose** substantial AI assistance in the PR description.
- **Understand and test** what you submit; "the model wrote it" is not a defense.
- **Keep the i18n and type contracts** intact (see CONTRIBUTING.md).
- **Do not paste secrets** (tokens, client secrets, session files, logs) into
  prompts or issues.
- A human reviewer must still review the change.

## No warranty

This software is provided "as is", without warranty of any kind. See
[../LICENSE](../LICENSE). The disclosure above is not a guarantee that the code
is fit for any purpose; it is an honest description of how it was made.
