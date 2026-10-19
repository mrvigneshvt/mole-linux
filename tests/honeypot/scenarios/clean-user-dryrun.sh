#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
touch -d '8 days ago' "$HOME/Downloads/stale.part"
output=$("$binary" clean --dry-run)
grep -Fq '.cache/thumbnails/old-preview.png' <<<"$output"
grep -Fq '.local/state/honeypot-app/log/old.log' <<<"$output"
grep -Fq "Not selected [user] $HOME/.local/share/Trash/files/old.txt" <<<"$output"
! grep -Fq 'Measured statvfs free-space delta' <<<"$output"
test -f "$HOME/Downloads/stale.part"
