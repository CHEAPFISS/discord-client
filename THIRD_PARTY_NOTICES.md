# Third-party notices

This project is MIT-licensed (see [LICENSE](LICENSE)), but it depends on and
vendors third-party software with its own licenses. This file summarizes the
main components; the full texts are shipped with the components or available
upstream.

## Vendored code

### oto (`vendor/oto`)

- Upstream: https://github.com/rayan6ms/oto
- License: **MIT OR Apache-2.0**
- Copyright: 2026 Oto contributors
- Full texts: [`vendor/oto/LICENSE-MIT`](vendor/oto/LICENSE-MIT),
  [`vendor/oto/LICENSE-APACHE`](vendor/oto/LICENSE-APACHE)
- The fork adds receive-side video, RTCP PLI/REMB and DAVE diagnostics.
- Third-party crates vendored inside oto are documented in
  [`vendor/oto/THIRD_PARTY_NOTICES.md`](vendor/oto/THIRD_PARTY_NOTICES.md).

### davey (inside `vendor/oto`)

- Upstream: https://github.com/Snazzah/davey
- License: **MIT**
- Copyright: 2025-present Snazzah
- License text: `vendor/oto/vendor/davey/LICENSE` (relative to the oto crate).

### openmls_rust_crypto (inside `vendor/oto`)

- Upstream: https://github.com/openmls/openmls
- License: **MIT**
- Copyright: 2020 OpenMLS Authors
- License text: `vendor/oto/vendor/openmls_rust_crypto/LICENSE`.

## Rust dependencies

The exact dependency graph is pinned by `src-tauri/Cargo.lock`. The main
first-party and ecosystem components include:

| Component | License |
| --- | --- |
| Tauri | MIT OR Apache-2.0 |
| tokio | MIT |
| reqwest | MIT OR Apache-2.0 |
| tokio-tungstenite | MIT |
| serde / serde_json | MIT OR Apache-2.0 |
| rustls | Apache-2.0 OR ISC OR MIT |
| cpal | Apache-2.0 |
| opus (Rust bindings) | MIT OR Apache-2.0 |
| nnnoiseless | BSD-3-Clause |
| fast_image_resize | MIT OR Apache-2.0 |
| turbojpeg (Rust bindings) | Unlicense OR MIT |
| openh264 (Rust bindings) | BSD-2-Clause |

Transitive dependencies keep their upstream licenses; use `cargo metadata` or
`cargo deny` to inspect the full graph.

## System libraries (dynamically linked)

These are **not** distributed with this repository. They are provided by the
Linux distribution and must be installed by the user:

| Library | Typical package | License |
| --- | --- | --- |
| WebKitGTK 4.1 | `webkit2gtk-4.1` | LGPL-2.1-or-later / BSD-style |
| GTK 3 | `gtk3` | LGPL-2.1-or-later |
| libsoup 3 | `libsoup3` | LGPL-2.1-or-later |
| ALSA | `alsa-lib` | LGPL-2.1-or-later |
| Opus | `opus` | BSD-3-Clause |
| libjpeg-turbo | `libjpeg-turbo` | BSD-3-Clause, Zlib, IJG |
| OpenH264 | `openh264` | BSD-2-Clause |
| GStreamer + plugins | `gst-plugins-*` | LGPL-2.0-or-later (some plugins differently licensed) |

The screen-share viewer loads `libopenh264.so` at runtime. A missing library
disables decoding; it does not change the project's license.

## Assets

The project does not bundle Discord trademarks, icons, sounds or fonts. Avatar
and attachment URLs point to Discord's CDN and are loaded at runtime; they remain
the property of their respective owners.

## Reporting a licensing issue

If you believe a component is missing an attribution or is incompatible with
MIT, please open an issue (or a security advisory if it is sensitive). We will
fix the notice or replace the component.
