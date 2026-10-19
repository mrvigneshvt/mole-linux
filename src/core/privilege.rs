// Ported from tw93/Mole bin/clean.sh (inherited-root rejection) and lib/core/sudo.sh
// (per-action sudo, never a cached session) (GPL-3.0).

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::outcome::Outcome;

pub const SYSTEM_BIN_DIR: &str = "/usr/bin";
pub const SUDO_PATH: &str = "/usr/bin/sudo";
const QUERY_OUTPUT_LIMIT: u64 = 1024 * 1024;
const QUERY_STDERR_LIMIT: u64 = 4096;
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegeError {
    InheritedRoot,
}

impl fmt::Display for PrivilegeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InheritedRoot => formatter.write_str(
                "Run mole-linux without sudo; it requests administrator access when needed.",
            ),
        }
    }
}

impl std::error::Error for PrivilegeError {}

/// Refuses to run with inherited root: user state and installed tools must never run as root.
pub fn ensure_not_root(euid: u32) -> Result<(), PrivilegeError> {
    if euid == 0 {
        Err(PrivilegeError::InheritedRoot)
    } else {
        Ok(())
    }
}

/// Closed allowlist of owner tools. Callers cannot name an arbitrary program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerProgram {
    Pacman,
    Paccache,
    Journalctl,
    Flatpak,
    Coredumpctl,
}

impl OwnerProgram {
    fn file_name(self) -> &'static str {
        match self {
            Self::Pacman => "pacman",
            Self::Paccache => "paccache",
            Self::Journalctl => "journalctl",
            Self::Flatpak => "flatpak",
            Self::Coredumpctl => "coredumpctl",
        }
    }

    fn path_in(self, bin_dir: &Path) -> PathBuf {
        bin_dir.join(self.file_name())
    }
}

/// One owner-tool invocation as argv data. It is never a shell string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerCommand {
    program: OwnerProgram,
    args: Vec<OsString>,
    needs_root: bool,
    no_match_exit: Option<i32>,
}

impl OwnerCommand {
    pub fn new<I, S>(program: OwnerProgram, args: I, needs_root: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
            needs_root,
            no_match_exit: None,
        }
    }

    /// Declares the exit code the tool uses for "nothing matched" (e.g. `pacman -Qm` → 1).
    /// Only a silent exit with this code counts as an empty answer in `OwnerRunner::query`.
    pub fn no_match_on_exit(mut self, code: i32) -> Self {
        self.no_match_exit = Some(code);
        self
    }

    /// The exact argv that executes: `sudo -- <program> <args>` only when root is needed.
    pub fn argv(&self, bin_dir: &Path, sudo: &Path) -> Vec<OsString> {
        let mut argv = Vec::with_capacity(self.args.len() + 3);
        if self.needs_root {
            argv.push(sudo.as_os_str().to_owned());
            argv.push(OsString::from("--"));
        }
        argv.push(self.program.path_in(bin_dir).into_os_string());
        argv.extend(self.args.iter().cloned());
        argv
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consent {
    Unasked,
    Granted,
    Refused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    DryRun,
    Apply,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerRun {
    pub outcome: Outcome,
    pub argv: Vec<OsString>,
    pub reason: Option<String>,
}

/// Runs owner tools for one run. Consent is asked at most once, on the first root action.
pub struct OwnerRunner {
    bin_dir: PathBuf,
    sudo: PathBuf,
    consent: Consent,
    query_timeout: Duration,
}

impl OwnerRunner {
    pub fn system(consent: Consent) -> Self {
        Self::new(
            PathBuf::from(SYSTEM_BIN_DIR),
            PathBuf::from(SUDO_PATH),
            consent,
        )
    }

    pub fn new(bin_dir: PathBuf, sudo: PathBuf, consent: Consent) -> Self {
        Self {
            bin_dir,
            sudo,
            consent,
            query_timeout: QUERY_TIMEOUT,
        }
    }

    pub fn with_query_timeout(mut self, timeout: Duration) -> Self {
        self.query_timeout = timeout;
        self
    }

    /// The exact argv `run` would execute for `cmd`, for consent prompts and dry-run output.
    pub fn argv(&self, cmd: &OwnerCommand) -> Vec<OsString> {
        cmd.argv(&self.bin_dir, &self.sudo)
    }

    /// Read-only owner query: runs an unprivileged owner tool and returns its bounded stdout.
    /// Never prompts and never uses sudo; a root-requiring query is refused, not escalated.
    /// A silent exit with the command's declared no-match code is an empty answer; any
    /// other failure is an error, so a locked database never reads as "nothing installed".
    pub fn query(&self, cmd: &OwnerCommand) -> Result<String, OwnerRun> {
        let argv = cmd.argv(&self.bin_dir, &self.sudo);
        if cmd.needs_root {
            return Err(skipped(argv, "owner query never runs with sudo; skipped"));
        }
        let program = cmd.program.path_in(&self.bin_dir);
        if !is_executable_file(&program) {
            return Err(unavailable(
                argv,
                format!("{} is missing or not executable", program.display()),
            ));
        }

        let mut child = match Process::new(&program)
            .args(&cmd.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(unavailable(argv, format!("{error}")));
            }
            Err(error) => {
                return Err(failed(argv, format!("could not start: {error}")));
            }
        };
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            let detail = stop(&mut child);
            return Err(failed(argv, format!("output was unavailable; {detail}")));
        };
        // Reading stops at each limit; dropping a reader closes its pipe, so an oversized
        // child gets EPIPE and exits instead of blocking forever.
        let stdout_reader = read_capped(stdout, QUERY_OUTPUT_LIMIT);
        let stderr_reader = read_capped(stderr, QUERY_STDERR_LIMIT);

        let started = Instant::now();
        let status: ExitStatus = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < self.query_timeout => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    // The readers are left to finish on their own: a grandchild may still hold a pipe.
                    let detail = stop(&mut child);
                    return Err(failed(argv, format!("timed out; {detail}")));
                }
                Err(error) => {
                    let detail = stop(&mut child);
                    return Err(failed(
                        argv,
                        format!("could not check status: {error}; {detail}"),
                    ));
                }
            }
        };
        let stdout_bytes = match join_reader(stdout_reader) {
            Ok(bytes) => bytes,
            Err(reason) => return Err(failed(argv, reason)),
        };
        if u64::try_from(stdout_bytes.len()).map_or(true, |length| length > QUERY_OUTPUT_LIMIT) {
            return Err(failed(argv, "output exceeded 1 MiB limit"));
        }
        let stderr_bytes = match join_reader(stderr_reader) {
            Ok(bytes) => bytes,
            Err(reason) => return Err(failed(argv, reason)),
        };
        if status.success() {
            return Ok(String::from_utf8_lossy(&stdout_bytes).into_owned());
        }
        if cmd.no_match_exit == status.code() && stdout_bytes.is_empty() && stderr_bytes.is_empty()
        {
            return Ok(String::new());
        }
        let message = String::from_utf8_lossy(&stderr_bytes);
        match message.lines().map(str::trim).find(|line| !line.is_empty()) {
            Some(line) => Err(failed(argv, format!("exited with {status}: {line}"))),
            None => Err(failed(argv, format!("exited with {status}"))),
        }
    }

    pub fn run(
        &mut self,
        cmd: &OwnerCommand,
        mode: RunMode,
        confirm: impl FnOnce() -> bool,
    ) -> OwnerRun {
        let argv = cmd.argv(&self.bin_dir, &self.sudo);
        if mode == RunMode::DryRun {
            return skipped(argv, "dry run: not executed");
        }

        let program = cmd.program.path_in(&self.bin_dir);
        if !is_executable_file(&program) {
            return unavailable(
                argv,
                format!("{} is missing or not executable", program.display()),
            );
        }
        if cmd.needs_root {
            if !is_executable_file(&self.sudo) {
                return unavailable(
                    argv,
                    "sudo is missing or not executable; owner action skipped",
                );
            }
            if self.consent == Consent::Unasked {
                self.consent = if confirm() {
                    Consent::Granted
                } else {
                    Consent::Refused
                };
            }
            if self.consent == Consent::Refused {
                return skipped(argv, "consent declined; owner action skipped");
            }
        }

        let mut process = if cmd.needs_root {
            let mut process = Process::new(&self.sudo);
            process.arg("--").arg(&program);
            process
        } else {
            Process::new(&program)
        };
        process.args(&cmd.args);

        match process.status() {
            Ok(status) if status.success() => OwnerRun {
                outcome: Outcome::Applied,
                argv,
                reason: None,
            },
            // ponytail: sudo's own auth failure (exit 1) is indistinguishable from the owner tool's exit 1; upgrade by checking sudo auth separately. Logged in KNOWN-GAPS.md.
            Ok(status) => OwnerRun {
                outcome: Outcome::Failed,
                argv,
                reason: Some(format!("exited with {status}")),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                unavailable(argv, format!("{error}"))
            }
            Err(error) => OwnerRun {
                outcome: Outcome::Failed,
                argv,
                reason: Some(format!("could not start: {error}")),
            },
        }
    }
}

