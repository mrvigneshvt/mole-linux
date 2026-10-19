#!/usr/bin/env bash
set -euo pipefail

binary=${1:-/usr/local/bin/mole-linux}
output=$("$binary" clean --yes --select project-pycache)
printf '%s\n' "$output"
[[ "$output" == *"$HOME/project/pkg/__pycache__"* ]]
[[ ! -e "$HOME/project/pkg/__pycache__" ]]
[[ -f "$HOME/project/src/__pycache__/tracked.pyc" ]]
[[ -f "$HOME/project/nested-cache/__pycache__/mole-linux-pycache-fixture.pyc" ]]
[[ -f "$HOME/project/nested-cache/__pycache__/vendored/.git/HEAD" ]]
