#!/usr/bin/env bash
# wait.sh <workspace-id> [timeout]  → waits until worker settles, prints state + envelope tail
w=$1; t=$(dirname "$0")/term.sh; tid=$($t "$w")
superset agents wait --local --workspace "$w" --terminal "$tid" --until settled --timeout "${2:-120}" --json 2>/dev/null | jq -r '.state // "timeout"'
