//! System-category rules for the pacman and AUR caches. Pacman cache work goes through
//! `OwnerRunner` (preview unprivileged, apply via sudo); AUR build dirs go through the sink.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::paths::XdgPaths;
use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRunner};
use crate::core::units::parse_iec_size;
use crate::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode,
    RuleContext, candidate, owner_candidate_state,
};
use crate::rules::flatpak::{FlatpakUnusedRule, SYSTEM_INSTALLATION, Scope};

pub const PACMAN_CACHE_DIR: &str = "/var/cache/pacman/pkg";

/// Upper bound on entries scanned inside one AUR build directory before refusing to decide.
const MAX_TREE_ENTRIES: u64 = 100_000;

/// `paccache` preview/apply pair over the pacman package cache. Linux-native: no upstream equivalent.
pub struct PaccacheRule {
    id: &'static str,
    reason: &'static str,
    preview: &'static [&'static str],
    apply: &'static [&'static str],
    cache_dir: PathBuf,
}

impl PaccacheRule {
    /// Keeps the newest three versions of every cached package.
    pub fn keep_three(cache_dir: PathBuf) -> Self {
        Self {
            id: "pacman-cache-keep3",
            reason: "older package versions beyond the newest three",
            preview: &["-dk3"],
            apply: &["-rk3"],
            cache_dir,
        }
    }

    /// Removes cached packages that are no longer installed.
    pub fn uninstalled(cache_dir: PathBuf) -> Self {
        Self {
            id: "pacman-cache-uninstalled",
            reason: "cached packages that are no longer installed",
            preview: &["-duk0"],
            apply: &["-ruk0"],
            cache_dir,
        }
    }
}

impl CleanRule for PaccacheRule {
    fn id(&self) -> &str {
        self.id
    }

    fn category(&self) -> Category {
        Category::System
    }

    /// Previewing needs `paccache`, so this rule cannot discover without an owner runner.
    fn discover(&self, _ctx: &RuleContext<'_>) -> Discovery {
        unavailable("paccache preview needs the owner runner")
    }

    fn discover_with_owner(&self, ctx: &RuleContext<'_>, owner: &OwnerRunner) -> Discovery {
        let query = OwnerCommand::new(OwnerProgram::Paccache, self.preview.iter().copied(), false);
        let output = match owner.query(&query) {
            Ok(output) => output,
            Err(run) => {
                return unavailable(
                    &run.reason
                        .unwrap_or_else(|| "owner query failed".to_owned()),
                );
            }
        };
        let Some(count) = parse_dry_run(&output) else {
            return unavailable("unrecognized paccache preview output");
        };
        if count == 0 {
            return complete(Vec::new());
        }
        let (state, _) = owner_candidate_state(ctx, "pacman");
        let reason = format!("{count} package file(s): {}", self.reason);
        match candidate(&self.cache_dir, reason, RecoveryMode::OwnerNative, state) {
            Ok(mut found) => {
                // A directory has no size of its own; the preview's figure is the logical size.
                found.apparent_size = parse_dry_run_saved(&output);
                found.owner_check = OwnerCheck::ProcessName("pacman".to_owned());
                complete(vec![found])
            }
            Err(error) => unavailable(&format!("{}: {error}", self.cache_dir.display())),
        }
    }

    // ponytail: apply re-runs `paccache -rk3`, so paccache selects the set at apply time, not the exact files previewed. Upgrade path: confirm the previewed set is unchanged before applying. Logged in KNOWN-GAPS.md.
    fn owner_command(&self, _candidate: &Candidate) -> Option<OwnerCommand> {
        Some(OwnerCommand::new(
            OwnerProgram::Paccache,
            self.apply.iter().copied(),
            true,
        ))
    }
}

