# Contributing

Thanks for taking the time to look at this project. It is an experimental,
AI-generated Discord client, and it needs human engineers far more than it needs
more features. **Code review is the most valuable contribution right now.**

Before you start, please read:

- [README.md](README.md) — what the project is and is not;
- [docs/AI_DISCLOSURE.md](docs/AI_DISCLOSURE.md) — how the code was made and
  where the risk is concentrated;
- [docs/LEGAL.md](docs/LEGAL.md) — Discord ToS, user tokens, license;
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — expected behaviour.

## Ways to contribute

- **Review code.** Pick an area from the risk table, read it, open issues for
  concrete findings. Partial reviews are welcome.
- **Reproduce and report bugs.** Include the commit, distro, desktop session
  (Wayland/X11), steps, and redacted logs.
- **Add tests.** Parsers, state transitions, reconnect logic, i18n parity.
- **Improve documentation.** Especially translating the remaining Russian code
  comments to English, one file per PR.
- **Add a UI language.** See [docs/I18N.md](docs/I18N.md).
- **Tackle a known limitation.** Screen-share broadcasting, WebCodecs, packaging.

Please open an issue before a large feature or refactor so we can agree on the
approach. Small fixes can go straight to a PR.

## Development setup

### Requirements

- Linux (Wayland or X11). Windows/macOS are not tested.
- Rust (stable) and Cargo.
- Node.js 22+ and npm.
- System libraries: WebKitGTK 4.1, GTK3, libsoup3, ALSA, Opus, libjpeg-turbo,
  OpenH264, GStreamer plugins.

Arch Linux:

```bash
sudo pacman -S --needed base-devel rust nodejs npm pkgconf \
  webkit2gtk-4.1 gtk3 libsoup3 \
  alsa-lib opus libjpeg-turbo openh264 \
  gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav
```

Debian/Ubuntu:

```bash
sudo apt-get install -y \
  build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev librsvg2-dev libxdo-dev \
  libasound2-dev libopus-dev libturbojpeg0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav
```

### First build

```bash
git clone https://github.com/CHEAPFISS/discord-client.git
cd discord-client
npm ci
npm run check          # tsc, i18n parity, bundle
cd src-tauri
cargo build
./target/debug/discord-client
```

The app expects Discord credentials. For token login, paste a user token into the
sign-in screen. For OAuth, create `~/.config/discord-client/config.json`:

```json
{
  "client_id": "YOUR_CLIENT_ID",
  "client_secret": "YOUR_CLIENT_SECRET"
}
```

Never commit this file or paste its contents into an issue. The token is a
full-access credential.

### Development loop

- TypeScript: `npm run fe:watch` (rebuilds `src/app.js`).
- Rust: `cargo build` in `src-tauri` (debug profile is optimized; see
  `Cargo.toml`).
- Restart the app after a backend change.
- `RUST_LOG=debug` gives useful logs. **Do not use `RUST_LOG=trace` with a real
  account** — it can log authorization headers.

## Checks

Run before opening a PR:

```bash
npm run check                 # fe:check + i18n:check + fe:build
cd src-tauri && cargo test
```

`npm run check` regenerates `src/app.js`; commit the updated bundle. CI verifies
that the committed bundle matches the TypeScript sources.

On Ubuntu 24.04, run `cargo test --features bundled-turbojpeg` (needs `cmake` and
`nasm`); Arch's libjpeg-turbo 3.x works without the feature. CI uses the bundled
path because Ubuntu ships 2.x.

Optional but recommended:

```bash
cd src-tauri
cargo fmt                     # large diff; do it in a separate commit
cargo clippy                  # useful warnings, not currently enforced
```

The repository does not enforce `rustfmt`/`clippy` yet, because the existing code
would produce a large formatting diff. If you want to normalize a module, do it
in a dedicated PR with no behaviour changes.

## Project layout

```
src/                  TypeScript frontend, CSS, HTML, locales
  ui/                 screens and widgets
  locales/            en.json / ru.json
src-tauri/src/        Rust backend
  voice/              voice connection, streams
  audio/              mic capture, playback mixer
vendor/oto/           vendored oto fork (do not reformat casually)
scripts/              build/check helpers
docs/                 architecture, history, legal, i18n, AI disclosure
```

