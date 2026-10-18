#!/usr/bin/env bash
# Usage: [WORKER=haiku] dispatch.sh <issue-number> <unit-id>  (WORKER=haiku → Claude Haiku fallback when Codex usage is exhausted)   → creates worktree workspace on branch issue-<n> from main and starts a Codex worker.
set -euo pipefail
n=$1; unit=$2
if [ "${WORKER:-codex}" = haiku ]; then agent="--agent claude --model claude-haiku-5-5"; else agent="--agent codex --model gpt-6-luna --effort medium"; fi
proj=d38ce0b7-1c16-409b-884a-d27b44b2bf13
prompt="You are the implementer for GitHub issue #$n ($unit) of mrvigneshvt/mole-linux. First read AGENTS.md (repo root) fully and obey it, then run: gh issue view $n --comments  — that issue AND its comments (orchestrator scope notes) are your spec. Read the referenced roadmap section and upstream source at the pin. Work strictly TDD red-first. Do only this issue. Commit locally with 'Closes #$n' in the body; do not push. Finish with the SUPERSET_WORKER_DONE or SUPERSET_WORKER_BLOCKED envelope from AGENTS.md."
superset workspaces create --local --project "$proj" --name "mole #$n $unit" --branch "issue-$n" --base-branch main --skip-branch-prefix \
  $agent --prompt "$prompt" --json
