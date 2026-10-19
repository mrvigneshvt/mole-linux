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
package_dir="$repo_root/packaging/arch"
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
docker_args=(--rm)
if [[ "$scenario" == analyze-delete ]]; then
    test_binary=
    for candidate in "$repo_root"/target/debug/deps/analyze_delete-*; do
        if [[ -x "$candidate" ]]; then
            test_binary=$candidate
        fi
    done
    if [[ -z "$test_binary" ]]; then
        printf 'missing analyze-delete integration test binary; run cargo test --test analyze_delete first\n' >&2
        exit 1
    fi
    docker_args+=(--cap-add SYS_ADMIN --mount "type=bind,src=$test_binary,dst=/usr/local/bin/analyze-delete-tests,readonly")
fi
docker run "${docker_args[@]}" \
    --mount "type=bind,src=$binary,dst=/usr/local/bin/mole-linux,readonly" \
    --mount "type=bind,src=$scenario_file,dst=/tmp/honeypot-scenario.sh,readonly" \
    --mount "type=bind,src=$package_dir,dst=/tmp/mole-linux-packaging-input,readonly" \
    "$image" bash -c '
set +e
if [ "$(id -un)" != mole ]; then
  printf "scenario is not running as mole\n" >&2
  exit 1
fi
cp -R /tmp/mole-linux-packaging-input "$HOME/arch-package"
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
