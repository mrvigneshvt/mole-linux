//! Project `__pycache__` cleanup rule (M4.4). Discovery is bounded, and every
//! candidate must pass Git and nested-repository guards both when discovered
//! and again immediately before the sink.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::core::proc::ProcessState;
use crate::core::sink::Identity;
use crate::purge::{discover_project_dirs, has_tracked_files};
use crate::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode,
    RuleContext,
};

pub const RULE_ID: &str = "project-pycache";

// Ported from tw93/Mole lib/clean/caches.sh:scan_project_cache_root (GPL-3.0).
// `.git` is pruned as in upstream; `Library` is dropped because it is macOS-only.
const MAX_DEPTH: usize = 9;
const PRUNED_DIRS: &[&str] = &[
    ".Trash",
    ".git",
    ".svn",
    ".hg",
    ".venv",
    "venv",
    ".pnpm-store",
    ".fvm",
    "DerivedData",
    "Pods",
    "miniconda3",
    "anaconda3",
    "miniforge3",
    "mambaforge",
    "site-packages",
    "node_modules",
    "target",
    ".build",
    ".gradle",
    "Index.noindex",
    "_cacache",
];
// ponytail: fixed walk bounds stand in for upstream's 6-second find timeout; a
// root that exceeds them is reported Partial. Upgrade path: deadline-driven walk.
const MAX_DIRECTORIES: usize = 200_000;
const MAX_TREE_ENTRIES: usize = 100_000;
// ponytail: one Git probe per candidate; upstream batches `git ls-files` per repository.
// Upgrade path: a per-repository tracked index, as in caches.sh:project_cache_build_git_index.
const GIT_BUDGET: Duration = Duration::from_secs(5);

pub struct PyCacheRule;

pub fn registered() -> Box<dyn CleanRule> {
    Box::new(PyCacheRule)
}

impl CleanRule for PyCacheRule {
    fn id(&self) -> &str {
        RULE_ID
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let roots = match discover_project_dirs(ctx.home) {
            Ok(roots) => roots,
            Err(error) => {
                return Discovery {
                    candidates: Vec::new(),
                    status: DiscoveryStatus::Unavailable(format!(
                        "cannot list {}: {error}",
                        ctx.home.display()
                    )),
                };
            }
        };
        let mut candidates = Vec::new();
        let mut incomplete = Vec::new();
        for root in &roots {
            let walk = find_pycache_dirs(root, ctx.cancel);
            incomplete.extend(walk.incomplete);
            let root_name = root.file_name().map_or_else(
                || root.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            for dir in walk.caches {
                match has_direct_bytecode(&dir) {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(reason) => {
                        incomplete.push(reason);
                        continue;
                    }
                }
                match cache_guard(&dir) {
                    // Definitive keeps are not listed; upstream logs them. See KNOWN-GAPS.md.
                    Guard::Kept(_) => {}
                    Guard::Unknown(reason) => incomplete.push(reason),
                    Guard::Clear(facts) => match candidate_for(&dir, &root_name, &facts) {
                        Ok(candidate) => candidates.push(candidate),
                        Err(reason) => incomplete.push(reason),
                    },
                }
            }
        }
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        let status = if incomplete.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(incomplete.join("; "))
        };
        Discovery { candidates, status }
    }

    fn final_guard(&self, candidate: &Candidate, _ctx: &RuleContext<'_>) -> Result<(), String> {
        match cache_guard(&candidate.path) {
            Guard::Clear(_) => Ok(()),
            Guard::Kept(reason) | Guard::Unknown(reason) => Err(reason),
        }
    }
}

#[derive(Default)]
struct Walk {
    caches: Vec<PathBuf>,
    incomplete: Vec<String>,
}

/// Lists `__pycache__` directories under `root`, never following symlinks and
/// never crossing a filesystem boundary. Unreadable paths make the walk incomplete.
fn find_pycache_dirs(root: &Path, cancel: &AtomicBool) -> Walk {
    let mut walk = Walk::default();
    let root_device = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata.dev(),
        Err(error) => {
            walk.incomplete
                .push(format!("cannot stat {}: {error}", root.display()));
            return walk;
        }
    };
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut directories = 0_usize;
    while let Some((dir, depth)) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            walk.incomplete.push("scan cancelled".to_owned());
            break;
        }
        directories += 1;
        if directories > MAX_DIRECTORIES {
            walk.incomplete.push(format!(
                "directory budget exhausted under {}",
                root.display()
            ));
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                walk.incomplete
                    .push(format!("cannot read {}: {error}", dir.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    walk.incomplete
                        .push(format!("cannot read entry in {}: {error}", dir.display()));
                    continue;
                }
            };
            let Ok(file_type) = entry.file_type() else {
                walk.incomplete
                    .push(format!("cannot stat entry in {}", dir.display()));
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if PRUNED_DIRS.iter().any(|pruned| name == *pruned) {
                continue;
            }
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    walk.incomplete
                        .push(format!("cannot stat {}: {error}", path.display()));
                    continue;
                }
            };
            if metadata.dev() != root_device {
                continue;
            }
            if name == "__pycache__" {
                walk.caches.push(path.clone());
            }
            if depth + 1 < MAX_DEPTH {
                pending.push((path, depth + 1));
            }
        }
    }
    walk
}

