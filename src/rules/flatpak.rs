//! Flatpak unused-ref rules (M6.5, Linux-native: no upstream Mole counterpart). Preview is an
//! unprivileged `flatpak list`; apply is `flatpak uninstall --unused` through `OwnerRunner`.
//! `--delete-data` is never passed, so app data under `~/.var/app` is untouched.

use std::path::PathBuf;

use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRunner};
use crate::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode,
    RuleContext, candidate, owner_candidate_state,
};

pub const SYSTEM_INSTALLATION: &str = "/var/lib/flatpak";

/// Which flatpak installation the rule manages. System needs sudo; user does not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    System,
    User,
}

impl Scope {
    fn flag(self) -> &'static str {
        match self {
            Self::System => "--system",
            Self::User => "--user",
        }
    }

    fn needs_root(self) -> bool {
        self == Self::System
    }
}

pub struct FlatpakUnusedRule {
    scope: Scope,
    installation: PathBuf,
}

impl FlatpakUnusedRule {
    pub fn new(scope: Scope, installation: PathBuf) -> Self {
        Self {
            scope,
            installation,
        }
    }
}

impl CleanRule for FlatpakUnusedRule {
    fn id(&self) -> &str {
        match self.scope {
            Scope::System => "flatpak-unused-system",
            Scope::User => "flatpak-unused-user",
        }
    }

    fn category(&self) -> Category {
        Category::System
    }

    /// Listing needs the owner runner to find `flatpak`, so this rule cannot discover without one.
    fn discover(&self, _ctx: &RuleContext<'_>) -> Discovery {
        unavailable("flatpak list needs the owner runner")
    }

    // flatpak offers no dry run for `--unused`, so the preview counts installed refs. The unused
    // subset is chosen by flatpak at apply time; the candidate reason says so.
    fn discover_with_owner(&self, ctx: &RuleContext<'_>, owner: &OwnerRunner) -> Discovery {
        let query = OwnerCommand::new(
            OwnerProgram::Flatpak,
            ["list", self.scope.flag(), "--runtime", "--columns=ref"],
            false,
        );
        let output = match owner.query(&query) {
            Ok(output) => output,
            Err(run) => {
                return unavailable(
                    &run.reason
                        .unwrap_or_else(|| "flatpak list failed".to_owned()),
                );
            }
        };
        let Some(count) = parse_refs(&output) else {
            return unavailable("unrecognized flatpak list output");
        };
        if count == 0 {
            return complete(Vec::new());
        }
        let (state, _) = owner_candidate_state(ctx, "flatpak");
        let reason = format!(
            "{count} installed runtime ref(s); flatpak removes only the unused ones at apply"
        );
        match candidate(&self.installation, reason, RecoveryMode::OwnerNative, state) {
            Ok(mut found) => {
                found.owner_check = OwnerCheck::ProcessName("flatpak".to_owned());
                complete(vec![found])
            }
            Err(error) => unavailable(&format!("{}: {error}", self.installation.display())),
        }
    }

    fn owner_command(&self, _candidate: &Candidate) -> Option<OwnerCommand> {
        // ponytail: apply re-runs `flatpak uninstall --unused`, so flatpak picks the set at apply time, not the refs counted in the preview. Upgrade path: a preview that lists the unused refs once flatpak exposes a dry run. Logged in KNOWN-GAPS.md.
        Some(OwnerCommand::new(
            OwnerProgram::Flatpak,
            ["uninstall", self.scope.flag(), "--unused", "--assumeyes"],
            self.scope.needs_root(),
        ))
    }
}

/// One installed ref per line (`kind/id/arch/branch`). The only tolerated non-ref line is the
/// column header `ref`, and only as the first line (the convention in
/// `commands/uninstall.rs::parse_flatpak_list`). Any other line is unknown, never zero.
fn parse_refs(output: &str) -> Option<u64> {
    let mut count = 0_u64;
    for (index, line) in output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.contains('/') {
            count = count.saturating_add(1);
        } else if index != 0 || !line.eq_ignore_ascii_case("ref") {
            return None;
        }
    }
    Some(count)
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
