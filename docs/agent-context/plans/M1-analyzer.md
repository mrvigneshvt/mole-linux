# Plan M1 — read-only storage analyzer (TUI)

Status: approved for implementation 2026-10-19. Decisions: [decisions/2026-10-19-v1-scope.md](../decisions/2026-10-19-v1-scope.md).
Upstream behavior reference: [research/upstream-analyzer-status.md](../research/upstream-analyzer-status.md) (Mole `cmd/analyze`, commit `8c55e8c`). GPL-3.0 allows porting logic; credit tw93/Mole in code comments where logic is ported.

## Hard boundaries (all tasks)

- **Read-only.** No unlink/rename/trash/chmod anywhere. No sudo. No network.
- Approved deps only: `ratatui`, `crossterm`, `rustix` (fs feature). Anything else → stop and report BLOCKED.
- Rust 2024, toolchain via `~/.cargo/bin` (1.99.1). No `.unwrap()` outside tests; no `as` casts on sizes from the filesystem (`try_from` + handle).
- TDD red-first: write failing test, run it, see right failure, then implement. Never edit a test to make it pass.
- Verify loop before claiming done: `cargo check && cargo clippy --all-targets -- -D warnings && cargo test`.
- Do not touch `docs/agent-context/` except appending gaps to `KNOWN-GAPS.md`. Do not commit.

## Architecture

```
src/main.rs        arg parsing (std only): `mole-linux [PATH]` → TUI; `--version`
src/scan.rs        pure scanner: Path → Node tree + ScanReport   (no TUI imports)
src/tui.rs         ratatui app over a finished/ongoing Node tree
tests/scan.rs      fixture tests using tempdirs built in std::env::temp_dir()
```

Scanner core must be testable without a terminal; TUI is a thin view.

## T1 — scanner core (`src/scan.rs`, `tests/scan.rs`)

Model per entry: name (`OsString`, non-UTF-8 safe), kind (dir/file/symlink/other), `apparent` bytes (st_size), `allocated` bytes (`st_blocks*512`), children, `status: Complete | Partial | Unavailable` with error reason.

Rules (ported from Mole analyzer, adapted to Linux):
1. Never follow symlinks; a symlink counts its own lstat size.
2. Hardlinks counted once per scan via `(st_dev, st_ino)` set for files with `st_nlink > 1`.
3. Do not cross mount boundaries (`st_dev` differs from root's) — record as child with status `Unavailable("mount boundary")`, size 0 and excluded from totals.
4. Permission/IO errors: entry `Unavailable(reason)`; every ancestor becomes `Partial`. Partial totals never reported as complete.
5. Children sorted by allocated size desc, ties by name.
6. Cancellation: scan takes an `&AtomicBool`; when set, returns promptly with unfinished nodes `Partial`.
7. Bounded parallelism is optional in T1; correctness first. If added, std threads only.

Acceptance tests (fixture-built, each red first):
- sparse file: apparent ≫ allocated.
- two hardlinks to one file in different subdirs → counted once in root total.
- symlink to a large file → only link size counted; symlink loop doesn't hang.
- unreadable subdir (chmod 000; skip test if running as root) → `Unavailable`, parent `Partial`.
- non-UTF-8 filename and name with `\n` survive in model.
- cancellation flag pre-set → returns `Partial` quickly.
- sort order deterministic.
- mount boundary: unit-test the decision function with injected dev ids (real mounts not required).

## T2 — TUI (`src/tui.rs`, `src/main.rs`)

Depends on T1. Mole-like explorer:
- Header: path, total allocated (and apparent), filesystem free/total via `statvfs` (rustix), scan status.
- List: top children with size, percent bar (percent suppressed when parent `Partial`), kind marker; unknown sizes shown as `?`.
- Keys: ↑/↓ or j/k move, Enter/→/l descend, ←/h/Backspace up, `a` toggle apparent/allocated, `q`/Esc quit, Ctrl+C quit.
- Scan runs on a background thread; UI shows "scanning…" spinner and stays responsive; quitting sets cancel flag.
- Filenames rendered safely (control chars escaped).
- Terminal always restored on exit and panic (panic hook).
- Testable logic (selection/navigation state, size formatting, escaping) lives in plain functions with unit tests; rendering tested with `ratatui::backend::TestBackend` for at least one snapshot of the header/list.
- Manual check: `cargo run -- ~/Workspace` renders, navigates, quits cleanly.

## Out of scope for M1

Deletion, cleanup hints, JSON output (needs serde — not approved), caching, overview of `/`, Btrfs extent accounting, deleted-open-file diagnosis.
