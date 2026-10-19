use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use crate::core::paths::XdgPaths;
use crate::core::privilege::{OwnerCommand, OwnerRunner};
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
    /// Shown as `attention` in every run; never deleted or sent to an owner tool.
    ReportOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerCheck {
    None,
    ProcessName(String),
    PathOpen,
}

/// An owner command that performs the cleanup of a candidate directory instead of a sink deletion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerAction {
    pub program: PathBuf,
    pub args: &'static [&'static str],
    pub timeout: std::time::Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub identity: Identity,
    pub apparent_size: Option<u64>,
    pub allocated_size: Option<u64>,
    pub reason: String,
    pub report: Option<String>,
    pub recovery: RecoveryMode,
    pub selected_by_default: bool,
    pub owner_state: ProcessState,
    pub owner_check: OwnerCheck,
    /// Set for owner-managed caches: the owner command replaces the sink deletion.
    pub owner_action: Option<OwnerAction>,
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

    pub fn check_owner(&self, check: &OwnerCheck, path: &Path) -> ProcessState {
        match check {
            OwnerCheck::None => ProcessState::NotRunning,
            OwnerCheck::ProcessName(name) => self.owner_state(name),
            OwnerCheck::PathOpen => crate::core::proc::is_path_open(Path::new("/proc"), path),
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

    /// Discovery for rules that need an owner tool to preview (e.g. `paccache -dk3`).
    /// Plain rules keep the default, which ignores the runner.
    fn discover_with_owner(&self, ctx: &RuleContext<'_>, _owner: &OwnerRunner) -> Discovery {
        self.discover(ctx)
    }

    /// The owner command that applies an `OwnerNative` candidate. Sink-backed rules return `None`.
    fn owner_command(&self, _candidate: &Candidate) -> Option<OwnerCommand> {
        None
    }

    /// Read-only report for the operator. Report-only rules return `Some`; candidate rules keep
    /// the default. A report never adds candidates and never applies anything.
    fn report_with_owner(&self, _ctx: &RuleContext<'_>, _owner: &OwnerRunner) -> Option<Report> {
        None
    }
}

/// What a report-only rule found. Each report is recorded as one outcome for the whole rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Report {
    /// Nothing needs the operator's attention.
    Clear,
    /// Lines for the operator; recorded as `attention`.
    Findings(Vec<String>),
    /// The report could not be made; recorded as `unavailable`, never as clear.
    Unavailable(String),
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
        report: None,
        recovery,
        selected_by_default: owner_state == ProcessState::NotRunning,
        owner_state,
        owner_check: OwnerCheck::None,
        owner_action: None,
    })
}

/// A candidate whose cleanup waits for the named process to be absent. A running or unknown
/// owner keeps the candidate visible but unselected, and the final guard re-checks it.
pub fn process_gated(
    path: &Path,
    reason: impl Into<String>,
    recovery: RecoveryMode,
    process: &str,
    ctx: &RuleContext<'_>,
) -> Result<Candidate, std::io::Error> {
    let state = ctx.owner_state(process);
    let mut item = candidate(path, reason, recovery, state)?;
    item.owner_check = OwnerCheck::ProcessName(process.to_owned());
    item.selected_by_default = state == ProcessState::NotRunning;
    Ok(item)
}

/// Every registered Linux cleanup rule. Filters are validated against these ids and categories.
pub fn catalog() -> Vec<Box<dyn CleanRule>> {
    let mut rules: Vec<Box<dyn CleanRule>> = vec![
        Box::new(UserEssentialsRule),
        Box::new(TrashRule),
        Box::new(CoredumpReportRule),
        super::browser::registered(),
        super::project_cache::registered(),
    ];
    rules.extend(super::system::catalog());
    rules.extend(super::owner_cache::registered());
    rules.extend(super::editor_cache::registered());
    rules.push(super::journal::registered());
    rules.extend(super::python::registered());
    rules
}

struct TrashRule;

