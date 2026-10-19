#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
# The Arch honeypot ships without desktop-file-utils, so an existing applications
# directory must surface as Unavailable (tool missing), never as a silent success.
mkdir -p "$HOME/.local/share/applications"
output=$("$binary" optimize)
printf '%s\n' "$output"
[[ "$output" == *"Applied "*" optimizations"* ]]
[[ "$output" == *"Desktop & MIME Database Refresh: unavailable"* ]]
[[ "$output" == *"Journal Health:"* ]]

# A whitelist_optimize entry must skip that task and never run its tool.
mkdir -p "$HOME/.config/mole-linux"
printf 'journal_health_report\n' > "$HOME/.config/mole-linux/whitelist_optimize"
output=$("$binary" optimize)
printf '%s\n' "$output"
[[ "$output" == *"Journal Health: skipped"* ]]
