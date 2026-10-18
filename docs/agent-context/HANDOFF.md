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

## Immediate next steps

1. Install or otherwise make a Rust toolchain available, then compile the scaffold.
2. Decide whether the first milestone is read-only disk analysis, cleanup candidate discovery, or another narrow workflow.
3. Define supported paths, privilege behavior, dry-run output, and distribution support before writing removal code.

## Current verification limits

`cargo` was absent from PATH on 2026-10-19; no compile or tests were run. Host OS facts and GitHub CLI authentication were checked in the setup session. Recheck them before relying on them later.
