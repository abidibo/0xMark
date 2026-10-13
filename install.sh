#!/usr/bin/env bash
# Install 0xMark into ~/.local: system packages, Rust (via rustup) if needed,
# a release build, the desktop entry, the icon and a GNOME keyboard shortcut.
#
#   curl -fsSL https://raw.githubusercontent.com/abidibo/0xMark/main/install.sh | bash
#   ./install.sh                 from a checkout, builds that checkout
#   ./install.sh --uninstall     removes everything the installer added
#
# Environment:
#   OXMARK_HOTKEY='<Super><Shift>a'   shortcut to bind, empty to skip it
#   OXMARK_REF=main                   branch or tag to fetch when piped from curl
set -euo pipefail

# Empty when piped from curl. Read it here: inside a function bash reports "main".
SELF="${BASH_SOURCE[0]:-}"
REPO="abidibo/0xMark"
REF="${OXMARK_REF:-main}"
HOTKEY="${OXMARK_HOTKEY-<Super><Shift>a}"
APP_ID="dev.abidibo.OxMark"
MIN_RUST="1.85" # edition 2024

BIN="$HOME/.local/bin/0xmark"
DESKTOP="$HOME/.local/share/applications/$APP_ID.desktop"
ICON_DIR="$HOME/.local/share/icons/hicolor"
ICON="$ICON_DIR/scalable/apps/$APP_ID.svg"
SCHEMA="org.gnome.settings-daemon.plugins.media-keys"
KEY_PATH="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/0xmark/"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

as_root() {
  if [[ $EUID -eq 0 ]]; then "$@"; else sudo "$@"; fi
}

