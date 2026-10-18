# Orchestration state (live)

Autonomous run per [decisions/2026-10-19-autonomous-run.md](decisions/2026-10-19-autonomous-run.md). Orchestrator = Claude session in workspace `b4f9a8b8`; workers = Codex gpt-6-luna medium, one Superset worktree workspace per issue (branch `issue-<n>` from main). Helpers: `scripts/orchestrator/{dispatch,wait,read,send,term}.sh`.

Gate before merge: rerun fmt/clippy/test in the worker worktree, review diff (hostile-input pass), live run (tmux / temp HOME / honeypot), then push branch, PR, squash-merge, close issue, delete nothing.

| Issue | Unit | Workspace | Status |
|---|---|---|---|
| #2 | M0.1 Honeypot test environment (Docker Arch + canaries + too | 014bc3e5 | merged |
| #3 | M0.2 clap CLI skeleton with all subcommands | e9ba1191 | merged |
| #4 | M2.1 Progressive + parallel scan | 2c9cb481 | merged |
| #5 | M2.2 Analyze overview mode | 6d4af52c | running |
| #6 | M2.3 Large-files view, filter, multi-select, refresh, histor | f5eb20ae | merged |
| #7 | M2.4 analyze --json | ca903858 | merged |
| #8 | M2.5 Cleanable hints (CACHEDIR.TAG, artifact names) | af3915f2 | merged |
| #9 | M2.6 Btrfs awareness banner | 05f0b9cd | merged |
| #10 | M3.1 Path model + deletion validator | 9dd64a0e | merged |
| #11 | M3.2 Deletion sink + freedesktop Trash | ac24dc05 | merged |
| #12 | M3.3 Operation log + history command | dd56514a | running |
| #13 | M3.4 Whitelist | ffd1d602 | merged |
| #14 | M3.5 Sink invariant test + owner-process probe + outcome enu | 3cf076ed | running |
| #15 | M3.6 Analyze delete-to-Trash | - | pending |
| #16 | M4.1 Clean framework: rule catalog + section runner + summar | - | pending |
| #17 | M4.2 User essentials (Trash, thumbnails, logs, coredumps, pa | - | pending |
| #18 | M4.3 Browser caches (Chromium family, Firefox) | - | pending |
| #19 | M4.4 Project caches (__pycache__) | - | pending |
| #20 | M5.1 Owner-tool adapter framework + JS (npm, pnpm, bun, core | - | pending |
| #21 | M5.2 Python (uv, pip, conda, pyinstaller) | - | pending |
| #22 | M5.3 Rust/Go/JVM/mise (cargo, go, gradle, maven, mise) | - | pending |
| #23 | M5.4 Editors, JetBrains, AI agents, automation browsers | - | pending |
| #24 | M5.5 Docker advisory + misc languages | - | pending |
| #25 | M6.1 Privilege model (sudo per owner action) | - | pending |
| #26 | M6.2 pacman + AUR caches via paccache | - | pending |
| #27 | M6.3 journald + coredump retention | - | pending |
| #28 | M6.4 Orphan packages report (read-only) | - | pending |
| #29 | M6.5 Flatpak unused refs | - | pending |
| #30 | M6.6 Btrfs/snapper snapshot explainer (read-only) | 87d9ffdf | running |
| #31 | M7.1 Purge: discovery + protection | - | pending |
| #32 | M7.2 Purge: selection TUI + execution | - | pending |
| #33 | M7.3 Installer cleanup | - | pending |
| #34 | M8.1 Uninstall inventory | - | pending |
| #35 | M8.2 Leftover discovery + personal-data review | - | pending |
| #36 | M8.3 Uninstall execution + selector TUI | - | pending |
| #37 | M9.1 Status collectors (/proc, /sys) | 16b523bb | merged |
| #38 | M9.2 Status TUI + --json + --watch NDJSON | 810c95cf | running |
| #39 | M9.3 Optimize (Linux catalog, six-state outcomes) | - | pending |
| #40 | M9.4 Interactive main menu + terminal restore | - | pending |
| #41 | M9.5 Shell completion | a264a863 | merged |
| #42 | M10.1 CI (fmt, clippy, test, honeypot) + release workflow | - | pending |
| #43 | M10.2 LICENSE, README, PKGBUILD | - | pending |
| #44 | M10.3 Final audit + v0.1.0 release | - | pending |

Resume: `gh issue list --label parity --state open`; for running rows run `scripts/orchestrator/read.sh <full-ws-id>` (full ids: `superset workspaces list --local --json`, names `mole #<n> ...`). Merge only via `scripts/orchestrator/merge.sh` (gate-enforcing).

Codex usage: 5h window, ~32% left at 05:20, resets 07:50 local. On exhaustion use `WORKER=haiku scripts/orchestrator/dispatch.sh`.
