# Honeypot test environment

The Rust fixture builder in `fixtures.rs` creates a disposable Linux-style HOME with XDG directories, browser caches and profiles, npm/pnpm/uv/cargo/Gradle caches, project build output and Git worktree metadata, Downloads, Trash, credential canaries, and SQLite database sidecars. `canary-manifest.txt` is shared by the Rust builder and Docker seed script. `HomeFixture::assert_canaries_intact()` compares deterministic checksums for protected files; call it after every mutating test action.

`generate_tool_shims(bin, log)` creates executable command shims for pacman, paccache, journalctl, coredumpctl, flatpak, docker, sudo, npm, pnpm, uv, cargo, go, and gradle. Each shim records tab-delimited argv and prints canned output.

## Arch container smoke

Build the release binary and run the smoke scenario:

```sh
cargo build --release
tests/honeypot/docker/run.sh smoke
```

The Docker image uses Arch Linux, installs `pacman-contrib`, creates a non-root user with passwordless sudo, seeds several package-cache versions and protected canary paths matching the Rust fixture contract. The smoke run mounts the release binary read-only, checks `mole-linux --version`, then verifies all canary SHA-256 entries. It exits nonzero on build, binary, or canary failure.
