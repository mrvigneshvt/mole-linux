#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
export XDG_CACHE_HOME="$HOME/.cache"
output=$("$binary" clean --yes --select browser-cache)
printf '%s\n' "$output"
[[ "$output" == *"Measured statvfs free-space delta:"* ]]
if [[ "$output" == *"$HOME/.cache/chromium: Chromium cache (owner process is running)"* || \
    "$output" == *"$HOME/.cache/chromium: Chromium cache (owner process state is unknown)"* ]]; then
    [[ -f "$HOME/.cache/chromium/Default/Cache/mole-linux-cache-fixture" ]]
else
    [[ ! -e "$HOME/.cache/chromium" ]]
fi
if [[ "$output" == *"$HOME/.cache/BraveSoftware/Brave-Browser: Brave cache (owner process is running)"* || \
    "$output" == *"$HOME/.cache/BraveSoftware/Brave-Browser: Brave cache (owner process state is unknown)"* ]]; then
    [[ -f "$HOME/.cache/BraveSoftware/Brave-Browser/Default/Cache/mole-linux-cache-fixture" ]]
else
    [[ ! -e "$HOME/.cache/BraveSoftware/Brave-Browser" ]]
fi
[[ ! -e "$HOME/.cache/mozilla/firefox/x.default-release/cache2" ]]
[[ -f "$HOME/.config/chromium/Default/Cookies" ]]
[[ -f "$HOME/.mozilla/firefox/x.default-release/cookies.sqlite" ]]
