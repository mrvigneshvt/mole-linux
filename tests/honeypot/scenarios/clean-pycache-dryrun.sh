#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
output=$("$binary" clean --dry-run --select project-pycache)
printf '%s\n' "$output"
[[ "$output" == *"$HOME/project/pkg/__pycache__"* ]]
[[ "$output" != *"$HOME/project/src/__pycache__"* ]]
[[ "$output" != *"$HOME/project/nested-cache/__pycache__"* ]]
[[ -f "$HOME/project/pkg/__pycache__/mole-linux-pycache-fixture.pyc" ]]
[[ -f "$HOME/project/nested-cache/__pycache__/mole-linux-pycache-fixture.pyc" ]]
