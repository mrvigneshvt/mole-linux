// Ported from tw93/Mole bin/clean.sh (inherited-root rejection) and lib/core/sudo.sh
// (per-action sudo, never a cached session) (GPL-3.0).

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use crate::core::outcome::Outcome;

pub const SYSTEM_BIN_DIR: &str = "/usr/bin";
pub const SUDO_PATH: &str = "/usr/bin/sudo";

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
        }
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
        }
    }

    /// The exact argv `run` would execute for `cmd`, for consent prompts and dry-run output.
    pub fn argv(&self, cmd: &OwnerCommand) -> Vec<OsString> {
        cmd.argv(&self.bin_dir, &self.sudo)
    }

    /// Runs an unprivileged read-only query and returns its stdout. Queries never use sudo.
    pub fn probe(&self, cmd: &OwnerCommand) -> Result<String, String> {
        if cmd.needs_root {
            return Err("owner query never runs with sudo".to_owned());
        }
        let program = cmd.program.path_in(&self.bin_dir);
        if !is_executable_file(&program) {
            return Err(format!(
                "{} is missing or not executable",
                program.display()
            ));
        }
        // ponytail: stdout is read whole; paccache's dry-run summary is one line. Upgrade path: a capped reader that kills the child on overflow. Logged in KNOWN-GAPS.md.
        let output = Process::new(&program)
            .args(&cmd.args)
            .output()
            .map_err(|error| format!("could not start {}: {error}", program.display()))?;
        if !output.status.success() {
            return Err(format!("exited with {}", output.status));
        }
        String::from_utf8(output.stdout)
            .map_err(|_| format!("{} printed non-UTF-8 output", program.display()))
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
