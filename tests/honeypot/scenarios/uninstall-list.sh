#!/usr/bin/env bash
set -euo pipefail

# The honeypot container mounts mole-linux on PATH and passes no argument; host tests pass the path.
binary=${1:-mole-linux}
output=$("$binary" uninstall --list 2>&1)
printf '%s\n' "$output"
grep -Eq '^pacman explicit: [0-9]+ item' <<< "$output"
grep -Fq 'nothing is removed' <<< "$output"