impl CleanRule for TrashRule {
    fn id(&self) -> &str {
        "trash"
    }

    fn category(&self) -> Category {
        Category::User
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut partial = None;
        let trash = ctx.xdg.data.join("Trash");
        discover_children(
            &trash.join("files"),
            "explicitly empty user Trash",
            RecoveryMode::Irreversible,
            &mut candidates,
            &mut partial,
        );
        discover_children(
            &trash.join("info"),
            "Trash metadata paired with user Trash items",
            RecoveryMode::Irreversible,
            &mut candidates,
            &mut partial,
        );
        for value in &mut candidates {
            value.selected_by_default = false;
        }
        let status = partial.map_or(DiscoveryStatus::Complete, DiscoveryStatus::Partial);
        Discovery { candidates, status }
    }
}

struct CoredumpReportRule;

impl CleanRule for CoredumpReportRule {
    fn id(&self) -> &str {
        "coredumps"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        const ROOT: &str = "/var/lib/systemd/coredump";
        const REASON: &str = "retention is managed by systemd-tmpfiles (/usr/lib/tmpfiles.d/systemd.conf, default 3d) and coredump.conf MaxUse=; mole-linux does not delete system coredumps";
        let stdout = match coredumpctl_list() {
            Ok(stdout) => stdout,
            Err(error) => {
                return Discovery {
                    candidates: Vec::new(),
                    status: DiscoveryStatus::Unavailable(format!(
                        "coredumpctl unavailable or denied: {error}"
                    )),
                };
            }
        };
        let rows: serde_json::Value = match serde_json::from_slice(&stdout) {
            Ok(value) => value,
            Err(error) => {
                return Discovery {
                    candidates: Vec::new(),
                    status: DiscoveryStatus::Unavailable(format!(
                        "coredumpctl returned invalid JSON: {error}"
                    )),
                };
            }
        };
        let Some(rows) = rows.as_array() else {
            return Discovery {
                candidates: Vec::new(),
                status: DiscoveryStatus::Unavailable(
                    "coredumpctl JSON response was not an array".to_owned(),
                ),
            };
        };
        let Ok(count) = u64::try_from(rows.len()) else {
            return Discovery {
                candidates: Vec::new(),
                status: DiscoveryStatus::Unavailable(
                    "coredump count does not fit the report".to_owned(),
                ),
            };
        };
        let size = readable_tree_size(Path::new(ROOT));
        // ponytail: anchored to HOME because `candidate` needs an existing path for Identity; the
        // report never touches it. Upgrade: anchor to ROOT once a missing dir is reported as its own state.
        let Ok(mut value) = candidate(
            ctx.home,
            REASON,
            RecoveryMode::ReportOnly,
            ProcessState::NotRunning,
        ) else {
            return Discovery {
                candidates: Vec::new(),
                status: DiscoveryStatus::Unavailable("cannot create report candidate".to_owned()),
            };
        };
        value.selected_by_default = false;
        value.apparent_size = size;
        value.allocated_size = None;
        value.report = Some(format!(
            "coredumpctl list count: {count}; readable size (SI): {}",
            size.map_or_else(|| "Unknown".to_owned(), format_size_si)
        ));
        Discovery {
            candidates: vec![value],
            status: DiscoveryStatus::Complete,
        }
    }
}

