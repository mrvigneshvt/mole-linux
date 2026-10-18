#!/usr/bin/env bash
# gate.sh <worktree>  → orchestrator's independent checks; full logs in <worktree>/target/gate.log
cd "$1" || exit 1
log=target/gate.log; mkdir -p target
{ git status --short; git log --oneline main..HEAD; } 
( cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test ) >"$log" 2>&1; rc=$?
grep -E "^test result|FAILED|panicked|^error|^warning: unused" "$log" | sort | uniq -c | head -20
echo "GATE_RC=$rc"
echo "--- sink invariant (remove/rename outside sink.rs):"
grep -rnE 'fs::remove_(file|dir)|fs::rename|remove_dir_all|unlinkat|renameat' src --include=*.rs | grep -v 'src/core/sink.rs' | head
