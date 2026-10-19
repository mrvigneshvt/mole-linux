#!/bin/bash
set -euo pipefail

# Seeded by docker/Dockerfile: four versions of each fixture package in /var/cache/pacman/pkg.
binary=/usr/local/bin/mole-linux
cache=/var/cache/pacman/pkg
seeded=$(printf '%s\n' \
    mole-fixture-a-1.0-1-x86_64.pkg.tar.zst \
    mole-fixture-a-1.1-1-x86_64.pkg.tar.zst \
    mole-fixture-a-1.2-1-x86_64.pkg.tar.zst \
    mole-fixture-a-1.3-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.0-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.1-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.2-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.3-1-x86_64.pkg.tar.zst)
expect_kept=$(printf '%s\n' \
    mole-fixture-a-1.1-1-x86_64.pkg.tar.zst \
    mole-fixture-a-1.2-1-x86_64.pkg.tar.zst \
    mole-fixture-a-1.3-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.1-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.2-1-x86_64.pkg.tar.zst \
    mole-fixture-b-2.3-1-x86_64.pkg.tar.zst)
list_fixtures() { ls "$cache" | grep '^mole-fixture-' | sort; }

if [[ "$(list_fixtures)" != "$(sort <<< "$seeded")" ]]; then
    printf 'unexpected seeded pacman cache:\n' >&2
    list_fixtures >&2
    exit 1
fi

preview=$("$binary" clean --dry-run --select pacman-cache-keep3)
printf '%s\n' "$preview"
if [[ "$(list_fixtures)" != "$(sort <<< "$seeded")" ]]; then
    printf 'dry run changed the pacman cache\n' >&2
    exit 1
fi

output=$("$binary" clean --yes --select pacman-cache-keep3)
printf '%s\n' "$output"
actual=$(list_fixtures)
if [[ "$actual" != "$(sort <<< "$expect_kept")" ]]; then
    printf 'apply did not remove exactly the versions beyond keep=3:\n%s\n' "$actual" >&2
    exit 1
fi
grep -Fq 'Clean complete' <<< "$output"