fn coredumpctl_list() -> Result<Vec<u8>, String> {
    const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
    let mut child = Command::new("coredumpctl")
        .args(["list", "--no-pager", "--json=short"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let Some(stdout) = child.stdout.take() else {
        let kill_result = child.kill();
        let wait_result = child.wait();
        return Err(match wait_result {
            Ok(_) => format!(
                "coredumpctl stdout pipe unavailable; kill: {}",
                kill_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned())
            ),
            Err(error) => format!(
                "coredumpctl stdout pipe unavailable; kill: {}; wait failed: {error}",
                kill_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned())
            ),
        });
    };
    let mut bytes = Vec::new();
    let read_result = stdout.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut bytes);
    if let Err(error) = read_result {
        let kill_result = child.kill();
        let wait_result = child.wait();
        return Err(format!(
            "failed to read coredumpctl output: {error}; kill: {}; wait: {}",
            kill_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned()),
            wait_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned())
        ));
    }
    let output_bytes = match u64::try_from(bytes.len()) {
        Ok(size) => size,
        Err(_) => {
            let kill_result = child.kill();
            let wait_result = child.wait();
            return Err(format!(
                "coredumpctl JSON size overflow; kill: {}; wait: {}",
                kill_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned()),
                wait_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned())
            ));
        }
    };
    if output_bytes > MAX_OUTPUT_BYTES {
        let kill_result = child.kill();
        let wait_result = child.wait();
        return Err(format!(
            "coredumpctl JSON exceeded the {} byte report limit; kill: {}; wait: {}",
            MAX_OUTPUT_BYTES,
            kill_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned()),
            wait_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned())
        ));
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let kill_result = child.kill();
            let wait_result = child.wait();
            return Err(format!(
                "failed to wait for coredumpctl: {error}; kill: {}; second wait: {}",
                kill_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned()),
                wait_result.map_or_else(|failure| failure.to_string(), |_| "ok".to_owned())
            ));
        }
    };
    if !status.success() {
        return Err(format!("coredumpctl list exited with {status}"));
    }
    Ok(bytes)
}

fn readable_tree_size(root: &Path) -> Option<u64> {
    const MAX_ENTRIES: u64 = 100_000;
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0_u64;
    let mut entries_seen = 0_u64;
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.is_file() {
            total = total.checked_add(metadata.len())?;
        } else if metadata.is_dir() {
            for entry in std::fs::read_dir(path).ok()? {
                entries_seen = entries_seen.checked_add(1)?;
                if entries_seen > MAX_ENTRIES {
                    return None;
                }
                let entry = entry.ok()?;
                let metadata = std::fs::symlink_metadata(entry.path()).ok()?;
                if metadata.is_dir() {
                    pending.push(entry.path());
                } else if metadata.is_file() {
                    total = total.checked_add(metadata.len())?;
                }
            }
        }
    }
    Some(total)
}

struct UserEssentialsRule;

