//! Toolchain caches cleaned through the sink (M5.3): Cargo's registry download
//! cache and Gradle's caches, notifications, daemon, and worker state. Neither
//! tool has a cache-clean command for these trees, so each entry goes to Trash
//! through the sink. The owner process gate runs at discovery and again before
//! every sink call.
// Ported from tw93/Mole lib/clean/dev.sh: clean_dev_rust, rust_cache_root_physical_path,
// resolve_tool_home, and clean_dev_jvm's Gradle groups (GPL-3.0).
// Dropped on purpose: registry/src (cargo builds from it), git checkouts, rustup
// downloads, and the Maven and Ivy stores (upstream never deletes those either).

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::proc::{self, ProcessState};
use crate::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, RecoveryMode, RuleContext,
    candidate,
};

use super::owner_cache::OwnerRuleEnv;

/// Process names that mean cargo is reading or writing the registry.
const CARGO_BUSY: &[&str] = &[
    "cargo",
    "rustc",
    "rustdoc",
    "clippy-driver",
    "cargo-nextest",
];
/// Gradle daemon markers matched anywhere in argv, as upstream's `pgrep -f` probe does.
const GRADLE_BUSY: &[&str] = &["org.gradle.launcher.daemon", "GradleDaemon"];

pub struct CargoRegistryRule {
    env: OwnerRuleEnv,
}

pub struct GradleCacheRule {
    env: OwnerRuleEnv,
}

/// The toolchain sink rules for this process environment.
pub fn registered_with(env: OwnerRuleEnv) -> Vec<Box<dyn CleanRule>> {
    vec![
        Box::new(CargoRegistryRule { env: env.clone() }),
        Box::new(GradleCacheRule { env }),
    ]
}

/// The toolchain sink rules for this process.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    registered_with(OwnerRuleEnv::from_process())
}

impl CargoRegistryRule {
    fn cargo_home(&self, home: &Path) -> PathBuf {
        tool_home(self.env.cargo_home.as_ref(), home.join(".cargo"))
    }
}

impl CleanRule for CargoRegistryRule {
    fn id(&self) -> &str {
        "cargo"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let cargo_home = self.cargo_home(ctx.home);
        match physical_cache_root(&cargo_home) {
            Ok(Some(_)) => {}
            Ok(None) => return complete_empty(),
            Err(reason) => return unavailable(reason),
        }
        let cache = cargo_home.join("registry/cache");
        let children = match visible_children(&cache) {
            Ok(children) => children,
            Err(error) => return unavailable(format!("cannot list {}: {error}", cache.display())),
        };
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        for child in children {
            push_candidate(&child, "Rust cargo cache", &mut candidates, &mut issues);
        }
        if !candidates.is_empty()
            && let Some(reason) = gate_reason(
                proc::is_command_running(&self.env.proc_root, CARGO_BUSY),
                "cargo",
            )
        {
            return unavailable(reason);
        }
        finish(candidates, issues)
    }