/// `__pycache__` with no direct `.pyc`/`.pyo` file is empty or already cleaned.
fn has_direct_bytecode(dir: &Path) -> Result<bool, String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for (seen, entry) in entries.enumerate() {
        if seen >= MAX_TREE_ENTRIES {
            return Err(format!("too many entries to verify {}", dir.display()));
        }
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        if matches!(
            entry.path().extension().and_then(|ext| ext.to_str()),
            Some("pyc" | "pyo")
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `Kept` is a definitive refusal, `Unknown` is an unanswered check; both keep the cache.
#[derive(Debug, Eq, PartialEq)]
enum Guard {
    Clear(TreeFacts),
    Kept(String),
    Unknown(String),
}

#[derive(Debug, Default, Eq, PartialEq)]
struct TreeFacts {
    apparent: u64,
    allocated: u64,
}

/// Sizes a cache tree. Any `.git` entry inside it, at any depth, is a nested
/// repository and refuses the cache. Symlinks are sized, never followed.
fn inspect_tree(dir: &Path) -> Guard {
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) => return Guard::Unknown(format!("cannot stat {}: {error}", dir.display())),
    };
    if !metadata.is_dir() {
        return Guard::Unknown(format!("{} is not a real directory", dir.display()));
    }
    let mut facts = TreeFacts::default();
    let mut pending = vec![dir.to_path_buf()];
    let mut entries_seen = 0_usize;
    while let Some(current) = pending.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) => {
                return Guard::Unknown(format!("cannot read {}: {error}", current.display()));
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    return Guard::Unknown(format!("cannot read {}: {error}", current.display()));
                }
            };
            entries_seen += 1;
            if entries_seen > MAX_TREE_ENTRIES {
                return Guard::Unknown(format!("too many entries to verify {}", dir.display()));
            }
            if entry.file_name() == ".git" {
                return Guard::Kept("holds a nested git repository".to_owned());
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    return Guard::Unknown(format!(
                        "cannot stat {}: {error}",
                        entry.path().display()
                    ));
                }
            };
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let sizes = facts.apparent.checked_add(metadata.len()).zip(
                facts
                    .allocated
                    .checked_add(metadata.blocks().saturating_mul(512)),
            );
            let Some((apparent, allocated)) = sizes else {
                return Guard::Unknown(format!("size overflow in {}", dir.display()));
            };
            facts.apparent = apparent;
            facts.allocated = allocated;
        }
    }
    Guard::Clear(facts)
}

/// Shared by discovery and the final guard. Anything but `Clear` keeps the cache.
// Ported from tw93/Mole lib/clean/caches.sh:_project_cache_final_guard,
// _project_cache_holds_nested_repo and project_cache_has_tracked_files (GPL-3.0).
fn cache_guard(dir: &Path) -> Guard {
    let facts = match inspect_tree(dir) {
        Guard::Clear(facts) => facts,
        refusal => return refusal,
    };
    match has_tracked_files(dir, Instant::now() + GIT_BUDGET) {
        Ok(false) => Guard::Clear(facts),
        Ok(true) => Guard::Kept("tracked by git".to_owned()),
        Err(error) => Guard::Unknown(format!("git status unknown: {error}")),
    }
}

fn candidate_for(dir: &Path, root_name: &str, facts: &TreeFacts) -> Result<Candidate, String> {
    let identity = Identity::capture(dir)
        .map_err(|error: io::Error| format!("cannot identify {}: {error}", dir.display()))?;
    Ok(Candidate {
        path: dir.to_path_buf(),
        identity,
        apparent_size: Some(facts.apparent),
        allocated_size: Some(facts.allocated),
        reason: format!("Python bytecode cache in {root_name}"),
        report: None,
        recovery: RecoveryMode::Regenerate,
        selected_by_default: true,
        owner_state: ProcessState::NotRunning,
        owner_check: OwnerCheck::None,
        owner_action: None,
    })
}
