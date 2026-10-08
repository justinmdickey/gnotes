#!/bin/sh
# Builds the GTK client and installs it for you: the binary, and the launcher and icon so it shows in
# GNOME's app grid. Goes in ~/.local unless PREFIX says otherwise. `gtk/install.sh --uninstall` removes it.
set -eu
cd "$(dirname "$0")/.."
prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin/gnotes"
apps="$prefix/share/applications"
icons="$prefix/share/icons/hicolor/scalable/apps"
if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$bin" "$apps/com.erisdor.Gnotes.desktop" "$icons/com.erisdor.Gnotes.svg"
  echo "Removed Gnotes from $prefix"
  exit 0
fi
cargo build --release -p gnotes-gtk
install -Dm755 target/release/gnotes "$bin"
install -Dm644 gtk/data/com.erisdor.Gnotes.desktop "$apps/com.erisdor.Gnotes.desktop"
install -Dm644 gtk/data/com.erisdor.Gnotes.svg "$icons/com.erisdor.Gnotes.svg"
if command -v update-desktop-database >/dev/null; then update-desktop-database "$apps" || true; fi
echo "Installed $bin; Gnotes is in your apps"
