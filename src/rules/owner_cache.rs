//! Owner-managed developer caches (M5.1): npm, pnpm, bun, corepack. Each rule
//! reports the directory its owner names and hands the cleanup to the owner's
//! own command through the shared runner.
// Ported from tw93/Mole lib/clean/dev.sh: clean_dev_npm, clean_pnpm_stores,
// clean_corepack_cache, and the bun block (GPL-3.0).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, io};

use crate::adapters::{AdapterEnv, Bun, Corepack, Npm, OwnerAdapter, Pnpm, resolve};
use crate::core::proc::{self, ProcessState};

use super::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerAction, RecoveryMode,
    RuleContext, candidate,
};

pub struct OwnerCacheRule {
    adapter: Box<dyn OwnerAdapter>,
    path: OsString,
    proc_root: PathBuf,
    corepack_home: Option<OsString>,
}

/// One rule per owner tool, using the given PATH, process root, and COREPACK_HOME.
pub fn registered_with(
    path: OsString,
    proc_root: PathBuf,
    corepack_home: Option<OsString>,
) -> Vec<Box<dyn CleanRule>> {
    let adapters: Vec<Box<dyn OwnerAdapter>> = vec![
        Box::new(Npm),
        Box::new(Pnpm),
        Box::new(Bun),
        Box::new(Corepack),
    ];
    adapters
        .into_iter()
        .map(|adapter| {
            Box::new(OwnerCacheRule {
                adapter,
                path: path.clone(),
                proc_root: proc_root.clone(),
                corepack_home: corepack_home.clone(),
            }) as Box<dyn CleanRule>
        })
        .collect()
}

/// The owner rules for this process: real PATH, /proc, and COREPACK_HOME.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    registered_with(
        env::var_os("PATH").unwrap_or_default(),
        PathBuf::from("/proc"),
        env::var_os("COREPACK_HOME"),
    )
}

impl OwnerCacheRule {
    fn env(&self, home: &Path) -> AdapterEnv {
        AdapterEnv {
            home: home.to_path_buf(),
            path: self.path.clone(),
            proc_root: self.proc_root.clone(),
            query_timeout: crate::adapters::QUERY_TIMEOUT,
            clean_timeout: crate::adapters::CLEAN_TIMEOUT,
            corepack_home: self.corepack_home.clone(),
        }
    }

    fn reason(&self) -> String {
        let command = format!(
            "{} {}",
            self.adapter.program(),
            self.adapter.clean_args().join(" ")
        );
        match self.adapter.caveat() {
            Some(note) => format!("{} · owner `{command}` · {note}", self.adapter.label()),
            None => format!("{} · owner `{command}`", self.adapter.label()),
        }
    }
}

impl CleanRule for OwnerCacheRule {
    fn id(&self) -> &str {
        self.adapter.rule_id()
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let env = self.env(ctx.home);
        let results = match resolve(self.adapter.as_ref(), &env) {
            Ok(results) => results,
            Err(refusal) => {
                return Discovery {
                    candidates: Vec::new(),
                    status: DiscoveryStatus::Unavailable(refusal.reason),
                };
            }
        };
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        let mut seen = BTreeSet::new();
        for result in results {
            let resolved = match result {
                Ok(resolved) => resolved,
                Err(refusal) => {
                    issues.push(refusal.reason);
                    continue;
                }
            };
            // Two binaries that name one store get one candidate, so the owner runs once.
            if !seen.insert(resolved.target.clone()) {
                continue;
            }
            match candidate(
                &resolved.target,
                self.reason(),
                RecoveryMode::OwnerNative,
                ProcessState::NotRunning,
            ) {
                Ok(mut item) => {
                    item.owner_action = Some(OwnerAction {
                        program: resolved.binary,
                        args: self.adapter.clean_args(),
                        timeout: env.clean_timeout,
                    });
                    candidates.push(item);
                }
                // Nothing is cached yet, so there is nothing to clean.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => issues.push(format!("{}: {error}", resolved.target.display())),
            }
        }
        // Refusals with nothing to offer mean the owner cannot be used, not that discovery is partial.
        if candidates.is_empty() && !issues.is_empty() {
            return Discovery {
                candidates,
                status: DiscoveryStatus::Unavailable(issues.join("; ")),
            };
        }
        let status = if issues.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(issues.join("; "))
        };
        Discovery { candidates, status }
    }

    /// The owner may have started between discovery and the owner command; refuse then.
    fn final_guard(&self, _candidate: &Candidate, _ctx: &RuleContext<'_>) -> Result<(), String> {
        match proc::is_command_running(&self.proc_root, self.adapter.busy_names()) {
            ProcessState::NotRunning => Ok(()),
            ProcessState::Running => Err(format!(
                "{} started after discovery",
                self.adapter.program()
            )),
            ProcessState::Unknown => Err("owner process state is unknown".to_owned()),
        }
    }
}