## Coding guidelines

### Rust

- Match the existing module boundaries; put Tauri commands in the module that
  owns the behaviour.
- Avoid new `unwrap()`/`expect()` on paths reachable from user input, network or
  audio callbacks. If an invariant makes it safe, add a comment explaining why.
- Keep async tasks cancellable and make sure every `spawn` has an owner that
  stops it. Document lock ordering when taking more than one lock.
- Use `log::debug!`/`info!`/`warn!` for diagnostics; do not log tokens or
  message contents.
- Comments may be Russian or English; new comments in English are preferred, but
  do not mix translation with logic changes.

### TypeScript

- `strict` mode is on; avoid `any`.
- No frontend frameworks. Plain DOM + CSS.
- All user-facing strings go through `t()` or `data-i18n*`. No hardcoded UI copy.
- Keep `api.ts` and `events.ts` in sync with Rust command signatures and emitted
  payloads. There is no codegen; this is a manual contract.
- Prefer small functions in `ui/*.ts`; state lives in `state.ts`.

### CSS

- Use the existing tokens in `main.css` (`--surface-*`, `--text-*`, `--accent*`,
  radii, shadows). Do not hardcode colors that already have a token.
- The visual language is Material-inspired dark with a teal accent
  (`#00acc1` / `#4dd0e1`).
- Test both narrow and wide windows, and both languages (German-style long words
  are a good stress test).

## Internationalization

Any new UI string must be added to **both** `src/locales/en.json` and
`src/locales/ru.json`. Use placeholders rather than concatenation. Run:

```bash
npm run i18n:check
```

See [docs/I18N.md](docs/I18N.md) for the full workflow, including adding a new
language.

## Commits and pull requests

- Keep PRs focused. One logical change per PR; separate refactors/formatting
  from behaviour.
- Write commit messages in **English** (the existing history is mostly Russian;
  new commits should be English so the project is approachable).
- Describe **how you tested**: distro/session, voice or stream scenario, language
  switched, etc. “Compiles” is not enough for voice/video changes.
- Link the issue if one exists.
- Fill in the PR template, including the AI-assistance question.
- Do not include generated binaries, `target/`, session files, tokens or logs.

## Review process

- CI must pass (frontend check, i18n parity, bundle freshness, `cargo test`).
- At least one human review is expected for non-trivial changes. The maintainer
  may not be a domain expert in every area, so reviews from Rust/WebRTC/audio
  engineers are especially welcome.
- Be kind and specific. “This looks wrong” is less useful than “this lock is held
  across an await, which can deadlock with …”.
- The project is experimental; a review that finds problems is a success, not a
  rejection.

## AI-assisted contributions

AI tools are allowed, with conditions:

- Disclose substantial AI assistance in the PR description.
- Understand the code you submit and test it. You are responsible for it.
- Do not paste tokens, session files or private logs into prompts.
- AI-generated changes still need human review and must follow the i18n/type
  contracts above.

## Security and privacy

- Never include tokens, `config.json`, session files or unredacted logs in an
  issue/PR. Redact ids if they are sensitive.
- Security issues: follow [SECURITY.md](SECURITY.md), not a public issue.
- Using a user token violates Discord's ToS; see [docs/LEGAL.md](docs/LEGAL.md).
  Do not use the project for spam, scraping or public services.

## License

By contributing, you agree that your contributions are licensed under the
project's MIT license (see [LICENSE](LICENSE)). Only submit code you have the
right to license this way. Vendored dependencies keep their own licenses; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Good first issues

- Add tests for `gateway.rs` voice-state transitions.
- Translate one backend file's comments to English.
- Add a missing i18n key or fix an awkward translation.
- Improve error messages when the token is invalid/expired.
- Document a runtime dependency (GStreamer/OpenH264) in the README.
- Review `open_url` and CSP for URL-handling edge cases.
- Add a language to `src/locales/`.

If you are unsure where to start, open a discussion/issue and ask.
