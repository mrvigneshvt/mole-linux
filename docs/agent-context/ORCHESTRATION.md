# Orchestration state (live)

Autonomous run per [decisions/2026-10-19-autonomous-run.md](decisions/2026-10-19-autonomous-run.md). Orchestrator = Claude session in workspace `b4f9a8b8`; workers = Codex gpt-6-luna medium, one Superset worktree workspace per issue (branch `issue-<n>` from main). Helpers: `scripts/orchestrator/{dispatch,wait,read,send,term}.sh`.

Gate before merge: rerun fmt/clippy/test in the worker worktree, review diff (hostile-input pass), live run (tmux / temp HOME / honeypot), then push branch, PR, squash-merge, close issue, delete nothing.

| Issue | Unit | Deps | Workspace | Status |
|---|---|---|---|---|
| #2 | M0.1 honeypot | – | 014bc3e5 | running |
| #3 | M0.2 clap CLI | – | e9ba1191 | running |
| #4 | M2.1 progressive scan | – | 2c9cb481 | running |
| #5–#44 | see issues | | | pending |

Resume: `gh issue list --label parity --state open`; for running rows run `scripts/orchestrator/read.sh <ws>`.
