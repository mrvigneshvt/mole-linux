//! Python tool caches (M5.2): PyInstaller's bincache, and the tool cache directories
//! upstream clears with `safe_clean`. Every entry is a regenerable candidate that the
//! sink moves to Trash; PyInstaller is gated on its own process.
// Ported from tw93/Mole lib/clean/dev.sh: clean_dev_python, clean_pyinstaller_bincache
// (GPL-3.0). Upstream's bincache root is macOS-only; the Linux root is PyInstaller's
// documented `$XDG_DATA_HOME/pyinstaller` (default `~/.local/share/pyinstaller`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::proc::ProcessState;

use super::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode,
    RuleContext, candidate,
};

const PYINSTALLER_PROCESS: &str = "pyinstaller";

/// Home-relative tool cache directories whose children upstream removes. Each entry is
/// (rule id, directory, label). Jupyter runtime files belong to live kernels and are not listed.
const TOOL_CACHES: &[(&str, &str, &str)] = &[
    ("pyenv-cache", ".pyenv/cache", "pyenv cache"),
    ("poetry-cache", ".cache/poetry", "Poetry cache"),
    ("ruff-cache", ".cache/ruff", "Ruff cache"),
    ("mypy-cache", ".cache/mypy", "MyPy cache"),
    ("pytest-cache", ".pytest_cache", "Pytest cache"),
];

pub struct PyInstallerRule;

// ponytail: PYINSTALLER_CONFIG_DIR is not read, so a relocated cache is not found. Ceiling: the default root only.
// Upgrade: honor the variable when it is an absolute path. Logged in KNOWN-GAPS.md.
// ponytail: the owner gate matches the process name (comm). `python -m PyInstaller` runs as `python`, so it is not seen.
// Upgrade: match argv through core::proc::is_command_running. Logged in KNOWN-GAPS.md.
impl CleanRule for PyInstallerRule {
    fn id(&self) -> &str {
        "pyinstaller"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let root = ctx.xdg.data.join("pyinstaller");
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                // A replaced root could point the sink at another directory; keep it out.
                issues.push(format!("{} is a symlink; not cleaned", root.display()));
            }
            Ok(metadata) if metadata.is_dir() => {
                let state = ctx.owner_state(PYINSTALLER_PROCESS);
                match children(&root) {
                    Ok(entries) => {
                        for entry in entries {
                            let is_bincache = entry
                                .file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.starts_with("bincache"));
                            if !is_bincache {
                                continue;
                            }
                            let check = OwnerCheck::ProcessName(PYINSTALLER_PROCESS.to_owned());
                            match candidate_for(&entry, "PyInstaller binary cache", state, check) {
                                Ok(Some(item)) => candidates.push(item),
                                Ok(None) => {}
                                Err(error) => {
                                    issues.push(format!("{}: {error}", entry.display()));
                                }
                            }
                        }
                    }
                    Err(error) => issues.push(format!("{}: {error}", root.display())),
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => issues.push(format!("{}: {error}", root.display())),
        }
        Discovery {
            candidates,
            status: status_of(issues),
        }
    }
}

/// The owner-gated, sink-backed children of the Python tool caches in TOOL_CACHES.
pub struct ToolCacheRule {
    id: &'static str,
    directory: &'static str,
    label: &'static str,
}

impl CleanRule for ToolCacheRule {
    fn id(&self) -> &str {
        self.id
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let root = ctx.home.join(self.directory);
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                match children(&root) {
                    Ok(entries) => {
                        for entry in entries {
                            let check = OwnerCheck::None;
                            match candidate_for(&entry, self.label, ProcessState::NotRunning, check)
                            {
                                Ok(Some(item)) => candidates.push(item),
                                Ok(None) => {}
                                Err(error) => {
                                    issues.push(format!("{}: {error}", entry.display()));
                                }
                            }
                        }
                    }
                    Err(error) => issues.push(format!("{}: {error}", root.display())),
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => issues.push(format!("{}: {error}", root.display())),
        }
        Discovery {
            candidates,
            status: status_of(issues),
        }
    }
}

/// Every Python rule for this process: PyInstaller first, then the tool caches in TOOL_CACHES order.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    let mut rules: Vec<Box<dyn CleanRule>> = vec![Box::new(PyInstallerRule)];
    for (id, directory, label) in TOOL_CACHES {
        rules.push(Box::new(ToolCacheRule {
            id,
            directory,
            label,
        }));
    }
    rules
}

/// Child paths of a directory, in directory order.
fn children(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        paths.push(entry?.path());
    }
    Ok(paths)
}

/// A regenerable candidate for one child, gated by `check`. Symlinks are never followed or removed: `None`.
fn candidate_for(
    path: &Path,
    label: &str,
    state: ProcessState,
    check: OwnerCheck,
) -> Result<Option<Candidate>, io::Error> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(None);
    }
    let mut item = candidate(path, label, RecoveryMode::Regenerate, state)?;
    item.owner_check = check;
    Ok(Some(item))
}

fn status_of(issues: Vec<String>) -> DiscoveryStatus {
    if issues.is_empty() {
        DiscoveryStatus::Complete
    } else {
        DiscoveryStatus::Partial(issues.join("; "))
    }
}
