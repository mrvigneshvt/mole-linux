#!/usr/bin/env bash
# send.sh <workspace-id> <text>
w=$1; tid=$($(dirname "$0")/term.sh "$w")
superset terminals send --local --workspace "$w" --terminal "$tid" --text "$2" --json >/dev/null
sleep 1; superset terminals send --local --workspace "$w" --terminal "$tid" --keys enter --json >/dev/null
