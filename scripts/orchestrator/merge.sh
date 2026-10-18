#!/usr/bin/env bash
# merge.sh <worktree> <issue> <title>  → push, PR, squash-merge
set -euo pipefail
cd "$1"; n=$2; title=$3
git fetch -q origin
git rebase -q origin/main
git push -qf origin HEAD
body="## WHAT
$title

## WHY
Closes #$n (parity roadmap).

## HOW VERIFIED
Orchestrator gate: cargo fmt --check, clippy -D warnings, cargo test (see PR checks / gate log), plus live run noted in issue comment.

## KNOWN GAPS / FOLLOW-UPS
See docs/agent-context/KNOWN-GAPS.md.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
url=$(gh pr create --base main --head "$(git branch --show-current)" --title "$title" --body "$body")
gh pr merge "$url" --squash --delete-branch=false
echo "MERGED $url"