    /// Cargo may start between discovery and the sink; the cache must still be the physical one.
    fn final_guard(&self, candidate: &Candidate, ctx: &RuleContext<'_>) -> Result<(), String> {
        let cargo_home = self.cargo_home(ctx.home);
        let root = physical_cache_root(&cargo_home)?
            .ok_or_else(|| "Cargo registry cache disappeared".to_owned())?;
        let parent = candidate
            .path
            .parent()
            .ok_or_else(|| "cache entry has no parent".to_owned())?;
        let physical_parent = fs::canonicalize(parent)
            .map_err(|error| format!("cannot resolve {}: {error}", parent.display()))?;
        if physical_parent != root {
            return Err("cache entry moved out of the Cargo registry cache".to_owned());
        }
        match gate_reason(
            proc::is_command_running(&self.env.proc_root, CARGO_BUSY),
            "cargo",
        ) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

// ponytail: upstream bounds the listing with a scan deadline and skips whitelisted entries before the protection probe.
// Ceiling: the listing is unbounded and whitelist filtering is left to the runner. Upgrade: a deadline-driven listing
// as in clean_dev_jvm. Logged in KNOWN-GAPS.md.
impl CleanRule for GradleCacheRule {
    fn id(&self) -> &str {
        "gradle"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let root = ctx.home.join(".gradle");
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Discovery {
                    candidates: Vec::new(),
                    status: DiscoveryStatus::Partial(format!(
                        "{} is not a plain directory; not followed",
                        root.display()
                    )),
                };
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return complete_empty(),
            Err(error) => return unavailable(format!("cannot stat {}: {error}", root.display())),
        }

        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        let groups = [
            ("notifications", "Gradle notifications cache"),
            ("daemon", "Gradle daemon/workers"),
            ("workers", "Gradle daemon/workers"),
        ];
        for (name, reason) in groups {
            collect_children(&root.join(name), reason, &mut candidates, &mut issues);
        }
        let caches = root.join("caches");
        match visible_children(&caches) {
            Ok(children) => {
                for build in children.into_iter().filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("build-cache-"))
                }) {
                    collect_children(&build, "Gradle build cache", &mut candidates, &mut issues);
                }
            }
            Err(error) => issues.push(format!("cannot list {}: {error}", caches.display())),
        }

        if !candidates.is_empty()
            && let Some(reason) = gate_reason(
                proc::is_cmdline_containing(&self.env.proc_root, GRADLE_BUSY),
                "Gradle daemon",
            )
        {
            return unavailable(reason);
        }
        finish(candidates, issues)
    }

    fn final_guard(&self, _candidate: &Candidate, _ctx: &RuleContext<'_>) -> Result<(), String> {
        match gate_reason(
            proc::is_cmdline_containing(&self.env.proc_root, GRADLE_BUSY),
            "Gradle daemon",
        ) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

/// Resolves a tool home from an optional env value: only an absolute value without `..`
/// components or control characters is used; anything else falls back to the default.
// Ported from tw93/Mole lib/clean/dev.sh:resolve_tool_home (GPL-3.0).
fn tool_home(value: Option<&OsString>, default: PathBuf) -> PathBuf {
    value
        .and_then(|value| value.to_str())
        .filter(|text| {
            text.starts_with('/')
                && !text.chars().any(char::is_control)
                && !text.contains("/../")
                && !text.ends_with("/..")
        })
        .map(|text| PathBuf::from(text.trim_end_matches('/')))
        .filter(|path| path.parent().is_some())
        .unwrap_or(default)
}

/// The physical registry cache, when it is a real directory inside the physical Cargo home.
/// `Ok(None)` means no cache exists yet. A symlinked cache, or one that resolves outside the home, is refused.
// Ported from tw93/Mole lib/clean/dev.sh:rust_cache_root_physical_path (GPL-3.0).
fn physical_cache_root(cargo_home: &Path) -> Result<Option<PathBuf>, String> {
    let cache = cargo_home.join("registry/cache");
    let metadata = match fs::symlink_metadata(&cache) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot stat {}: {error}", cache.display())),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "{} is a symlink; cache path leaves CARGO_HOME",
            cache.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", cache.display()));
    }
    let physical_home = fs::canonicalize(cargo_home)
        .map_err(|error| format!("cannot resolve {}: {error}", cargo_home.display()))?;
    let physical_root = fs::canonicalize(&cache)
        .map_err(|error| format!("cannot resolve {}: {error}", cache.display()))?;
    if physical_home == Path::new("/")
        || physical_root == physical_home
        || !physical_root.starts_with(&physical_home)
    {
        return Err(format!(
            "{} leaves CARGO_HOME ({})",
            cache.display(),
            physical_home.display()
        ));
    }
    Ok(Some(physical_root))
}

/// Gate result for one owner family: `None` when the sink may proceed.
fn gate_reason(state: ProcessState, what: &str) -> Option<String> {
    match state {
        ProcessState::NotRunning => None,
        ProcessState::Running => Some(format!("{what} is running")),
        ProcessState::Unknown => Some(format!("{what} process state is unknown")),
    }
}

/// Direct children of `dir` that are not hidden, as the upstream `dir/*` glob lists them.
/// A missing directory has no children.
fn visible_children(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut children = Vec::new();
    for entry in entries {
        let entry = entry?;
        let hidden = entry.file_name().as_encoded_bytes().first() == Some(&b'.');
        if !hidden {
            children.push(entry.path());
        }
    }
    children.sort();
    Ok(children)
}

fn collect_children(
    dir: &Path,
    reason: &str,
    candidates: &mut Vec<Candidate>,
    issues: &mut Vec<String>,
) {
    match visible_children(dir) {
        Ok(children) => {
            for child in children {
                push_candidate(&child, reason, candidates, issues);
            }
        }
        Err(error) => issues.push(format!("cannot list {}: {error}", dir.display())),
    }
}

/// Adds one sink candidate. Symlinks are left in place: the sink would remove the link itself.
fn push_candidate(
    path: &Path,
    reason: &str,
    candidates: &mut Vec<Candidate>,
    issues: &mut Vec<String>,
) {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            issues.push(format!("{} is a symlink; left in place", path.display()));
            return;
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            issues.push(format!("{}: {error}", path.display()));
            return;
        }
    }
    match candidate(
        path,
        reason,
        RecoveryMode::Regenerate,
        ProcessState::NotRunning,
    ) {
        Ok(item) => candidates.push(item),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => issues.push(format!("{}: {error}", path.display())),
    }
}

fn finish(candidates: Vec<Candidate>, issues: Vec<String>) -> Discovery {
    if candidates.is_empty() && !issues.is_empty() {
        return unavailable(issues.join("; "));
    }
    let status = if issues.is_empty() {
        DiscoveryStatus::Complete
    } else {
        DiscoveryStatus::Partial(issues.join("; "))
    };
    Discovery { candidates, status }
}

fn complete_empty() -> Discovery {
    Discovery {
        candidates: Vec::new(),
        status: DiscoveryStatus::Complete,
    }
}

fn unavailable(reason: String) -> Discovery {
    Discovery {
        candidates: Vec::new(),
        status: DiscoveryStatus::Unavailable(reason),
    }
}
