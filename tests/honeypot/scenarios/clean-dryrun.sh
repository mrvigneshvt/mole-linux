#!/usr/bin/env bash
set -euo pipefail

binary=${1:?pass the mole-linux binary path}
"$binary" clean --dry-run
