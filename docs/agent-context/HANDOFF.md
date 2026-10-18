# Handoff — mole-linux

## Why this project exists

The owner asked whether [tw93/Mole](https://github.com/tw93/Mole/) supports this machine. Its current README says Mole requires macOS 12 or newer and is built for macOS. The host checked on 2026-10-19 is Omarchy 4.0.4 (Arch-based), x86_64, so Mole is not a supported system cleaner here. The owner then proposed building an open-source Linux counterpart in Rust and selected `Workspace/Projects/personal/mole-linux`.

## What exists

- A minimal Cargo binary crate (`Cargo.toml`, `src/main.rs`) with no dependencies.
- A README and this agent-context set.
- No cleanup behavior, feature design, distribution matrix, or upstream relationship.

## Guardrails for next work

- Keep the implementation independent; do not imply affiliation with or compatibility with the macOS Mole project.
- Before non-trivial implementation, map the relevant system behavior, propose scope and safety boundaries, and wait for agreement.
- For deletion features, start with inventory and dry-run review; never silently remove candidates.
- Keep distro-specific behavior explicit and do not assume Arch paths or package-manager behavior apply to all Linux systems.
- Use red-first tests for implementation. Do not claim build/test success until run with a Rust toolchain.
- Do not add dependencies without approval. Do not commit automatically.

## Immediate next steps (updated 2026-10-19, after M1)

1. M1 analyzer TUI is implemented and committed — see [plans/M1-analyzer.md](plans/M1-analyzer.md), [STATUS.md](STATUS.md), [KNOWN-GAPS.md](KNOWN-GAPS.md).
2. Full-parity plan drafted: [plans/PARITY-ROADMAP.md](plans/PARITY-ROADMAP.md) (closed loop, M2–M10) + [plans/PARITY-LEDGER.md](plans/PARITY-LEDGER.md). Awaiting owner approval; M1.1 progressive scan is now M2.1.
3. Add GPL-3.0 LICENSE text; choose a non-Mole name before public release.
4. Working model: this repo is orchestrated from a Claude session (CTO/QA/reviewer) dispatching Codex `gpt-6-luna` medium workers via Superset (`superset agents create --agent codex --model gpt-6-luna --effort medium`). Orchestrator reruns checks and drives the live TUI in tmux before accepting. Fast tier needs `service_tier="priority"`, not passable via Superset CLI.

## Current verification limits

Rust 1.99.1 via rustup/mise is installed (2026-10-19); M1 suite green — see STATUS.md. Host OS facts and GitHub CLI authentication were checked in the setup session. Recheck them before relying on them later.
