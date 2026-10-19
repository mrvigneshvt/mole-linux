#!/bin/bash
set -euo pipefail

target="$HOME/Downloads/installer.AppImage"
test -f "$target"
output=$(printf '%s\n\n%s\n' "$target" yes | mole-linux installer --type appimage)
test ! -e "$target"
test -f "$HOME/.local/share/Trash/files/installer.AppImage"
grep -Fq 'Space is not freed until Trash is emptied.' <<< "$output"
printf 'Installer apply moved the selected target to Trash\n'
