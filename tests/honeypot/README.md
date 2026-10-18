# Honeypot test environment

`manifest.tsv` is the single path/type source for the Rust builder and Docker seed script. It defines directories, protected canaries, deletable target files, a tracked Git source file, a repository, and a linked worktree. The fixture builder creates a disposable Linux-style HOME with a real Git commit and an untracked authored file under `node_modules`. Git must be available on the host for Rust fixture tests; the Arch image installs it. `HomeFixture::assert_canaries_intact()` compares deterministic checksums for every canary; call it after every mutating test action. The manifest covers credentials, browser profiles and SQLite sidecars, user documents, Docker-like data, model weights, project source and Git metadata, plus a protected whitelist path.

`generate_tool_shims(bin, log)` creates executable command shims for pacman, paccache, journalctl, coredumpctl, flatpak, docker, sudo, npm, pnpm, uv, cargo, go, and gradle. Each shim records tab-delimited argv and prints canned output.

## Arch container smoke

Build the release binary and run scenarios (each runs as the non-root `mole` user and verifies canaries afterward):

```sh
cargo build --release
tests/honeypot/docker/run.sh smoke
tests/honeypot/docker/run.sh paccache-dryrun
```

The Arch image installs `pacman-contrib`, creates a non-root user with passwordless sudo and Git, and seeds four nonempty 1 MiB versions of each of two packages. `smoke.sh` checks `mole-linux --version`; `paccache-dryrun.sh` runs `paccache -dk3 -v` and checks that it lists both oldest versions while retaining newer ones. The binary and selected scenario script are mounted read-only. The runner verifies all canary SHA-256 entries even when a scenario fails and returns nonzero for scenario or canary failure. The image tag is `mole-linux-honeypot:latest`.
