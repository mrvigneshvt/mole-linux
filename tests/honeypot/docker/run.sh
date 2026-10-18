#!/bin/bash
set -euo pipefail

scenario=${1:-smoke}
if [[ ! "$scenario" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
    printf 'invalid scenario name: %s\n' "$scenario" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
dockerfile_dir="$repo_root/tests/honeypot/docker"
build_context="$repo_root/tests/honeypot"
scenario_file="$repo_root/tests/honeypot/scenarios/$scenario.sh"
binary="$repo_root/target/release/mole-linux"
image=mole-linux-honeypot:latest

if [[ ! -f "$scenario_file" ]]; then
    printf 'unknown scenario: %s\n' "$scenario" >&2
    exit 2
fi
if [[ ! -x "$binary" ]]; then
    printf 'missing release binary; build with cargo build --release first\n' >&2
    exit 1
fi

docker build --file "$dockerfile_dir/Dockerfile" --tag "$image" "$build_context"
docker run --rm \
    --mount "type=bind,src=$binary,dst=/usr/local/bin/mole-linux,readonly" \
    --mount "type=bind,src=$scenario_file,dst=/tmp/honeypot-scenario.sh,readonly" \
    "$image" bash -c '
set +e
if [ "$(id -un)" != mole ]; then
  printf "scenario is not running as mole\n" >&2
  exit 1
fi
bash /tmp/honeypot-scenario.sh
scenario_status=$?
(cd /home/mole && sha256sum --check canary.sha256)
canary_status=$?
if [ "$scenario_status" -ne 0 ]; then
  printf "scenario failed with status %s\n" "$scenario_status" >&2
  exit "$scenario_status"
fi
if [ "$canary_status" -ne 0 ]; then
  printf "canary verification failed with status %s\n" "$canary_status" >&2
  exit "$canary_status"
fi
'
