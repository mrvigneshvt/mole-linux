use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use crate::core::paths::XdgPaths;
use crate::core::proc::ProcessState;
use crate::core::sink::Identity;
use crate::core::whitelist::Whitelist;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Category {
    User,
    Browser,
    Development,
    System,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Browser => "browser",
            Self::Development => "development",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryMode {
    Regenerate,
    Trash,
    OwnerNative,
    Irreversible,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerCheck {
    None,
    ProcessName(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub identity: Identity,
    pub apparent_size: Option<u64>,
    pub allocated_size: Option<u64>,
    pub reason: String,
    pub recovery: RecoveryMode,
    pub selected_by_default: bool,
    pub owner_state: ProcessState,
    pub owner_check: OwnerCheck,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryStatus {
    Complete,
    Partial(String),
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    pub status: DiscoveryStatus,
}

pub struct RuleContext<'a> {
    pub home: &'a Path,
    pub xdg: &'a XdgPaths,
    pub whitelist: &'a Whitelist,
    pub proc_probe: &'a dyn Fn(&str) -> ProcessState,
    pub now: SystemTime,
    pub cancel: &'a AtomicBool,
}

impl RuleContext<'_> {
    pub fn owner_state(&self, process_name: &str) -> ProcessState {
        (self.proc_probe)(process_name)
    }

    pub fn check_owner(&self, check: &OwnerCheck) -> ProcessState {
        match check {
            OwnerCheck::None => ProcessState::NotRunning,
            OwnerCheck::ProcessName(name) => self.owner_state(name),
        }
    }
}

pub trait CleanRule: Send + Sync {
    fn id(&self) -> &str;
    fn category(&self) -> Category;
    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery;
    /// Re-checks a candidate just before the sink; `Err` keeps it and gives the reason.
    fn final_guard(&self, _candidate: &Candidate, _ctx: &RuleContext<'_>) -> Result<(), String> {
        Ok(())
    }
}

/// Owner-dependent rules must preserve `Unknown`; it cannot authorize cleanup.
pub fn owner_candidate_state(ctx: &RuleContext<'_>, process_name: &str) -> (ProcessState, bool) {
    let state = ctx.owner_state(process_name);
    (state, state == ProcessState::NotRunning)
}

pub fn candidate(
    path: &Path,
    reason: impl Into<String>,
    recovery: RecoveryMode,
    owner_state: ProcessState,
) -> Result<Candidate, std::io::Error> {
    let identity = Identity::capture(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    let (apparent_size, allocated_size) = if metadata.is_file() {
        use std::os::unix::fs::MetadataExt;
        (
            Some(metadata.len()),
            Some(metadata.blocks().saturating_mul(512)),
        )
    } else {
        (None, None)
    };
    Ok(Candidate {
        path: path.to_path_buf(),
        identity,
        apparent_size,
        allocated_size,
        reason: reason.into(),
        recovery,
        selected_by_default: owner_state == ProcessState::NotRunning,
        owner_state,
        owner_check: OwnerCheck::None,
    })
}

/// Concrete Linux cleanup targets are registered by M4.2 and later units.
pub fn catalog() -> Vec<Box<dyn CleanRule>> {
    vec![super::browser::registered()]
}
