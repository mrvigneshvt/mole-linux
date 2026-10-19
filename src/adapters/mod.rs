//! Owner-tool adapters for developer caches (M5.1). Each tool runs its own cache
//! command by absolute path; this module never deletes files itself.

mod bun;
mod corepack;
mod npm;
mod pnpm;

use std::collections::BTreeSet;
use std::env::split_paths;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::core::outcome::Outcome;
use crate::core::privilege::{RunMode, is_executable_file};
use crate::core::proc::{self, ProcessState};
use crate::core::whitelist::Whitelist;

pub use bun::Bun;
pub use corepack::Corepack;
pub use npm::Npm;
pub use pnpm::Pnpm;

/// Owner tools that show a download prompt would hang a section until its timeout.
const NO_DOWNLOAD_PROMPT: (&str, &str) = ("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0");
/// Owner stdout is read up to this many bytes; a larger reply is treated as a failure.
const OUTPUT_LIMIT: u64 = 64 * 1024;
const POLL: Duration = Duration::from_millis(10);
/// Probes and queries. Upstream QUICK_DETECT is 2s, but `pnpm --version` takes ~3.1s on the reference
/// host (measured), so this uses upstream's PKG_LIST budget (10s). PKG_CLEANUP (20s) bounds cleanup.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
pub const CLEAN_TIMEOUT: Duration = Duration::from_secs(20);

/// Everything an adapter may read. Tests build it from temp directories; no ambient state.
#[derive(Clone, Debug)]
pub struct AdapterEnv {
    pub home: PathBuf,
    pub path: OsString,
    pub proc_root: PathBuf,
    pub whitelist: Whitelist,
    pub query_timeout: Duration,
    pub clean_timeout: Duration,
    pub corepack_home: Option<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub label: &'static str,
    pub outcome: Outcome,
    pub program: Option<PathBuf>,
    pub target: Option<PathBuf>,
    pub reason: String,
}

/// Why an adapter cannot name a target. The outcome says whether the tool is absent or the path is refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub outcome: Outcome,
    pub reason: String,
}

pub trait OwnerAdapter: Sync {
    /// Section label shown to the user, as in upstream output.
    fn label(&self) -> &'static str;
    /// Owner program basename, resolved only through absolute PATH entries.
    fn program(&self) -> &'static str;
    /// Basenames in argv[0] or argv[1] that mean the owner is busy (fail closed).
    fn busy_names(&self) -> &'static [&'static str];
    /// Installed binaries to consider, in preference order.
    fn binaries(&self, env: &AdapterEnv) -> Result<Vec<PathBuf>, String> {
        Ok(resolve_on_path(self.program(), &env.path)
            .into_iter()
            .collect())
    }
    /// The directory the owner will clean, as the owner itself reports it.
    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal>;
    /// Owner command arguments that perform the cleanup.
    fn clean_args(&self) -> &'static [&'static str];
}

/// Runs one adapter. Dry run only queries and reports; apply runs the owner command once per distinct target.
pub fn run_adapter(adapter: &dyn OwnerAdapter, env: &AdapterEnv, mode: RunMode) -> Vec<Report> {
    let label = adapter.label();
    let binaries = match adapter.binaries(env) {
        Ok(binaries) => binaries,
        Err(reason) => return vec![report(label, Outcome::Unavailable, None, None, reason)],
    };
    if binaries.is_empty() {
        let reason = format!(
            "{} is not installed on an absolute PATH entry",
            adapter.program()
        );
        return vec![report(label, Outcome::Unavailable, None, None, reason)];
    }
    match proc::is_command_running(&env.proc_root, adapter.busy_names()) {
        ProcessState::NotRunning => {}
        ProcessState::Running => {
            let reason = format!("{} is running", adapter.program());
            return vec![report(label, Outcome::Skipped, None, None, reason)];
        }
        ProcessState::Unknown => {
            let reason = "owner process state is unknown".to_owned();
            return vec![report(label, Outcome::Unavailable, None, None, reason)];
        }
    }

    let command = format!("{} {}", adapter.program(), adapter.clean_args().join(" "));
    let mut reports = Vec::new();
    let mut scheduled = BTreeSet::new();
    for binary in binaries {
        if let Err(error) = run_bounded(&binary, &["--version"], env.query_timeout) {
            let reason = format!("--version failed: {}", error.describe());
            reports.push(report(
                label,
                Outcome::Unavailable,
                Some(binary),
                None,
                reason,
            ));
            continue;
        }
        let target = match adapter.target(&binary, env) {
            Ok(target) => target,
            Err(refusal) => {
                reports.push(report(
                    label,
                    refusal.outcome,
                    Some(binary),
                    None,
                    refusal.reason,
                ));
                continue;
            }
        };
        if env.whitelist.is_whitelisted(&target) {
            let reason = "whitelisted".to_owned();
            reports.push(report(
                label,
                Outcome::Skipped,
                Some(binary),
                Some(target),
                reason,
            ));
            continue;
        }
        // Two binaries that resolve to one store run one owner cleanup, as upstream dedupes.
        if !scheduled.insert(target.clone()) {
            continue;
        }
        let (outcome, reason) = match mode {
            RunMode::DryRun => (Outcome::Unchanged, format!("would run `{command}`")),
            RunMode::Apply => match run_bounded(&binary, adapter.clean_args(), env.clean_timeout) {
                Ok(captured) if captured.status.success() => {
                    (Outcome::Applied, format!("ran `{command}`"))
                }
                Ok(captured) => (
                    Outcome::Failed,
                    format!("`{command}` failed ({})", captured.status),
                ),
                Err(RunError::Timeout) => (
                    Outcome::Unavailable,
                    format!("`{command}` timed out and was killed"),
                ),
                Err(error) => (
                    Outcome::Failed,
                    format!("`{command}`: {}", error.describe()),
                ),
            },
        };
        reports.push(report(label, outcome, Some(binary), Some(target), reason));
    }
    reports
}

