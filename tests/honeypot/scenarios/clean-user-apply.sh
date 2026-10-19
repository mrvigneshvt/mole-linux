#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
touch -d '8 days ago' "$HOME/Downloads/stale.part"
"$binary" clean --yes --select user-essentials
test ! -e "$HOME/.cache/thumbnails/old-preview.png"
test ! -e "$HOME/.local/state/honeypot-app/log/old.log"
test -f "$HOME/.local/share/Trash/files/old.txt"
test ! -e "$HOME/Downloads/stale.part"
test -f "$HOME/Documents/thesis.odt"
"$binary" clean --yes --select trash
test ! -e "$HOME/.local/share/Trash/files/old.txt"
