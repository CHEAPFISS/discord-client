# egui-probe

A small native UI probe used to estimate the memory cost of replacing the
WebKit-based Tauri frontend with an `egui` frontend.

It is **not** a Discord client: there is no network, voice, or media. It only
draws a static chat mock (server rail, channel list, messages, member list) and
shows the process RSS/PSS in the top bar.

## Why

The Tauri client runs a separate WebKit web process, which dominates memory:

| Scenario | Total PSS |
| --- | ---: |
| Tauri client, fresh start | ~280 MB |
| Tauri client, after a short screen-share | ~440 MB |
| This egui probe (static chat mock, debug build) | ~60 MB |

The probe uses `eframe` with the `glow` (OpenGL) backend; `wgpu` is not needed
for a 2D chat and would add memory.

## Run

```sh
cargo run
```

The top bar updates RSS and PSS once per second. The values come from
`/proc/self/status` and `/proc/self/smaps_rollup`.

## Notes

- This is a layout/memory probe, not a migration plan. Reusing the existing
  Rust backend (gateway, voice, stream decode) would require separating it from
  Tauri's `AppHandle`/event plumbing.
- The mock uses only text; real avatars, embeds, and GIFs will add memory.
