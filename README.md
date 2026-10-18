# mole-linux

A Rust-based, Linux-first cleanup and system-inspection tool, inspired by the kinds of workflows offered by macOS Mole. This is an independent project; it is not affiliated with or a port of Mole.

## Status

Early scaffold. No cleanup or inspection commands are implemented. Initial target is Omarchy/Arch Linux; broader Linux support is undecided.

## Build

Requires a Rust toolchain with Cargo.

```sh
cargo run
```

## Safety direction

Any future file-removal workflow should present reviewable candidates and a dry-run preview before deletion. Supported paths, cleanup categories, privilege boundaries, and broader distribution support still need product decisions.

## Agent context

Start with [`docs/agent-context/README.md`](docs/agent-context/README.md), then read [`STATUS.md`](docs/agent-context/STATUS.md) and [`HANDOFF.md`](docs/agent-context/HANDOFF.md).
