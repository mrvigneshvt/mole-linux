#!/bin/bash
set -euo pipefail

scenario=${1:-smoke}
if [[ "$scenario" != smoke ]]; then
    printf 'unsupported scenario: %s\n' "$scenario" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
dockerfile_dir="$repo_root/tests/honeypot/docker"
build_context="$repo_root/tests/honeypot"
binary="$repo_root/target/release/mole-linux"
image=mole-linux-honeypot:issue-2

if [[ ! -x "$binary" ]]; then
    printf 'missing release binary; build with cargo build --release first\n' >&2
    exit 1
fi
docker build --file "$dockerfile_dir/Dockerfile" --tag "$image" "$build_context"
docker run --rm --mount "type=bind,src=$binary,dst=/usr/local/bin/mole-linux,readonly" "$image" \
    bash -ec 'mole-linux --version; (cd /home/mole && sha256sum --check canary.sha256)'
