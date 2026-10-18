# Decision: autonomous overnight parity run — 2026-10-19

Owner approved [plans/PARITY-ROADMAP.md](../plans/PARITY-ROADMAP.md) and asked for every unit to be a GitHub issue, run to completion by a Claude orchestrator while the owner sleeps.

| Topic | Decision |
|---|---|
| Implementers | **Only** Codex `gpt-6-luna`, effort `medium`, via Superset (`--agent codex --model gpt-6-luna --effort medium`). Claude session = orchestrator/QA only. |
| Tracking | One GitHub issue per roadmap unit on `mrvigneshvt/mole-linux`; milestone labels. |
| Merging | Orchestrator may squash-merge PRs to `main` after its own gate passes (rerun suite + live honeypot run). One PR per issue. |
| Pre-approved deps | serde, serde_json, clap, clap_complete, tempfile (dev), glob/globset, plus earlier ratatui, crossterm, rustix. Anything else → worker BLOCKS. |
| Live safety | Mutating/privileged behavior is exercised only in a **honeypot**: disposable Docker Arch container (docker works without sudo) with seeded home, canary files and real pacman/paccache, plus fake-tool PATH shims. Real host: `--dry-run` only, never sudo, never mutate real `$HOME`. |
| Name | Keep `mole-linux` for now; rename later (upstream TRADEMARK.md). |
| Release | End of run: tag `v0.1.0` and create a GitHub release with binaries + SHA256SUMS. AUR publishing stays manual. |
