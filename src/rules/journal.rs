//! System rule for journald (M6.3). Preview reads journal disk usage and the archived/active
//! split through `OwnerRunner`; apply runs `journalctl --vacuum-size` (archived files only).

use std::path::PathBuf;

use crate::commands::journal::{self, Inventory, Retention, Usage};
use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRunner};
use crate::core::proc::ProcessState;
use crate::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, RecoveryMode, RuleContext,
    candidate,
};

/// Archived journal files are trimmed to this size. Fixed for now: there is no flag for it yet.
pub const DEFAULT_MAX_MIB: u32 = 512;

pub struct JournalRule {
    journal_dir: Result<PathBuf, String>,
}

impl JournalRule {
    pub fn with_dir(journal_dir: PathBuf) -> Self {
        Self {
            journal_dir: Ok(journal_dir),
        }
    }

    /// Resolves `/var/log/journal/<machine-id>` at catalog time; a failure is reported as unavailable.
    pub fn from_system() -> Self {
        Self {
            journal_dir: journal::journal_dir().map_err(|error| error.to_string()),
        }
    }
}

pub fn registered() -> Box<dyn CleanRule> {
    Box::new(JournalRule::from_system())
}

impl CleanRule for JournalRule {
    fn id(&self) -> &str {
        "journal"
    }

    fn category(&self) -> Category {
        Category::System
    }

    /// Previewing needs `journalctl`, so this rule cannot discover without an owner runner.
    fn discover(&self, _ctx: &RuleContext<'_>) -> Discovery {
        unavailable("journal preview needs the owner runner")
    }

    fn discover_with_owner(&self, _ctx: &RuleContext<'_>, owner: &OwnerRunner) -> Discovery {
        let dir = match &self.journal_dir {
            Ok(dir) => dir,
            Err(reason) => return unavailable(&format!("journal directory: {reason}")),
        };
        let value = match journal::disk_usage(owner) {
            Usage::Reported(value) => value,
            Usage::Unknown(reason) => return unavailable(&reason),
        };
        let found = match journal::inventory(dir) {
            Ok(found) => found,
            Err(error) => return unavailable(&format!("{}: {error}", dir.display())),
        };
        if found.archived == 0 {
            return complete(Vec::new());
        }
        match candidate(
            dir,
            reason(&value, &found),
            RecoveryMode::OwnerNative,
            ProcessState::NotRunning,
        ) {
            Ok(found) => complete(vec![found]),
            Err(error) => unavailable(&format!("{}: {error}", dir.display())),
        }
    }

    // ponytail: the preview lists counts, not files; journald picks the removal set at apply time. Upgrade path: project the archived set from `inventory` with sizes and times. Logged in KNOWN-GAPS.md.
    fn owner_command(&self, _candidate: &Candidate) -> Option<OwnerCommand> {
        Some(OwnerCommand::new(
            OwnerProgram::Journalctl,
            [Retention::SizeMiB(DEFAULT_MAX_MIB).arg()],
            true,
        ))
    }
}

fn reason(value: &str, found: &Inventory) -> String {
    format!(
        "journal disk usage {value}; {} archived file(s) trimmed to {DEFAULT_MAX_MIB} MiB (default); {} active file(s) kept, journald never vacuums them",
        found.archived,
        found.active.len()
    )
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
