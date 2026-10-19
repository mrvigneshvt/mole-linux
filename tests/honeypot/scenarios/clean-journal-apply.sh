#!/bin/bash
set -euo pipefail

# Plants one active and one archived journal file in the container's root-owned journal through
# sudo. `--vacuum-size=512M` keeps a tiny archived file, so the apply runs real sudo and journalctl
# and removes nothing that matters; the active journal must survive either way.
binary=/usr/local/bin/mole-linux
id=$(cat /etc/machine-id 2>/dev/null || true)
if [[ ! "$id" =~ ^[0-9a-f]{32}$ ]]; then
    id=00000000000000000000000000000001
    printf '%s\n' "$id" | sudo tee /etc/machine-id >/dev/null
fi
journal=/var/log/journal/$id
sudo install -d -m 2755 "$journal"
printf 'active' | sudo tee "$journal/system.journal" >/dev/null
printf 'archived' | sudo tee "$journal/system@0000000000000000-0000000000000001-0000000000000001.journal" >/dev/null

preview=$("$binary" clean --dry-run --select journal)
printf '%s\n' "$preview"
grep -Fq '1 archived file(s)' <<< "$preview"
if [[ ! -f "$journal/system.journal" ]]; then
    printf 'dry run changed the journal\n' >&2
    exit 1
fi

output=$("$binary" clean --yes --select journal)
printf '%s\n' "$output"
grep -Fq 'applied: 1' <<< "$output"
grep -Fq 'Clean complete' <<< "$output"
if [[ ! -f "$journal/system.journal" ]]; then
    printf 'apply removed the active journal\n' >&2
    exit 1
fi
