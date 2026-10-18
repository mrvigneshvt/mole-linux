# AGENTS.md — mole-linux

Linux-first Rust port of [tw93/Mole](https://github.com/tw93/Mole) (macOS cleaner): analyze, clean, purge, installer, uninstall, optimize, status, history, completion. GPL-3.0-only. Target: Arch/Omarchy, x86_64, btrfs root.

## Read before working

1. Your GitHub issue — it is your spec. Do only that issue.
2. `docs/agent-context/plans/PARITY-ROADMAP.md` — architecture, milestone, closed loop.
3. `docs/agent-context/plans/PARITY-LEDGER.md` — upstream entry points/tests for your unit; update your rows' Status.
4. Upstream source at the pin `8c55e8c07f3b34eeeb0352753b660f823468cd43`. Local checkout: `/tmp/mole-upstream-review-20261019` (if missing: `git clone https://github.com/tw93/Mole /tmp/mole-upstream-review-20261019 && git -C … checkout 8c55e8c`). Port **behavior and test contracts**, not macOS paths. Comment ported logic: `// Ported from tw93/Mole <file>:<fn> (GPL-3.0)`.

## Hard rules

- **Single deletion sink.** All removal/rename/trash goes through `src/core/sink.rs` (once it exists). No `std::fs::remove_*` / `rename` elsewhere in `src/`. A test enforces this.
- **Preview ≠ permission.** Revalidate identity (dev, ino, type, mtime) and policy at the sink. Changed target → skip with reason.
- **Unknown is a state.** Permission errors, timeouts, missing tools, "can't tell if running" → `Unknown`/`Unavailable`, never 0 / not-running / safe.
- **Never** touch: credentials (`~/.ssh`, `~/.gnupg`, keyrings, `.env`), browser profiles/cookies/sessions, live SQLite (`*.db` + `-wal`/`-shm`), git repos/worktrees, Docker/Podman volumes, model weights, user documents. Directory names alone never authorize deletion.
- **Space freed = measured statvfs delta**, never the sum of sizes (btrfs: Trash and snapshots keep extents). Show logical vs allocated.
- **No privilege escalation by the tool itself** except invoking an owner tool through `sudo` for a single explicit action, after consent; `--dry-run` never needs sudo. Never run as root.
- **Safety in testing:** mutating code is exercised only against temp HOMEs (`tempfile`) and the honeypot (`tests/honeypot/`, Docker Arch container). **Never run a mutating command against the real `$HOME` or host system.** Real host: `--dry-run` only.
- Deps allowed: ratatui, crossterm, rustix, serde, serde_json, clap, clap_complete, tempfile (dev), glob/globset. Anything else → `SUPERSET_WORKER_BLOCKED`.
- No `.unwrap()`/`.expect()` outside tests. No `as` casts on sizes/counts from files, `/proc`, or tool output — `try_from` + handle. No silent error swallowing.
- Unknown-input rendering: escape control chars in every path/filename shown in the TUI or logs.
- No TODO/placeholder values. Can't finish → BLOCKED with reason.

## Workflow (TDD, red first)

**Plan gate is pre-approved:** your GitHub issue *is* the approved plan (owner approved the roadmap; orchestrator owns scope). Never stop to ask for approval of an in-scope plan — proceed to completion. Only stop via `SUPERSET_WORKER_BLOCKED` for a real blocker (unapproved dep, out-of-scope need, safety conflict). The owner is asleep; nobody will answer questions.


1. Branch is prepared for you; don't switch branches.
2. Write the failing test(s) first — port the upstream test contract named in the issue. Run; confirm the failure is the missing behavior.
3. Minimal implementation. Then:
   `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
4. If your unit is user-visible, run the binary (TUI via tmux) against a temp HOME or the honeypot and report what you saw.
5. Do not edit `docs/agent-context/STATUS.md`, `ORCHESTRATION.md` or `plans/PARITY-LEDGER.md` (orchestrator-owned; report ledger status in your envelope). For caveats/shortcuts (`ponytail:` comments), `docs/agent-context/KNOWN-GAPS.md`.
6. Commit (Conventional Commits, `Closes #<issue>` in body). Do **not** push or open PRs — the orchestrator does.
7. End with:

```
SUPERSET_WORKER_DONE
task: #<issue>
summary: <one line>
files: <paths>
checks: <commands + outcomes, including the red run>
handoff: <notes or none>
```
or `SUPERSET_WORKER_BLOCKED` / `task:` / `reason:` / `needs:`.

## Layout

```
src/main.rs  src/lib.rs  src/scan.rs  src/tui.rs     (M1, exists)
src/core/{paths,policy,validate,sink,oplog,outcome,proc,units}.rs
src/rules/  src/adapters/  src/commands/  src/tui/
tests/  tests/fixtures/  tests/honeypot/
```

## Units

SI (kB/MB/GB) for disk sizes in summaries, IEC (KiB/MiB) for memory — label explicitly (upstream `internal/units/bytes.go`).
