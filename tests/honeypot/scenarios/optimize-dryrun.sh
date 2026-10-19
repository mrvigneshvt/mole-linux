#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
# Dry run must not touch cache or data directories; hash names, sizes and mtimes first.
state() { find "$HOME/.cache" "$HOME/.local/share" -printf '%p %s %T@\n' 2>/dev/null | sha256sum; }
before=$(state)
output=$("$binary" optimize --dry-run)
printf '%s\n' "$output"
[[ "$output" == *"Dry run: would apply"* ]]
[[ "$output" == *"Journal Health:"* ]]
[[ "$output" != *"Applied "* ]]
[[ "$output" != *": applied - "* ]]
[[ "$before" == "$(state)" ]]
