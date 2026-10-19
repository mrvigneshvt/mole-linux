#!/usr/bin/env bash
# merge.sh <worktree> <issue> <title>  → push, PR, squash-merge
set -euo pipefail
cd "$1"; n=$2; title=$3
git fetch -q origin
if ! git rebase -q origin/main; then
  # ledger/status are orchestrator-owned: keep main's copy, then continue
  for f in $(git diff --name-only --diff-filter=U); do
    case $f in docs/agent-context/plans/PARITY-LEDGER.md|docs/agent-context/STATUS.md) git checkout --ours "$f"; git add "$f";; Cargo.lock) git checkout --ours "$f"; cargo generate-lockfile -q 2>/dev/null || cargo check -q; git add "$f";;
      *) echo "CONFLICT $f — resolve manually"; exit 3;; esac
  done
  GIT_EDITOR=true git rebase --continue
fi
cargo fmt --all
if ! git diff --quiet; then git commit -qam "style: cargo fmt after rebase

Agent-Context: n/a — formatting"; fi
"$(dirname "$(readlink -f "$0")")/gate.sh" "$PWD" > target/gate-summary.txt 2>&1 || true
if ! grep -q "GATE_RC=0" target/gate-summary.txt; then cat target/gate-summary.txt; echo "GATE FAILED — not merging"; exit 4; fi
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