/// The single per-run consent prompt for owner actions. It lists every exact argv first.
/// Without a terminal nothing can be asked, so consent is refused and nothing runs.
pub fn ask_owner_consent(listing: &[Vec<OsString>]) -> bool {
    if !io::stdin().is_terminal() {
        eprintln!("owner actions need a terminal consent prompt or --yes; skipped");
        return false;
    }
    println!("Owner actions need administrator access (sudo). Exact commands:");
    for argv in listing {
        let rendered: Vec<String> = argv
            .iter()
            .map(|arg| arg.to_string_lossy().escape_debug().to_string())
            .collect();
        println!("  {}", rendered.join(" "));
    }
    print!("Run these commands with sudo? [y/N] ");
    if io::stdout().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes" | "YES")
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Reads at most `limit` bytes plus one, so the caller can tell "over the limit" from "at it".
fn read_capped<R: Read + Send + 'static>(
    reader: R,
    limit: u64,
) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    })
}

fn join_reader(handle: thread::JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>, String> {
    match handle.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(format!("could not read output: {error}")),
        Err(_) => Err("output reader failed".to_owned()),
    }
}

/// Kills and reaps a child that must not keep running; the text says what happened.
fn stop(child: &mut Child) -> String {
    match (child.kill(), child.wait()) {
        (Ok(()), Ok(_)) => "terminated the tool".to_owned(),
        (Err(kill_error), Ok(_)) => format!("could not terminate the tool: {kill_error}"),
        (_, Err(wait_error)) => format!("could not reap the tool: {wait_error}"),
    }
}

fn failed(argv: Vec<OsString>, reason: impl Into<String>) -> OwnerRun {
    OwnerRun {
        outcome: Outcome::Failed,
        argv,
        reason: Some(reason.into()),
    }
}

fn skipped(argv: Vec<OsString>, reason: &str) -> OwnerRun {
    OwnerRun {
        outcome: Outcome::Skipped,
        argv,
        reason: Some(reason.to_owned()),
    }
}

fn unavailable(argv: Vec<OsString>, reason: impl Into<String>) -> OwnerRun {
    OwnerRun {
        outcome: Outcome::Unavailable,
        argv,
        reason: Some(reason.into()),
    }
}
