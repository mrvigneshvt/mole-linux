#!/usr/bin/env bash
# env.sh <workspace-id>  → last SUPERSET_WORKER envelope (screen first, then transcript)
w=$1; d=$(dirname "$0"); tid=$($d/term.sh "$w")
out=$($d/read.sh "$w" 200 | grep -A14 "SUPERSET_WORKER_\(DONE\|BLOCKED\)" | tail -16)
[ -z "$out" ] && out=$(superset agents read --local --workspace "$w" --terminal "$tid" --json 2>/dev/null | jq -r '.. | strings' 2>/dev/null | grep -A14 "SUPERSET_WORKER_\(DONE\|BLOCKED\)" | tail -16)
echo "${out:-NO ENVELOPE}"
