#!/usr/bin/env bash
# read.sh <workspace-id> [lines]
w=$1; tid=$($(dirname "$0")/term.sh "$w")
superset terminals read --local --workspace "$w" --terminal "$tid" --max-lines "${2:-80}" --json | jq -r .text | grep -v '^\s*$'
