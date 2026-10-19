//! Leaked automation-browser profiles (M5.4): Playwright's temporary Chromium profiles that
//! outlived their run. A profile is offered only when it is at least two hours old and no live
//! process names its path. An unreadable process table keeps every profile.
// Ported from tw93/Mole lib/clean/dev.sh: clean_dev_automation_browsers, profile part (GPL-3.0).
// The upstream process-termination part is not ported; see KNOWN-GAPS.md.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::core::proc::{ProcessState, is_argv_containing};

use super::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, RecoveryMode, RuleContext,
    candidate,
};

const PROFILE_PREFIX: &str = "playwright_chromiumdev_profile-";
const STALE_AFTER: Duration = Duration::from_secs(2 * 60 * 60);

/// Where profiles live and where live processes are read from. Tests use temporary roots.
#[derive(Clone, Debug)]
pub struct AutomationRuleEnv {
    pub tmp_root: PathBuf,
    pub proc_root: PathBuf,
}

impl AutomationRuleEnv {
    /// `TMPDIR` when it is an absolute path, otherwise `/tmp`, as Playwright's `os.tmpdir()` picks.
    pub fn from_process() -> Self {
        let tmp_root = env::var_os("TMPDIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            tmp_root,
            proc_root: PathBuf::from("/proc"),
        }
    }
}

// ponytail: upstream also TERMs then KILLs leaked automation processes (ps scan with PID/start-time rebind). Ceiling: profiles only; leaked processes keep running. Upgrade: a process-stop unit with identity recheck, after an owner decision on signalling. Logged in KNOWN-GAPS.md.
pub struct AutomationProfileRule {
    env: AutomationRuleEnv,
}

impl CleanRule for AutomationProfileRule {
    fn id(&self) -> &str {
        "automation-profiles"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        let entries = match fs::read_dir(&self.env.tmp_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Discovery {
                    candidates,
                    status: DiscoveryStatus::Complete,
                };
            }
            Err(error) => {
                return Discovery {
                    candidates,
                    status: DiscoveryStatus::Partial(format!(
                        "{}: {error}",
                        self.env.tmp_root.display()
                    )),
                };
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    issues.push(format!("{}: {error}", self.env.tmp_root.display()));
                    continue;
                }
            };
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(PROFILE_PREFIX) {
                continue;
            }
            match self.offer(&path, ctx) {
                Ok(Some(item)) => candidates.push(item),
                Ok(None) => {}
                Err(reason) => issues.push(reason),
            }
        }
        let status = if issues.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(issues.join("; "))
        };
        Discovery { candidates, status }
    }
}

impl AutomationProfileRule {
    /// `Ok(None)` keeps the profile: symlinks, fresh profiles, and profiles a live process uses.
    /// `Err` means the profile could not be judged and is reported.
    fn offer(&self, path: &Path, ctx: &RuleContext<'_>) -> Result<Option<Candidate>, String> {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Ok(None);
        }
        let modified = metadata
            .modified()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        // A future mtime cannot prove the profile is stale, so it is kept.
        let Ok(age) = ctx.now.duration_since(modified) else {
            return Ok(None);
        };
        if age < STALE_AFTER {
            return Ok(None);
        }
        let needle = path.to_string_lossy();
        match is_argv_containing(&self.env.proc_root, &needle) {
            ProcessState::NotRunning => {}
            ProcessState::Running => return Ok(None),
            ProcessState::Unknown => {
                return Err(format!("{}: process state unknown", path.display()));
            }
        }
        candidate(
            path,
            "Leaked automation browser profile · no live process uses it",
            RecoveryMode::Regenerate,
            ProcessState::NotRunning,
        )
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// The automation-profile rule for the given roots.
pub fn registered_with(env: AutomationRuleEnv) -> Box<dyn CleanRule> {
    Box::new(AutomationProfileRule { env })
}

/// The automation-profile rule for this process.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    vec![registered_with(AutomationRuleEnv::from_process())]
}
