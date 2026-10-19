#!/usr/bin/env bash
# term.sh <workspace-id>  → terminal id of the newest live worker session
superset terminals list --local --workspace "$1" --json | jq -r '[.sessions[] | select(.exited|not)] | sort_by(.createdAt) | last | .terminalId'