fn report(
    label: &'static str,
    outcome: Outcome,
    program: Option<PathBuf>,
    target: Option<PathBuf>,
    reason: String,
) -> Report {
    Report {
        label,
        outcome,
        program,
        target,
        reason,
    }
}

/// Resolves `program` through absolute PATH entries only; a relative entry could run a repository's binary.
fn resolve_on_path(program: &str, path: &OsStr) -> Option<PathBuf> {
    split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable_file(candidate))
}

/// Runs an owner query and returns its stdout without the trailing line break.
fn query_text(binary: &Path, args: &[&str], env: &AdapterEnv) -> Result<String, Refusal> {
    let command = format!("{} {}", binary.display(), args.join(" "));
    let captured = run_bounded(binary, args, env.query_timeout).map_err(|error| Refusal {
        outcome: Outcome::Unavailable,
        reason: format!("`{command}`: {}", error.describe()),
    })?;
    if !captured.status.success() {
        return Err(Refusal {
            outcome: Outcome::Unavailable,
            reason: format!("`{command}` failed ({})", captured.status),
        });
    }
    Ok(captured.stdout.trim_end_matches(['\n', '\r']).to_owned())
}

/// Accepts an absolute path with no control bytes and at least one component. Returns None otherwise.
fn validated_absolute(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text);
    let clean = !text.is_empty() && !text.chars().any(char::is_control);
    (clean && path.is_absolute() && path.parent().is_some()).then_some(path)
}

#[derive(Debug)]
enum RunError {
    Timeout,
    Io(String),
}

impl RunError {
    fn describe(&self) -> String {
        match self {
            Self::Timeout => "timed out and was killed".to_owned(),
            Self::Io(reason) => reason.clone(),
        }
    }
}

/// Kills and reaps a child we are abandoning. kill fails only when the child exited first; wait reaps either way.
fn reap(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct Captured {
    status: ExitStatus,
    stdout: String,
}

/// Runs an owner command by absolute path with argv as data, no shell, no inherited stdin.
/// A child that outlives its timeout is killed; the caller decides what that means.
fn run_bounded(program: &Path, args: &[&str], timeout: Duration) -> Result<Captured, RunError> {
    let mut child = Command::new(program)
        .args(args)
        .env(NO_DOWNLOAD_PROMPT.0, NO_DOWNLOAD_PROMPT.1)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| RunError::Io(error.to_string()))?;
    let Some(mut pipe) = child.stdout.take() else {
        reap(&mut child);
        return Err(RunError::Io("stdout pipe missing".to_owned()));
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = (&mut pipe)
            .take(OUTPUT_LIMIT)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        // The receiver is gone only after a timeout, when nobody needs the bytes.
        let _ = sender.send(read);
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                reap(&mut child);
                return Err(RunError::Timeout);
            }
            Err(error) => {
                reap(&mut child);
                return Err(RunError::Io(error.to_string()));
            }
        }
    };
    let stdout = match receiver.recv_timeout(Duration::from_secs(1)) {
        Ok(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(Err(error)) => return Err(RunError::Io(error.to_string())),
        Err(_) => return Err(RunError::Io("output stream did not close".to_owned())),
    };
    Ok(Captured { status, stdout })
}
