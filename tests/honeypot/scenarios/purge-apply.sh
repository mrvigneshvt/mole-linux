#!/bin/bash
set -euo pipefail

project="$HOME/www/purge-fixture"
artifact="$project/node_modules"
mkdir -p "$artifact"
printf '{"name":"purge-fixture"}\n' > "$project/package.json"
printf 'generated cache\n' > "$artifact/cache.js"
touch -d '30 days ago' "$artifact/cache.js" "$artifact" "$project/package.json" "$project"

mkdir -p "$HOME/.config/mole-linux"
printf '%s\n' '~/www' > "$HOME/.config/mole-linux/purge_paths"
/usr/local/bin/mole-linux purge --yes

test ! -e "$artifact"
test "$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 1
test -d "$HOME/project/.git/worktrees/project-linked"
test -f "$HOME/project/.env"
test -f "$HOME/project-linked/.git"
printf 'Purged generated fixture to Trash; Git worktree and canary paths remain present\n'
