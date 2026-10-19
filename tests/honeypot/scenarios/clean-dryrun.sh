#!/usr/bin/env bash
set -euo pipefail

binary=${1:-mole-linux}
"$binary" clean --dry-run
