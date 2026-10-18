#!/usr/bin/env bash
# term.sh <workspace-id>  → prints terminal id of the worker
superset terminals list --local --workspace "$1" --json | jq -r '.sessions[0].terminalId'