impl CleanRule for UserEssentialsRule {
    fn id(&self) -> &str {
        "user-essentials"
    }
    fn category(&self) -> Category {
        Category::User
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut partial = None;
        discover_children(
            &ctx.xdg.cache.join("thumbnails"),
            "regenerable thumbnail cache",
            RecoveryMode::Regenerate,
            &mut candidates,
            &mut partial,
        );

        match std::fs::read_dir(&ctx.xdg.state) {
            Ok(apps) => {
                for app in apps {
                    match app {
                        Ok(app) => {
                            let app_path = app.path();
                            match std::fs::symlink_metadata(&app_path) {
                                Ok(metadata) if metadata.is_dir() => {}
                                Ok(_) => continue,
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                    continue;
                                }
                                Err(error) => {
                                    partial = Some(format!("{}: {error}", escaped_path(&app_path)));
                                    continue;
                                }
                            }
                            let log_dir = app_path.join("log");
                            match std::fs::symlink_metadata(&log_dir) {
                                Ok(metadata) if !metadata.is_dir() => continue,
                                Ok(_) => {}
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                    continue;
                                }
                                Err(error) => {
                                    partial = Some(format!("{}: {error}", escaped_path(&log_dir)));
                                    continue;
                                }
                            }
                            discover_children(
                                &log_dir,
                                "application log under XDG state",
                                RecoveryMode::Trash,
                                &mut candidates,
                                &mut partial,
                            );
                        }
                        Err(error) => {
                            partial = Some(format!("{}: {error}", escaped_path(&ctx.xdg.state)))
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => partial = Some(format!("{}: {error}", escaped_path(&ctx.xdg.state))),
        }

        // Ported from tw93/Mole lib/clean/user.sh:_clean_incomplete_downloads (GPL-3.0).
        // Linux partials are eligible only after seven days and a conclusive idle-fd probe.
        let downloads = ctx.home.join("Downloads");
        match std::fs::read_dir(&downloads) {
            Ok(entries) => {
                for entry in entries {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            partial = Some(format!("{}: {error}", escaped_path(&downloads)));
                            continue;
                        }
                    };
                    let path = entry.path();
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if ![".part", ".crdownload", ".download"]
                        .iter()
                        .any(|suffix| name.ends_with(suffix))
                    {
                        continue;
                    }
                    let metadata = match std::fs::symlink_metadata(&path) {
                        Ok(metadata) if metadata.is_file() => metadata,
                        Ok(_) => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            partial = Some(format!("{}: {error}", escaped_path(&path)));
                            continue;
                        }
                    };
                    let modified = match metadata.modified() {
                        Ok(modified) => modified,
                        Err(error) => {
                            partial = Some(format!("{}: {error}", escaped_path(&path)));
                            continue;
                        }
                    };
                    let old_enough = ctx
                        .now
                        .duration_since(modified)
                        .is_ok_and(|age| age.as_secs() >= 7 * 24 * 60 * 60);
                    if !old_enough {
                        continue;
                    }
                    let owner_state = crate::core::proc::is_path_open(Path::new("/proc"), &path);
                    if owner_state == ProcessState::Running {
                        continue;
                    }
                    if owner_state == ProcessState::Unknown {
                        partial =
                            Some("open-file state unavailable for incomplete downloads".to_owned());
                        continue;
                    }
                    match candidate(
                        &path,
                        "stale incomplete download",
                        RecoveryMode::Trash,
                        owner_state,
                    ) {
                        Ok(mut value) => {
                            value.owner_check = OwnerCheck::PathOpen;
                            candidates.push(value);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => partial = Some(format!("{}: {error}", escaped_path(&path))),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => partial = Some(format!("{}: {error}", escaped_path(&downloads))),
        }

        let status = partial.map_or(DiscoveryStatus::Complete, DiscoveryStatus::Partial);
        Discovery { candidates, status }
    }
}

fn discover_children(
    root: &Path,
    reason: &str,
    recovery: RecoveryMode,
    candidates: &mut Vec<Candidate>,
    partial: &mut Option<String>,
) {
    match std::fs::read_dir(root) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        match candidate(&entry.path(), reason, recovery, ProcessState::NotRunning) {
                            Ok(value) => candidates.push(value),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => {
                                *partial = Some(format!("{}: {error}", escaped_path(&entry.path())))
                            }
                        }
                    }
                    Err(error) => *partial = Some(format!("{}: {error}", escaped_path(root))),
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {}
        Err(error) => *partial = Some(format!("{}: {error}", escaped_path(root))),
    }
}

fn escaped_path(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|byte| {
            if *byte < 0x20 || *byte == 0x7f {
                format!("\\x{byte:02x}")
            } else {
                char::from(*byte).to_string()
            }
        })
        .collect()
}

fn format_size_si(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut divisor = 1000_u64;
    for (index, unit) in UNITS.iter().enumerate() {
        if index == UNITS.len() - 1 || bytes < divisor.saturating_mul(1000) {
            let whole = bytes / divisor;
            let tenths = (bytes % divisor) / (divisor / 10);
            return format!("{whole}.{tenths} {unit}");
        }
        divisor = divisor.saturating_mul(1000);
    }
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::format_size_si;

    #[test]
    fn coredump_sizes_use_si_units_across_u64_range() {
        assert_eq!(format_size_si(999), "999 B");
        assert_eq!(format_size_si(1_000), "1.0 kB");
        assert_eq!(format_size_si(1_234_567), "1.2 MB");
        assert_eq!(format_size_si(u64::MAX), "18.4 EB");
    }
}
