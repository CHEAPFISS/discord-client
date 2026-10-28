# ui-tree-probe

A mock prototype of a node-tree window constructor for the native (egui)
Discord client. There is **no backend**: servers, channels, members, and
activity are placeholders. The goal is to validate the layout model and its
memory cost before wiring real data.

## Model

The canvas is the root node. Every node has a local position and scale
relative to its parent, and children inherit both:

```
world(child) = world(parent) * local(child)
```

Nodes are a tree, like Godot scenes: a window can contain a list, a button,
another container, etc. Children are positioned inside the parent's local
space; there is no rotation or clipping in this prototype.

Each node is one Rust type implementing the `Node` trait:

- `draw()` renders itself in its local world;
- `props()` / `from_spec()` persist and restore its state;
- `children` live in the shared `BaseNode`.

Built-in mock nodes: `window`, `label`, `button`, `list`, `servers`, `voice`,
`members`, `debug`.

## Controls

- Drag a window by its title bar to move it.
- Title bar buttons: `−` / `+` change the window's own scale (children scale
  with it), `×` closes the window.
- `Ctrl` + mouse wheel zooms the canvas toward the cursor.
- Middle mouse button pans the canvas.
- The `Debug` window shows PSS/RSS, zoom, node count, the last button action,
  and has buttons to add a window, save, reset, and zoom.

## Layout persistence

The tree is saved to `~/.config/discord-client/layout.json`
(`$XDG_CONFIG_HOME` is respected). The file is versioned:

```json
{
  "version": 1,
  "nodes": [
    {
      "kind": "window",
      "id": "win-servers",
      "pos": [20.0, 20.0],
      "scale": 1.0,
      "size": [190.0, 350.0],
      "props": { "title": "Сервера" },
      "children": []
    }
  ]
}
```

Unknown node kinds are skipped with a warning, so removing a node type does
not break old layouts. Layouts with an unsupported version are ignored and
the default layout is used.

## Run

```sh
cargo run --release
```

Debug builds are noticeably heavier; memory numbers below are for the
release build.

## Memory

Measured on Linux with `smaps_rollup` (PSS), same machine as the Tauri client:

| Scenario | PSS | RSS |
| --- | ---: | ---: |
| Tauri/WebKit client, fresh start | ~280 MB | — |
| Tauri/WebKit client, after a short screen share | ~440 MB | — |
| This prototype, release, 6 windows / 13 nodes | ~68 MB | ~117 MB |
| This prototype, debug | ~82–93 MB | ~130–146 MB |

The prototype stays under the 70 MB target in release. It is still a mock:
real avatars, images, and video will add memory.

## Limitations

- No backend and no real data.
- No clipping or rotation; children are expected to stay inside their parent.
- Windows are resized only by changing their scale, not by dragging edges.
- The layout format is a prototype and may change.
