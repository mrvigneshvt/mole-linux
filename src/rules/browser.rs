use std::fs;
use std::path::Path;

use crate::core::proc::ProcessState;

use super::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode,
    RuleContext, candidate,
};

const CHROMIUM_FAMILIES: &[(&str, &str, &str)] = &[
    ("chromium", "chromium", "Chromium cache"),
    ("google-chrome", "chrome", "Chrome cache"),
    ("BraveSoftware/Brave-Browser", "brave", "Brave cache"),
    ("microsoft-edge", "msedge", "Edge cache"),
    ("vivaldi", "vivaldi-bin", "Vivaldi cache"),
];

pub struct BrowserCacheRule;

impl CleanRule for BrowserCacheRule {
    fn id(&self) -> &str {
        "browser-cache"
    }

    fn category(&self) -> Category {
        Category::Browser
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        for (directory, process, label) in CHROMIUM_FAMILIES {
            let path = ctx.xdg.cache.join(directory);
            match browser_candidate(&path, process, label, ctx) {
                Ok(Some(item)) => candidates.push(item),
                Ok(None) => {}
                Err(error) => issues.push(format!("{}: {error}", path.display())),
            }
        }

        let profiles = ctx.xdg.cache.join("mozilla/firefox");
        match fs::read_dir(&profiles) {
            Ok(entries) => {
                for entry in entries {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            issues.push(format!("{}: {error}", profiles.display()));
                            continue;
                        }
                    };
                    let profile = entry.path();
                    let metadata = match fs::symlink_metadata(&profile) {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            issues.push(format!("{}: {error}", profile.display()));
                            continue;
                        }
                    };
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        continue;
                    }
                    let cache = profile.join("cache2");
                    match browser_candidate(&cache, "firefox", "Firefox profile cache", ctx) {
                        Ok(Some(item)) => candidates.push(item),
                        Ok(None) => {}
                        Err(error) => issues.push(format!("{}: {error}", cache.display())),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => issues.push(format!("{}: {error}", profiles.display())),
        }

        let status = if issues.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(issues.join("; "))
        };
        Discovery { candidates, status }
    }
}

// Ported from tw93/Mole lib/clean/user.sh:_clean_firefox_caches_guarded (GPL-3.0):
// cache-only targets and owner-running guard. Linux has no equivalent to the
// macOS browser version bundles handled by lib/clean/user.sh:clean_chrome_old_versions.
fn browser_candidate(
    path: &Path,
    process: &str,
    reason: &str,
    ctx: &RuleContext<'_>,
) -> Result<Option<Candidate>, std::io::Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let state = ctx.owner_state(process);
    let mut item = candidate(path, reason, RecoveryMode::Regenerate, state)?;
    item.owner_check = OwnerCheck::ProcessName(process.to_owned());
    item.selected_by_default = state == ProcessState::NotRunning;
    Ok(Some(item))
}

pub fn registered() -> Box<dyn CleanRule> {
    Box::new(BrowserCacheRule)
}
