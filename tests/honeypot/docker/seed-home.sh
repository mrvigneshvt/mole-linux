#!/bin/bash
set -euo pipefail

home=${1:?usage: seed-home.sh HOME}
mkdir -p "$home"/{.config,.cache,.local/share,.local/state,.mozilla/firefox/profile,.npm/_cacache,.cache/pnpm/store/v3,.cache/uv/archive-v0,.cargo/registry/src,.gradle/caches/modules-2,project/node_modules,project/target,project/.git/objects,project/.git/refs/heads,project/.git/worktrees/linked,project-linked,browser/Default/Cache,browser/Default}
mkdir -p "$home/Downloads" "$home/.local/share/Trash/files" "$home/.local/share/Trash/info" "$home/.ssh"
printf 'fixture installer\n' > "$home/Downloads/installer.AppImage"
printf 'old trashed file\n' > "$home/.local/share/Trash/files/old"
printf '[core]\n\trepositoryformatversion = 0\n' > "$home/project/.git/config"
printf 'ref: refs/heads/main\n' > "$home/project/.git/HEAD"
printf 'ref: refs/heads/main\n' > "$home/project/.git/worktrees/linked/HEAD"
printf '../../../../project-linked/.git\n' > "$home/project/.git/worktrees/linked/gitdir"
printf 'gitdir: ../project/.git/worktrees/linked\n' > "$home/project-linked/.git"

manifest=/usr/local/share/mole-honeypot/canary-manifest.txt
while IFS= read -r relative; do
    [[ -n "$relative" ]] || continue
    mkdir -p "$home/$(dirname "$relative")"
    if [[ "$relative" == *.db ]]; then
        printf 'SQLite format 3\0mole-linux protected canary: %s\n' "$relative" > "$home/$relative"
    else
        printf 'mole-linux protected canary: %s\n' "$relative" > "$home/$relative"
    fi
done < "$manifest"
(cd "$home" && xargs sha256sum < "$manifest" > canary.sha256)
