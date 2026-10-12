# 0xMark

Tiny GNOME tool: copy an image, launch 0xMark, annotate, press Enter, paste anywhere.

## Flow

1. Copy an image to the clipboard (screenshot, browser, ...).
2. Launch `0xmark`. The image loads automatically.
3. Draw rectangles, arrows, freehand strokes or text labels.
4. Press `Enter` (or `Ctrl+C`). The annotated PNG replaces the clipboard content and the app quits.

## Shortcuts

| Key          | Action                        |
|--------------|-------------------------------|
| `R`          | Rectangle tool                |
| `A`          | Arrow tool                    |
| `P`          | Freehand pen                  |
| `T`          | Text label (click to place)   |
| `Ctrl+Z`     | Undo last annotation          |
| `Ctrl+V`     | Reload image from clipboard   |
| `Enter`/`Ctrl+C` | Copy result and quit      |
| `Esc`        | Quit without copying          |

## Build and install

Requires Rust, GTK 4.14+, libadwaita 1.5+ and `wl-copy` (package `wl-clipboard`).

```sh
cargo build --release
install -Dm755 target/release/0xmark ~/.local/bin/0xmark
install -Dm644 data/0xmark.desktop ~/.local/share/applications/0xmark.desktop
```

Then bind a key in GNOME Settings > Keyboard > Custom Shortcuts with command `0xmark`.

## Why wl-copy

On Wayland the clipboard content lives inside the process that copied it. `wl-copy` forks into
the background and keeps serving the image after 0xMark has exited.
