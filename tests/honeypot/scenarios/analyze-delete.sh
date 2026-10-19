#!/bin/bash
set -euo pipefail

test_binary=/usr/local/bin/analyze-delete-tests
parent="$HOME/.cache/mole-analyze-bind-boundary"
source_dir="$(mktemp -d /tmp/mole-bind-source.XXXXXX)"
mkdir -p "$parent/trash-parent/mounted" "$parent/permanent-parent/mounted"
printf 'bind-mount-canary' > "$source_dir/canary"
sudo mount --bind "$source_dir" "$parent/trash-parent/mounted"
sudo mount --bind "$source_dir" "$parent/permanent-parent/mounted"
cleanup() {
    trash_mount="$HOME/.local/share/Trash/files/trash-parent/mounted"
    if mountpoint -q "$trash_mount"; then sudo umount "$trash_mount"; fi
    if mountpoint -q "$parent/permanent-parent/mounted"; then
        sudo umount "$parent/permanent-parent/mounted"
    fi
    rm "$source_dir/canary"
    rmdir "$source_dir"
}
trap cleanup EXIT

MOLE_HONEYPOT_MOUNT_PARENT="$parent" "$test_binary" \
    --exact bind_mount_boundary_honeypot --nocapture
printf 'Trash and Permanent preserved canaries inside same-device bind mounts\n'