/// Reads `finished dry run: N candidates` or `no candidate packages found`; anything else is unknown.
fn parse_dry_run(output: &str) -> Option<u64> {
    if output.contains("no candidate packages found") {
        return Some(0);
    }
    let line = output
        .lines()
        .find(|line| line.contains("finished dry run:"))?;
    line.split("finished dry run:")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// The `disk space saved: 510.4 KiB` figure of a dry run; absent or unparseable is unknown.
fn parse_dry_run_saved(output: &str) -> Option<u64> {
    let line = output
        .lines()
        .find(|line| line.contains("finished dry run:"))?;
    let figure = line.split("disk space saved:").nth(1)?;
    parse_iec_size(figure.trim().trim_end_matches(')').trim())
}

/// Makepkg build directories (`src/`, `pkg/`) under each AUR cache entry. Git checkouts stay.
pub struct YayBuildRule {
    cache_root: PathBuf,
}

impl YayBuildRule {
    pub fn new(cache_root: PathBuf) -> Self {
        Self { cache_root }
    }
}

impl CleanRule for YayBuildRule {
    fn id(&self) -> &str {
        "aur-build-dirs"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let entries = match fs::read_dir(&self.cache_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return complete(Vec::new()),
            Err(error) => return unavailable(&format!("{}: {error}", self.cache_root.display())),
        };
        let mut candidates = Vec::new();
        let mut kept = 0_u64;
        let mut partial = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    partial.push(format!("unreadable AUR cache entry: {error}"));
                    continue;
                }
            };
            let package_dir = entry.path();
            if !is_real_dir(&package_dir) {
                continue;
            }
            for leaf in ["src", "pkg"] {
                let build = package_dir.join(leaf);
                if !is_real_dir(&build) {
                    continue;
                }
                match scan_tree(&build) {
                    TreeScan::Clean => {
                        // ponytail: only `makepkg` is checked at discovery and apply; a `yay` started mid-run is not rechecked. Upgrade path: a combined owner check over yay and makepkg. Logged in KNOWN-GAPS.md.
                        let (state, _) = owner_candidate_state(ctx, "makepkg");
                        match candidate(
                            &build,
                            "makepkg build directory",
                            RecoveryMode::Regenerate,
                            state,
                        ) {
                            Ok(mut found) => {
                                found.owner_check = OwnerCheck::ProcessName("makepkg".to_owned());
                                candidates.push(found);
                            }
                            Err(error) => partial.push(format!("{}: {error}", build.display())),
                        }
                    }
                    TreeScan::ContainsGit => kept = kept.saturating_add(1),
                    TreeScan::Unknown(reason) => partial.push(reason),
                }
            }
        }
        if kept > 0 {
            partial.push(format!(
                "kept {kept} build dir(s) containing a git checkout"
            ));
        }
        let status = if partial.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(partial.join("; "))
        };
        Discovery { candidates, status }
    }
}

enum TreeScan {
    Clean,
    ContainsGit,
    Unknown(String),
}

/// Walks without following symlinks. Any `.git` entry (directory or worktree file) blocks removal.
fn scan_tree(root: &Path) -> TreeScan {
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0_u64;
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => return TreeScan::Unknown(format!("{}: {error}", dir.display())),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return TreeScan::Unknown(format!("{}: {error}", dir.display())),
            };
            seen = seen.saturating_add(1);
            if seen > MAX_TREE_ENTRIES {
                return TreeScan::Unknown(format!("{}: scan budget exceeded", root.display()));
            }
            if entry.file_name() == ".git" {
                return TreeScan::ContainsGit;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(_) => {}
                Err(error) => return TreeScan::Unknown(format!("{}: {error}", dir.display())),
            }
        }
    }
    TreeScan::Clean
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

/// Every system rule. The AUR cache root follows XDG resolution; `run()` already requires HOME.
pub fn catalog() -> Vec<Box<dyn CleanRule>> {
    let mut rules: Vec<Box<dyn CleanRule>> = vec![
        Box::new(PaccacheRule::keep_three(PathBuf::from(PACMAN_CACHE_DIR))),
        Box::new(PaccacheRule::uninstalled(PathBuf::from(PACMAN_CACHE_DIR))),
        Box::new(super::orphans::OrphanPackagesRule),
    ];
    let xdg = XdgPaths::from_env();
    if let Some(xdg) = &xdg {
        rules.push(Box::new(YayBuildRule::new(xdg.cache.join("yay"))));
    }
    rules.push(Box::new(FlatpakUnusedRule::new(
        Scope::System,
        PathBuf::from(SYSTEM_INSTALLATION),
    )));
    if let Some(xdg) = &xdg {
        rules.push(Box::new(FlatpakUnusedRule::new(
            Scope::User,
            xdg.data.join("flatpak"),
        )));
    }
    rules
}

fn complete(candidates: Vec<Candidate>) -> Discovery {
    Discovery {
        candidates,
        status: DiscoveryStatus::Complete,
    }
}

fn unavailable(reason: &str) -> Discovery {
    Discovery {
        candidates: Vec::new(),
        status: DiscoveryStatus::Unavailable(reason.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_saved_figure_is_the_logical_size() {
        assert_eq!(
            parse_dry_run_saved(
                "==> finished dry run: 1 candidates (disk space saved: 510.4 KiB)\n"
            ),
            Some(522_649)
        );
        assert_eq!(
            parse_dry_run_saved("==> finished dry run: 2 candidates"),
            None
        );
    }

    #[test]
    fn dry_run_parser_reads_counts_and_zero_message() {
        assert_eq!(
            parse_dry_run("==> finished dry run: 4 candidates (disk space saved: 16 KiB)\n"),
            Some(4)
        );
        assert_eq!(
            parse_dry_run("==> no candidate packages found for pruning\n"),
            Some(0)
        );
        assert_eq!(parse_dry_run("something else"), None);
    }
}
