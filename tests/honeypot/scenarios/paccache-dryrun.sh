#!/bin/bash
set -euo pipefail

output=$(paccache -dk3 -v 2>&1)
printf '%s\n' "$output"
grep -Fq 'mole-fixture-a-1.0-1-x86_64.pkg.tar.zst' <<< "$output"
grep -Fq 'mole-fixture-b-2.0-1-x86_64.pkg.tar.zst' <<< "$output"
if grep -Eq 'mole-fixture-a-1\.[123]-1|mole-fixture-b-2\.[123]-1' <<< "$output"; then
    printf 'paccache dry run selected a retained package version\n' >&2
    exit 1
fi
