# Security policy

## Supported versions

The project is experimental and has no stable releases. Security fixes are made
on the `master` branch; only the latest commit is supported.

| Version | Supported |
| --- | --- |
| `master` (0.1.x) | ✅ best effort |
| Older commits/forks | ❌ |

## Reporting a vulnerability

**Please do not open a public issue with technical details.**

This repository is private, and GitHub's private vulnerability reporting may not
be available on the current plan. Use one of these routes:

1. If **Security** → **Advisories** → **Report a vulnerability** is available,
   use it.
2. Otherwise, contact the maintainer [@CHEAPFISS](https://github.com/CHEAPFISS)
   to arrange a private channel. You may open an issue titled
   “Security contact request” **without technical details** so the maintainer
   can respond.

Include:
   - affected commit/version;
   - a description of the issue and its impact;
   - reproduction steps or a proof of concept;
   - your distro/session (Wayland/X11) and relevant logs with secrets redacted;
   - whether you want to be credited.

If private reporting is unavailable, contact the maintainer to arrange a
private channel — **without** technical details or secrets.

This is a volunteer project. There is no bug bounty and no guaranteed response
time, but reports will be acknowledged as soon as possible.

## Scope

In scope:

- token or credential leakage (files, logs, IPC, error messages);
- code execution, path traversal, command injection (including `open_url`);
- memory-safety issues in the project's Rust code or its vendored fork;
- denial of service caused by malformed Discord data (crash, infinite loop,
  unbounded memory growth);
- CSP/webview escape that allows arbitrary script execution;
- dependency vulnerabilities that are actually reachable.

Out of scope:

- **Discord account termination or ToS enforcement.** Using a user token
  violates Discord's Terms of Service. That is a known, documented risk, not a
  vulnerability in this project. See [docs/LEGAL.md](docs/LEGAL.md).
- Issues in Discord's servers, the official client, or third-party services.
- Missing hardening that has no demonstrated impact.
- Reports that require an attacker to already control the user's account or
  machine.
- Spam/rate-limit bypass techniques.

## Threat model (short)

- The token is a **full-access credential**. Anyone who obtains it can control
  the account. It is stored in `~/.config/discord-client/` with `0600`
  permissions and sent only to Discord endpoints over TLS.
- The app renders remote content (messages, embeds, images) in a WebKit webview
  with a restrictive CSP. Treat a webview escape as high severity.
- The backend is not sandboxed. A malicious local process/user with access to the
  same account can read the token file.
- Voice and video data comes from Discord/other users; parsers must treat it as
  untrusted. The H.264 decoder is a system library and outside this project's
  memory-safety guarantees.
- Logs can contain identifiers and, at `RUST_LOG=trace`, authorization headers.
  Do not use trace logging with a real account, and redact logs before sharing.

## Safe harbour

Good-faith security research is welcome. Please:

- test only against your own account and data;
- do not access, modify or exfiltrate other users' data;
- do not run denial-of-service tests against Discord's infrastructure;
- give the maintainer reasonable time to fix the issue before disclosure.

## Dependency and supply-chain notes

- `vendor/oto` is a fork committed to this repository; review changes there like
  any other code.
- System libraries (OpenH264, libjpeg-turbo, GStreamer, WebKitGTK, GTK, ALSA,
  Opus) are provided by the distribution and patched by it.
- CI runs `cargo test` and frontend checks, but there is no automated dependency
  audit or SAST yet. Adding one is a welcome contribution.

## Secrets checklist for contributors

Before posting logs or screenshots, remove:

- tokens and `Authorization` headers;
- `config.json` contents (OAuth client secret);
- session files from `~/.config/discord-client/`;
- private message content, user ids and server ids you do not want public.