install_packages() {
  local missing=()
  if have apt-get; then
    for p in build-essential pkg-config libgtk-4-dev libadwaita-1-dev wl-clipboard curl; do
      dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q "ok installed" || missing+=("$p")
    done
    if ((${#missing[@]})); then
      say "Installing ${missing[*]}"
      as_root apt-get update
      as_root apt-get install -y "${missing[@]}"
    fi
  elif have dnf; then
    for p in gcc pkgconf-pkg-config gtk4-devel libadwaita-devel wl-clipboard curl; do
      rpm -q "$p" >/dev/null 2>&1 || missing+=("$p")
    done
    if ((${#missing[@]})); then
      say "Installing ${missing[*]}"
      as_root dnf install -y "${missing[@]}"
    fi
  elif have pacman; then
    for p in base-devel gtk4 libadwaita wl-clipboard curl; do
      pacman -Qi "$p" >/dev/null 2>&1 || missing+=("$p")
    done
    if ((${#missing[@]})); then
      say "Installing ${missing[*]}"
      as_root pacman -S --needed --noconfirm "${missing[@]}"
    fi
  else
    warn "Unknown package manager. Install GTK 4.14+, libadwaita 1.5+ (with headers) and wl-clipboard yourself."
  fi

  pkg-config --atleast-version=4.14 gtk4 ||
    die "GTK 4.14 or newer is required, found $(pkg-config --modversion gtk4 2>/dev/null || echo none). Ubuntu 24.04 or newer has it."
  pkg-config --atleast-version=1.5 libadwaita-1 ||
    die "libadwaita 1.5 or newer is required, found $(pkg-config --modversion libadwaita-1 2>/dev/null || echo none)."
}

rust_is_recent() {
  local v
  v="$(rustc --version 2>/dev/null | awk '{print $2}')" || return 1
  [[ -n $v ]] && [[ "$(printf '%s\n%s\n' "$MIN_RUST" "$v" | sort -V | head -1)" == "$MIN_RUST" ]]
}

ensure_rust() {
  # rustup puts cargo in ~/.cargo/bin, which a fresh shell may not have on PATH yet.
  [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
  if have cargo && rust_is_recent; then
    return
  fi
  if have rustup; then
    # Build with a recent stable without changing the user's default toolchain.
    say "Installing the stable Rust toolchain"
    rustup toolchain install stable --profile minimal
    export RUSTUP_TOOLCHAIN=stable
  else
    # Distribution Rust packages are usually too old for edition 2024, so use rustup.
    say "Installing Rust with rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
      RUSTUP_INIT_SKIP_PATH_CHECK=yes sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
  fi
  rust_is_recent || die "Rust $MIN_RUST or newer is required, found $(rustc --version 2>/dev/null || echo none)."
}

hotkey_taken() {
  gsettings list-recursively | grep -Fqi "'$HOTKEY'" && return 0
  # list-recursively skips custom shortcuts, which live at relocatable paths.
  local path
  for path in $(tr -d "[],'" <<<"$1"); do
    [[ $path == /* ]] || continue
    gsettings get "$SCHEMA.custom-keybinding:$path" binding | grep -Fqix "'$HOTKEY'" && return 0
  done
  return 1
}

bind_hotkey() {
  [[ -n $HOTKEY ]] || return 0
  if ! have gsettings || ! gsettings list-schemas | grep -qx "$SCHEMA"; then
    warn "GNOME settings not found, skipping the keyboard shortcut."
    return 0
  fi

  local current new
  current="$(gsettings get "$SCHEMA" custom-keybindings)"
  if [[ $current != *"$KEY_PATH"* ]]; then
    # Warn rather than steal a combination that is already taken.
    if hotkey_taken "$current"; then
      warn "$HOTKEY is already in use, skipping the shortcut. Pick another with OXMARK_HOTKEY='<Super><Shift>m'."
      return 0
    fi
    if [[ $current == "@as []" || $current == "[]" ]]; then
      new="['$KEY_PATH']"
    else
      new="${current%]}, '$KEY_PATH']"
    fi
    gsettings set "$SCHEMA" custom-keybindings "$new"
  fi
  gsettings set "$SCHEMA.custom-keybinding:$KEY_PATH" name "0xMark"
  gsettings set "$SCHEMA.custom-keybinding:$KEY_PATH" command "$BIN"
  gsettings set "$SCHEMA.custom-keybinding:$KEY_PATH" binding "$HOTKEY"
  BOUND=1
}

unbind_hotkey() {
  have gsettings && gsettings list-schemas | grep -qx "$SCHEMA" || return 0
  local current new
  current="$(gsettings get "$SCHEMA" custom-keybindings)"
  [[ $current == *"$KEY_PATH"* ]] || return 0
  new="$(sed -e "s|, '$KEY_PATH'||; s|'$KEY_PATH', ||; s|'$KEY_PATH'||" <<<"$current")"
  [[ $new == "[]" ]] && new="@as []"
  gsettings set "$SCHEMA" custom-keybindings "$new"
  gsettings reset-recursively "$SCHEMA.custom-keybinding:$KEY_PATH" 2>/dev/null || true
}

uninstall() {
  say "Removing 0xMark"
  rm -f "$BIN" "$DESKTOP" "$ICON"
  touch "$ICON_DIR" 2>/dev/null || true
  unbind_hotkey
  say "Done. System packages and Rust were left in place."
}

main() {
  if [[ ${1:-} == "--uninstall" ]]; then
    uninstall
    return
  fi

  [[ ${XDG_SESSION_TYPE:-} == "wayland" ]] ||
    warn "This is not a Wayland session. 0xMark needs GNOME on Wayland to copy images back."

  install_packages
  ensure_rust

  # Build the checkout this script lives in, or fetch the sources when piped from curl.
  local src="" dir tmp
  if [[ -n $SELF && -f $SELF ]]; then
    dir="$(cd "$(dirname "$SELF")" && pwd)"
    grep -qs '^name = "oxmark"' "$dir/Cargo.toml" && src="$dir"
  fi
  if [[ -z $src ]]; then
    tmp="$(mktemp -d)"
    trap "rm -rf '$tmp'" EXIT
    say "Downloading 0xMark ($REF)"
    curl -fsSL "https://github.com/$REPO/archive/$REF.tar.gz" | tar -xz -C "$tmp" --strip-components=1
    src="$tmp"
  fi

  say "Building (this takes a minute or two the first time)"
  cargo build --release --locked --manifest-path "$src/Cargo.toml"

  say "Installing into ~/.local"
  install -Dm755 "$src/target/release/0xmark" "$BIN"
  install -Dm644 "$src/data/icons/$APP_ID.svg" "$ICON"
  mkdir -p "$(dirname "$DESKTOP")"
  # Absolute path: GNOME hides entries whose program is not on its PATH.
  sed "s|^Exec=0xmark$|Exec=$BIN|" "$src/data/$APP_ID.desktop" >"$DESKTOP"
  touch "$ICON_DIR"
  have update-desktop-database && update-desktop-database -q "$(dirname "$DESKTOP")" 2>/dev/null || true

  BOUND=0
  bind_hotkey

  say "0xMark is installed. Find it in the app grid."
  if ((BOUND)); then
    say "Shortcut: $HOTKEY. Change it in Settings > Keyboard > Custom Shortcuts."
  fi
  [[ ":$PATH:" == *":$HOME/.local/bin:"* ]] ||
    warn "~/.local/bin is not on your PATH, so '0xmark' won't run from a terminal. The app grid and the shortcut are not affected."
}

# Everything runs from main, so a download cut short by the network never runs half a script.
main "$@"
