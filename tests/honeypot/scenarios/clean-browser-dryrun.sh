#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
export XDG_CACHE_HOME="$HOME/.cache"
output=$("$binary" clean --dry-run --select browser-cache)
printf '%s\n' "$output"
[[ "$output" == *".cache/chromium"* ]]
[[ "$output" == *".cache/BraveSoftware/Brave-Browser"* ]]
[[ "$output" == *".cache/mozilla/firefox/x.default-release/cache2"* ]]
[[ "$output" != *"Measured statvfs free-space delta"* ]]
[[ -f "$HOME/.cache/chromium/Default/Cache/mole-linux-cache-fixture" ]]
[[ -f "$HOME/.cache/BraveSoftware/Brave-Browser/Default/Cache/mole-linux-cache-fixture" ]]
[[ -f "$HOME/.cache/mozilla/firefox/x.default-release/cache2/mole-linux-cache-fixture" ]]
