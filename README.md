# mole-linux

`mole-linux` is an independent, Linux-first Rust project inspired by the cleanup and inspection workflows of [Mole](https://github.com/tw93/Mole). It is not affiliated with, endorsed by, or an official port of tw93/Mole. Mole and its author retain their own names and trademarks. This project reuses GPL-3.0-licensed ideas and code where noted in source files; see [LICENSE](LICENSE).

## Scope and status

The project targets x86_64 Arch Linux and Omarchy. Other distributions are not currently supported or verified.

| Command | Current behavior |
|---|---|
| `analyze [PATH]` | Analyze disk usage with a TUI; `analyze --json` emits a JSON report. |
| `status` | Report system status. |
| `snapshots` | List Btrfs and Snapper snapshots without changing them. |
| `history` | Show the operation history. |
| `completion` | Generate Bash, Zsh, or Fish completion; managed shell configuration is also supported. |
| `clean`, `purge`, `installer`, `uninstall`, `optimize` | CLI surfaces exist, but these commands are not implemented yet. |

Check `mole-linux --help` and each subcommand's `--help` for available options. This is early software; command availability does not imply full parity with Mole.

## Safety model

Implemented removal logic is designed around one mutation sink. The sink rechecks the target's filesystem identity and deletion policy, including protected paths and the user whitelist, before acting. Trash is the default for eligible removals; permanent deletion requires an explicit mode. Preview is not authorization to remove a target. The honeypot uses disposable HOME data and canaries to check that protected files remain unchanged.

These protections apply only to implemented paths. The unimplemented commands above do not clean anything. Do not run mutating workflows against important data; review the preview and retain backups. Space reclaimed is filesystem-dependent, especially on Btrfs with Trash and snapshots.

## Install

### Build from a source checkout

Install Rust/Cargo, then:

```sh
git clone https://github.com/mrvigneshvt/mole-linux.git
cd mole-linux
cargo build --release
install -Dm755 target/release/mole-linux "$HOME/.local/bin/mole-linux"
```

Ensure `$HOME/.local/bin` is on `PATH`.

### Build from a release source tarball

Download and extract the source tarball attached to a project release, then run `cargo build --release` in the extracted directory. Install `target/release/mole-linux` as above. Verify downloaded release assets against the checksums published with that release when available.

### Arch package recipe

`packaging/arch/PKGBUILD` builds the source archive for the matching `v$pkgver` GitHub tag. It is provided as a recipe; no AUR package or release is implied by its presence. Use `makepkg -si` from that directory on Arch Linux.

## License and attribution

This project is licensed under GNU GPL version 3 only; the full license text is in [LICENSE](LICENSE). Mole is a separate project by tw93, available at [github.com/tw93/Mole](https://github.com/tw93/Mole). This project is community-developed and has no official relationship with tw93 or the Mole project.
