# 0xMark

A tiny GNOME app for marking up images on the clipboard.

Copy an image, open 0xMark, draw a box or an arrow, press `Enter`. The annotated image is back on
the clipboard, ready to paste into chat, email, an issue tracker or a document.

![0xMark with an image loaded, a blue rectangle drawn on it and the floating toolbar at the bottom](screenshot.png)

## Features

- Loads the clipboard image automatically when the window opens.
- Rectangles, arrows, freehand pen and text labels.
- Seven colours and three stroke sizes.
- Undo and redo.
- Exports at the original resolution, whatever the zoom level in the window.
- Follows the GNOME light or dark style.

## Requirements

- Linux with a **GNOME Wayland** session. X11 sessions are not supported.
- GTK 4.14 or newer and libadwaita 1.5 or newer. Ubuntu 24.04, Pop!\_OS 24.04 and Fedora 40 or
  newer have these.
- `wl-clipboard`, which provides the `wl-copy` command.
- A Rust toolchain to build it. Install one from <https://rustup.rs> if you don't have it.

### Ubuntu, Pop!\_OS, Debian

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev wl-clipboard
```

### Fedora

```sh
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel wl-clipboard
```

### Arch

```sh
sudo pacman -S base-devel gtk4 libadwaita wl-clipboard
```

## Install

```sh
git clone https://github.com/abidibo/0xMark.git
cd 0xMark
cargo build --release
install -Dm755 target/release/0xmark ~/.local/bin/0xmark
install -Dm644 data/icons/dev.abidibo.OxMark.svg \
  ~/.local/share/icons/hicolor/scalable/apps/dev.abidibo.OxMark.svg
touch ~/.local/share/icons/hicolor
mkdir -p ~/.local/share/applications
sed "s|^Exec=0xmark$|Exec=$HOME/.local/bin/0xmark|" data/dev.abidibo.OxMark.desktop \
  > ~/.local/share/applications/dev.abidibo.OxMark.desktop
```

The `sed` line writes the full path of the binary into the desktop entry. Some GNOME sessions do
not have `~/.local/bin` on their `PATH`, and GNOME silently hides desktop entries whose program it
cannot find. The `touch` line tells running apps, such as GNOME Shell or a launcher, to
reload icons.

0xMark now shows up in the GNOME app grid. You can also run `0xmark` from a terminal.

### Keyboard shortcut (recommended)

The app is meant to be opened instantly, so bind it to a key:

1. Open **Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts**.
2. Add a shortcut with name `0xMark`. For the command, use the full path, for example
   `/home/you/.local/bin/0xmark`. Run `echo ~/.local/bin/0xmark` to get yours.
3. Pick a key combination, for example `Super+Shift+A`.

## Usage

1. Copy an image. For a screenshot, press `Print`, select an area and confirm. GNOME puts the
   screenshot on the clipboard. Copying an image from a browser or an image viewer works too.
2. Open 0xMark with your shortcut. The image appears right away.
3. Pick a tool, a colour and a size in the toolbar at the bottom, then drag on the image.
   For text, click where the label should go, type it and press `Enter`.
4. Press `Enter` or click **Copy**. 0xMark closes and the annotated PNG is on the clipboard.
5. Paste it wherever you need it.

Press `Esc` to close without touching the clipboard.

### Shortcuts

| Key                | Action                            |
|--------------------|-----------------------------------|
| `R`                | Rectangle                         |
| `A`                | Arrow                             |
| `P`                | Freehand pen                      |
| `T`                | Text, then click to place         |
| `1` `2` `3`        | Thin, medium or thick stroke      |
| `Ctrl+Z`           | Undo                              |
| `Ctrl+Shift+Z`     | Redo, `Ctrl+Y` also works         |
| `Ctrl+V`           | Load the clipboard image again    |
| `Enter` / `Ctrl+C` | Copy the result and close         |
| `Esc`              | Close without copying             |

## Troubleshooting

**0xMark is missing from the app grid or your launcher.** The desktop entry probably still says
`Exec=0xmark` and your session cannot find it. Repeat the `sed` step from the install section.

**"No image in clipboard".** The clipboard holds text or nothing. Copy an image and press
`Ctrl+V` in the window.

**"cannot run wl-copy".** Install `wl-clipboard` using the commands above.

**The pasted image is empty or the paste fails.** You are probably in an X11 session. Check with
`echo $XDG_SESSION_TYPE`. It must print `wayland`.

## Uninstall

```sh
rm ~/.local/bin/0xmark \
   ~/.local/share/applications/dev.abidibo.OxMark.desktop \
   ~/.local/share/icons/hicolor/scalable/apps/dev.abidibo.OxMark.svg
```

## How it works

0xMark is a single Rust file using GTK 4 and libadwaita. It reads the clipboard through GTK and
draws annotations with Cairo. Annotations are kept in image coordinates, so the preview can be
scaled freely while the export stays at full resolution.

On Wayland, clipboard content belongs to the app that copied it and disappears when that app
quits. 0xMark therefore hands the final PNG to `wl-copy`, which keeps serving it in the background
after 0xMark has closed.
