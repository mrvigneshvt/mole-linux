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
- 2026-10-19: Implementing issue #3 (M0.2): clap command surface and flag parsing, with analyzer routed through `analyze` and no-argument analysis retained. Non-analyze commands remain explicit exit-2 placeholders; JSON analyze output is also explicitly unavailable. Parser/help contract tests were added; their first run failed on the old CLI as expected, and the updated parsing tests pass. Full gate and live analyzer verification remain pending.
- 2026-10-19: Issue #3 M0.2 completed locally: fmt/clippy/full tests passed, including the CLI red-first contracts; tmux showed the analyzer working through both `analyze PATH` and no-argument invocation against a temporary fixture/HOME. Subcommand behavior remains pending by design for its later milestones; no cleanup behavior is implemented by this CLI skeleton.
- 2026-10-19: Owner approved roadmap + autonomous run ([decision](decisions/2026-10-19-autonomous-run.md)). Added root AGENTS.md for workers. Next: GitHub issues, honeypot, dispatch.
- 2026-10-19: Issues #2–#44 created; wave 1 (#2,#3,#4) dispatched. Orchestrator scripts + live state: [ORCHESTRATION.md](ORCHESTRATION.md).
- 2026-10-19: Implemented issue #4 (M2.1): `scan_progressive` publishes the immediate listing, then child results from up to 12 standard threads with shared hardlink dedupe; cancellation returns a partial report. Red tests first, then clippy and all 21 tests passed. Live tmux showed the list while scanning. Warm release scan of `/home/vixyz`: 5.83s vs `du -sx --one-file-system`: 7.41s (0.79×); allocated totals 117,831,655,424 bytes vs 115,081,980 KiB from `du` (117,843,947,520 bytes, about 0.01% lower; cache was warm for both). Cold scan was 38.5s and cold `du` was killed by the environment, so cold comparison remains unverified. Next: orchestrator review and live gate.
- 2026-10-19: Implemented M0.1 test honeypot: disposable HOME builder and checksum canaries, argv-logging tool shims, and Arch Docker smoke scenario. `cargo test` passes; `tests/honeypot/docker/run.sh smoke` printed `mole-linux 0.1.0` and verified all nine SHA-256 canaries. See [tests/honeypot/README.md](../../tests/honeypot/README.md). M0.1 ledger row added.
- 2026-10-19: M0.1 review round 1 expanded the shared manifest to 22 protected canaries and realistic targets, switched to a real Git repo/worktree, and added selectable container scenarios plus populated pacman caches. Rust and both Arch scenarios pass; see issue #2 review commit.
- 2026-10-19: Incident: #41 (PR #53) merged with a failing gate (duplicate [dev-dependencies] after auto-merge) — main broke briefly; fixed in follow-up PR; merge.sh now runs the gate itself and refuses to merge on failure.
- 2026-10-19 06:05: Codex 5h usage at 9% (resets 07:50); new units dispatched to Haiku 5.5 workers (#19, #25). Merged so far: #2–#16 (except none open in M2/M3), #30, #31, #33, #37, #38, #40–#43.
- 2026-10-19 ~07:00 standup (owner asked: no new workers, merge all, report): merged #2–#22, #25–#35, #37–#43, #81 plus partial #23 (editor caches) and #24 (docker advisory + misc langs). Open: #23/#24 remainders, #36 uninstall execution + selector TUI, #44 final audit + v0.1.0 release. Main: fmt/clippy clean, 529 tests pass locally. CI on main broke after #25 root refusal (Arch container ran as root) and an Arch-only honeypot test on Ubuntu — fixed in the CI PR from this branch. Codex hit its 5h limit at ~06:20 (resets 07:50); Haiku 5.5 took over remaining units. Not done: real-host apply runs (dry-run only, by policy), v0.1.0 tag/release, PKGBUILD checksum, parity ledger refresh.
