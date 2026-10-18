# Status

## Current state — 2026-10-19

Created initial Rust binary scaffold in `mole-linux/`. The project targets Linux and starts with Omarchy/Arch as the verified development host. No cleanup, uninstall, optimization, or monitoring features exist yet.

The host is Omarchy 4.0.4, Arch-based, x86_64. `pacman` is present. `cargo` was not found on PATH, so the scaffold has not been compiled or tested. GitHub CLI is authenticated as `mrvigneshvt`; no existing `mrvigneshvt/mole-linux` repository was found before creation.

## Decisions captured

- Build an independent Linux-focused project in Rust, inspired by the broad category of Mole workflows.
- Keep the initial scope and supported distributions open for product discussion.
- Treat dry-run candidate review as the safety direction for any future deletion command; this is a proposal, not implemented behavior.

## Current resume point — upstream research

Repository-wide source research of Mole main at `8c55e8c07f3b34eeeb0352753b660f823468cd43`
is captured in [research/mole-upstream-review.md](research/mole-upstream-review.md).
The review inventories all 323 tracked paths and traces command architecture,
critical safety flows, analyzer/status and lifecycle/maintenance behavior. It is
not an exhaustive line-by-line audit; upstream tests and macOS runtime were not run.

Recommended discussion: Arch-first read-only storage analysis, explainable
candidates, then one reviewed cleanup workflow. These are proposals; feature
scope remains unapproved. Upstream already has dry-run, Trash, history, worktree
guards and structured inspection, so these are baseline requirements. Resolve
branding and potential upstream code reuse before release; see
[KNOWN-GAPS.md](KNOWN-GAPS.md). Product source remains unchanged.

## Checkpoints

- 2026-10-19: Created README, Rust binary manifest/source, and agent context. No feature implementation or verification was possible because Cargo is unavailable. Public repository setup/push is tracked in the setup handoff.
- 2026-10-19: Added pinned upstream research and Linux product opportunities. Source-only inspection; no cleanup, dependency installation, implementation, commit or push. Next: agree the first milestone and its safety/recovery boundaries.
- 2026-10-19: Owner locked v1 decisions: GPL-3.0 with porting allowed, read-only analyzer first, Arch/Omarchy only, TUI from day one, rename before release. See [decisions/2026-10-19-v1-scope.md](decisions/2026-10-19-v1-scope.md). Next: toolchain + GPL variant + dep approval, then learning recap and handoff plan.
- 2026-10-19: Plan [plans/M1-analyzer.md](plans/M1-analyzer.md) written; T1 (scanner) dispatched to Codex worker. LICENSE file text not yet added.
- 2026-10-19: T1 scanner accepted after review round 1 (mount Excluded status, dir inode size, explicit stack). Orchestrator verified clippy + 10 tests. T2 (TUI) dispatched.
- 2026-10-19: M1 T2 TUI accepted after review round 1 (header followed root, not the current dir). Orchestrator verified: clippy clean, 18 tests, live tmux run, totals == du. Uncommitted. Next: commit decision, progressive scan, LICENSE file.
- 2026-10-19: Committed M1 analyzer (feat) on branch plan-linux-mole-tool-b4f9a8b8 with .gitignore. Not pushed. Next: M1.1 progressive scan plan.
- 2026-10-19: Drafted full-parity closed-loop plan [plans/PARITY-ROADMAP.md](plans/PARITY-ROADMAP.md) + generated [plans/PARITY-LEDGER.md](plans/PARITY-LEDGER.md) (upstream entry functions + tests; dispositions proposed). Awaiting owner approval; nothing dispatched.
- 2026-10-19: Owner approved roadmap + autonomous run ([decision](decisions/2026-10-19-autonomous-run.md)). Added root AGENTS.md for workers. Next: GitHub issues, honeypot, dispatch.
