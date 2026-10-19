//! Owner-managed developer caches (M5.1, M5.2): npm, pnpm, bun, corepack, uv, pip3,
//! conda. Each rule reports the directory its owner names and hands the cleanup to
//! the owner's own command through the shared runner.
// Ported from tw93/Mole lib/clean/dev.sh: clean_dev_npm, clean_pnpm_stores,
// clean_corepack_cache, the bun block, clean_uv_cache, clean_dev_python (pip3),
// and clean_conda_metadata_caches (GPL-3.0).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, io};

use crate::adapters::{
    AdapterEnv, Bun, Conda, Corepack, Npm, OwnerAdapter, Pip, Pnpm, Uv, resolve,
};
use crate::core::proc::{self, ProcessState};

use super::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerAction, RecoveryMode,
    RuleContext, candidate,
};

pub struct OwnerCacheRule {
    adapter: Box<dyn OwnerAdapter>,
    env: OwnerRuleEnv,
}

/// What the owner rules read from the process: PATH, /proc, and the tool environment variables.
#[derive(Clone, Debug, Default)]
pub struct OwnerRuleEnv {
    pub path: OsString,
    pub proc_root: PathBuf,
    pub corepack_home: Option<OsString>,
    pub bun_install_cache_dir: Option<OsString>,
    pub bun_install: Option<OsString>,
}

impl OwnerRuleEnv {
    pub fn from_process() -> Self {
        Self {
            path: env::var_os("PATH").unwrap_or_default(),
            proc_root: PathBuf::from("/proc"),
            corepack_home: env::var_os("COREPACK_HOME"),
            bun_install_cache_dir: env::var_os("BUN_INSTALL_CACHE_DIR"),
            bun_install: env::var_os("BUN_INSTALL"),
        }
    }
}

/// One rule per owner tool, using the given environment.
pub fn registered_with(env: OwnerRuleEnv) -> Vec<Box<dyn CleanRule>> {
    let adapters: Vec<Box<dyn OwnerAdapter>> = vec![
        Box::new(Npm),
        Box::new(Pnpm),
        Box::new(Bun),
        Box::new(Corepack),
        Box::new(Uv),
        Box::new(Pip),
        Box::new(Conda),
    ];
    adapters
        .into_iter()
        .map(|adapter| {
            Box::new(OwnerCacheRule {
                adapter,
                env: env.clone(),
            }) as Box<dyn CleanRule>
        })
        .collect()
}

/// The owner rules for this process.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    registered_with(OwnerRuleEnv::from_process())
}

impl OwnerCacheRule {
    fn env(&self, home: &Path) -> AdapterEnv {
        AdapterEnv {
            home: home.to_path_buf(),
            path: self.env.path.clone(),
            proc_root: self.env.proc_root.clone(),
            query_timeout: crate::adapters::QUERY_TIMEOUT,
            clean_timeout: crate::adapters::CLEAN_TIMEOUT,
            corepack_home: self.env.corepack_home.clone(),
            bun_install_cache_dir: self.env.bun_install_cache_dir.clone(),
            bun_install: self.env.bun_install.clone(),
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
        let mut owner_refused = false;
        let mut protected = Vec::new();
        for result in results {
            let resolved = match result {
                Ok(resolved) => resolved,
                Err(refusal) => {
                    owner_refused = true;
                    issues.push(refusal.reason);
                    continue;
                }
            };
            // A whitelisted directory anywhere under the owner's cleanup keeps the owner command from running.
            if let Some(kept) = resolved
                .guarded
                .iter()
                .find(|path| ctx.whitelist.matches(path))
            {
                protected.push(format!(
                    "{} is whitelisted; owner command skipped",
                    kept.display()
                ));
                continue;
            }
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
        // An owner that is installed and idle but cannot run its cache command falls back to
        // its cache directory, removed through the sink (Trash), as the owner would have done.
        if owner_refused && let Some(dir) = self.adapter.fallback_dir(&env) {
            let reason = format!(
                "{} · owner cannot run; sink fallback to Trash",
                self.adapter.label()
            );
            match candidate(
                &dir,
                reason,
                RecoveryMode::Regenerate,
                ProcessState::NotRunning,
            ) {
                Ok(item) => candidates.push(item),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => issues.push(format!("{}: {error}", dir.display())),
            }
        }
        // Refusals with nothing to offer mean the owner cannot be used, not that discovery is partial.
        if candidates.is_empty() && !issues.is_empty() {
            return Discovery {
                candidates,
                status: DiscoveryStatus::Unavailable(issues.join("; ")),
            };
        }
        issues.extend(protected);
        let status = if issues.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(issues.join("; "))
        };
        Discovery { candidates, status }
    }

    /// The owner may have started between discovery and the owner command; refuse then.
    fn final_guard(&self, _candidate: &Candidate, _ctx: &RuleContext<'_>) -> Result<(), String> {
        match proc::is_command_running(&self.env.proc_root, self.adapter.busy_names()) {
            ProcessState::NotRunning => Ok(()),
            ProcessState::Running => Err(format!(
                "{} started after discovery",
                self.adapter.program()
            )),
            ProcessState::Unknown => Err("owner process state is unknown".to_owned()),
        }
    }
}
