// Ported from tw93/Mole lib/optimize/catalog.sh (catalog validation, exact action lookup,
// exclusions) and lib/optimize/tasks.sh (task handlers) (GPL-3.0). Linux tasks are a
// narrow replacement: every task is either a user-scope cache refresh or a report.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::core::outcome::Outcome;
use crate::core::paths::XdgPaths;
use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRun, OwnerRunner, RunMode};

/// Journals above this are flagged for review. Ponytail: a fixed SI threshold; upgrade path
/// is a configurable value once the whitelist format carries settings.
const JOURNAL_ATTENTION_BYTES: u64 = 4_000_000_000;
const MAX_EXCLUSIONS_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Handler {
    FontCache,
    DesktopMime,
    FailedUserUnits,
    FstrimStatus,
    JournalHealth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizeTask {
    pub action: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub safe: bool,
    pub handler: Handler,
}

static CATALOG: [OptimizeTask; 5] = [
    OptimizeTask {
        action: "font_cache_refresh",
        label: "Font Cache Refresh",
        description: "Rebuild the per-user fontconfig cache (no root)",
        safe: true,
        handler: Handler::FontCache,
    },
    OptimizeTask {
        action: "desktop_mime_refresh",
        label: "Desktop & MIME Database Refresh",
        description: "Rebuild user desktop-entry and MIME caches where those directories exist",
        safe: true,
        handler: Handler::DesktopMime,
    },
    OptimizeTask {
        action: "failed_user_units_report",
        label: "Failed User Units",
        description: "Report failed systemd user units (report only)",
        safe: true,
        handler: Handler::FailedUserUnits,
    },
    OptimizeTask {
        action: "fstrim_status_report",
        label: "Fstrim Timer Status",
        description: "Report whether fstrim.timer runs periodic TRIM (report only)",
        safe: true,
        handler: Handler::FstrimStatus,
    },
    OptimizeTask {
        action: "journal_health_report",
        label: "Journal Health",
        description: "Report journal disk usage and flag large journals (report only)",
        safe: true,
        handler: Handler::JournalHealth,
    },
];

pub fn catalog() -> &'static [OptimizeTask] {
    &CATALOG
}

/// Exact action-id lookup. Display labels and prefixes never resolve a task.
pub fn lookup(action: &str) -> Option<&'static OptimizeTask> {
    CATALOG.iter().find(|task| task.action == action)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    Empty,
    InvalidIdentity(String),
    MissingText(String),
    Unsafe(String),
    Duplicate(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("optimize task catalog is empty"),
            Self::InvalidIdentity(action) => {
                write!(formatter, "invalid optimize task id: {action}")
            }
            Self::MissingText(action) => {
                write!(formatter, "optimize task text is incomplete: {action}")
            }
            Self::Unsafe(action) => write!(
                formatter,
                "optimize task is not safe for automatic execution: {action}"
            ),
            Self::Duplicate(action) => write!(formatter, "duplicate optimize task id: {action}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Rejects empty catalogs, malformed or duplicate ids, missing text, and unsafe tasks.
pub fn validate(tasks: &[OptimizeTask]) -> Result<(), CatalogError> {
    if tasks.is_empty() {
        return Err(CatalogError::Empty);
    }
    let mut seen = BTreeSet::new();
    for task in tasks {
        if !valid_action(task.action) {
            return Err(CatalogError::InvalidIdentity(task.action.to_owned()));
        }
        if task.label.is_empty() || task.description.is_empty() {
            return Err(CatalogError::MissingText(task.action.to_owned()));
        }
        if !task.safe {
            return Err(CatalogError::Unsafe(task.action.to_owned()));
        }
        if !seen.insert(task.action) {
            return Err(CatalogError::Duplicate(task.action.to_owned()));
        }
    }
    Ok(())
}

fn valid_action(action: &str) -> bool {
    !action.is_empty()
        && action
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Action ids the user excluded in `whitelist_optimize` (one id per line, `#` comments).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Exclusions {
    actions: BTreeSet<String>,
    pub warnings: Vec<String>,
}

impl Exclusions {
    pub fn path(xdg: &XdgPaths) -> PathBuf {
        xdg.config.join("mole-linux/whitelist_optimize")
    }

    pub fn load(xdg: &XdgPaths) -> io::Result<Self> {
        let path = Self::path(xdg);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > MAX_EXCLUSIONS_BYTES => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} exceeds {MAX_EXCLUSIONS_BYTES} bytes", path.display()),
            )),
            Ok(_) => Ok(Self::parse(&fs::read_to_string(&path)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn parse(contents: &str) -> Self {
        let mut exclusions = Self::default();
        for line in contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            if lookup(line).is_some() {
                exclusions.actions.insert(line.to_owned());
            } else {
                exclusions
                    .warnings
                    .push(format!("unknown optimize task: {}", escape_controls(line)));
            }
        }
        exclusions
    }

    pub fn excludes(&self, action: &str) -> bool {
        self.actions.contains(action)
    }
}

pub struct TaskContext<'a> {
    pub xdg: &'a XdgPaths,
    pub runner: &'a mut OwnerRunner,
    pub mode: RunMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskReport {
    pub outcome: Outcome,
    pub message: String,
}

/// Runs one catalog task. Report tasks are read-only and run in dry-run too; refresh
/// tasks only preview (and check their tool is present) in dry-run.
pub fn execute(task: &OptimizeTask, ctx: &mut TaskContext<'_>) -> TaskReport {
    match task.handler {
        Handler::FontCache => refresh(
            ctx,
            &OwnerCommand::new(OwnerProgram::FcCache, Vec::<&str>::new(), false),
            "fc-cache completed",
        ),
        Handler::DesktopMime => desktop_mime(ctx),
        Handler::FailedUserUnits => failed_user_units(ctx),
        Handler::FstrimStatus => fstrim_status(ctx),
        Handler::JournalHealth => journal_health(ctx),
    }
}

fn refresh(ctx: &mut TaskContext<'_>, cmd: &OwnerCommand, done: &str) -> TaskReport {
    match ctx.mode {
        RunMode::DryRun => {
            if ctx.runner.is_available(cmd.program()) {
                TaskReport {
                    outcome: Outcome::Applied,
                    message: format!("would run {}", cmd.describe()),
                }
            } else {
                unavailable(format!("{} is not installed", cmd.program().file_name()))
            }
        }
        RunMode::Apply => {
            let run = ctx.runner.run(cmd, RunMode::Apply, || false);
            TaskReport {
                outcome: run.outcome,
                message: run.reason.unwrap_or_else(|| done.to_owned()),
            }
        }
    }
}

fn desktop_mime(ctx: &mut TaskContext<'_>) -> TaskReport {
    // update-mime-database rebuilds from <dir>/packages and exits 1 without it, so that
    // subdirectory is the precondition; a bare mime/ directory has nothing to rebuild.
    let targets = [
        (
            ctx.xdg.data.join("applications"),
            OwnerProgram::UpdateDesktopDatabase,
            "desktop entry",
            "no desktop entry directory",
        ),
        (
            ctx.xdg.data.join("mime/packages"),
            OwnerProgram::UpdateMimeDatabase,
            "MIME",
            "no MIME packages directory",
        ),
    ];
    let mut reports = Vec::with_capacity(targets.len());
    for (dir, program, what, missing) in targets {
        if !dir.is_dir() {
            reports.push(TaskReport {
                outcome: Outcome::Unchanged,
                message: format!("{what}: {missing}"),
            });
            continue;
        }
        let cmd = OwnerCommand::new(program, [dir.into_os_string()], false);
        let report = refresh(ctx, &cmd, "database rebuilt");
        reports.push(TaskReport {
            outcome: report.outcome,
            message: format!("{what}: {}", report.message),
        });
    }

    let count = |wanted: Outcome| {
        reports.iter().fold(0_u64, |total, report| {
            total + u64::from(report.outcome == wanted)
        })
    };
    let outcome = match Outcome::from_counts(
        count(Outcome::Applied),
        count(Outcome::Failed),
        count(Outcome::Skipped),
    ) {
        Outcome::Unchanged if count(Outcome::Unavailable) > 0 => Outcome::Unavailable,
        other => other,
    };
    TaskReport {
        outcome,
        message: reports
            .iter()
            .map(|report| report.message.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn failed_user_units(ctx: &mut TaskContext<'_>) -> TaskReport {
    let cmd = OwnerCommand::new(
        OwnerProgram::Systemctl,
        ["--user", "--failed", "--no-legend", "--plain", "--no-pager"],
        false,
    );
    match ctx.runner.query(&cmd) {
        Ok(stdout) => {
            let units: Vec<String> = stdout
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .map(escape_controls)
                .collect();
            if units.is_empty() {
                TaskReport {
                    outcome: Outcome::Unchanged,
                    message: "no failed user units".to_owned(),
                }
            } else {
                TaskReport {
                    outcome: Outcome::Attention,
                    message: format!("{} failed user unit(s): {}", units.len(), units.join(", ")),
                }
            }
        }
        Err(run) => {
            let reason = query_reason(&run);
            // Without a session bus systemctl --user cannot answer; say so instead of the raw bus error.
            let hint = if reason.contains("Failed to connect to") {
                "; hint: no user session bus (run from your desktop session)"
            } else {
                ""
            };
            unavailable(format!("systemctl --user --failed: {reason}{hint}"))
        }
    }
}

fn fstrim_status(ctx: &mut TaskContext<'_>) -> TaskReport {
    // `is-enabled` prints its answer on stdout and exits nonzero for disabled units.
    let cmd = OwnerCommand::new(
        OwnerProgram::Systemctl,
        ["is-enabled", "fstrim.timer"],
        false,
    )
    .answer_on_exit(1);
    match ctx.runner.query(&cmd) {
        Ok(stdout) => match stdout.trim() {
            "enabled" => TaskReport {
                outcome: Outcome::Unchanged,
                message: "fstrim.timer is enabled".to_owned(),
            },
            state @ ("disabled" | "masked") => TaskReport {
                outcome: Outcome::Attention,
                message: format!("fstrim.timer is {state}; periodic TRIM is off"),
            },
            "not-found" => unavailable("fstrim.timer is not installed".to_owned()),
            "" => unavailable("systemctl is-enabled gave no state".to_owned()),
            other => unavailable(format!(
                "unrecognized fstrim.timer state: {}",
                escape_controls(other)
            )),
        },
        Err(run) => unavailable(format!("systemctl is-enabled: {}", query_reason(&run))),
    }
}

fn journal_health(ctx: &mut TaskContext<'_>) -> TaskReport {
    let cmd = OwnerCommand::new(OwnerProgram::Journalctl, ["--disk-usage"], false);
    match ctx.runner.query(&cmd) {
        Ok(stdout) => match parse_journal_usage(&stdout) {
            Some(bytes) if bytes > JOURNAL_ATTENTION_BYTES => TaskReport {
                outcome: Outcome::Attention,
                message: format!(
                    "journal uses {} (review threshold {})",
                    format_si(bytes),
                    format_si(JOURNAL_ATTENTION_BYTES)
                ),
            },
            Some(bytes) => TaskReport {
                outcome: Outcome::Unchanged,
                message: format!("journal uses {}", format_si(bytes)),
            },
            None => unavailable("unrecognized journalctl --disk-usage output".to_owned()),
        },
        Err(run) => unavailable(format!("journalctl --disk-usage: {}", query_reason(&run))),
    }
}

/// Reads journalctl's `take up <n><unit>` figure in binary units (journalctl prints IEC
/// suffixes). Exact integer math: no floats, overflow returns None.
pub fn parse_journal_usage(output: &str) -> Option<u64> {
    let (_, rest) = output.split_once("take up ")?;
    let token = rest.split_whitespace().next()?;
    let unit = token.chars().last()?;
    let number = &token[..token.len() - unit.len_utf8()];
    let multiplier: u128 = match unit {
        'B' => 1,
        'K' => 1 << 10,
        'M' => 1 << 20,
        'G' => 1 << 30,
        'T' => 1 << 40,
        'P' => 1 << 50,
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole: u128 = whole.parse().ok()?;
    // Nine fractional digits are more than journalctl's one-decimal output needs.
    let digits = &fraction[..fraction.len().min(9)];
    let fraction_value: u128 = if digits.is_empty() {
        0
    } else {
        digits.parse().ok()?
    };
    let scale = 10_u128.pow(u32::try_from(digits.len()).ok()?);
    let total = whole
        .checked_mul(multiplier)?
        .checked_add(fraction_value.checked_mul(multiplier)? / scale)?;
    u64::try_from(total).ok()
}

/// SI (kB/MB/GB) for disk sizes in summaries.
pub fn format_si(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "kB")];
    match UNITS.iter().find(|(divisor, _)| bytes >= *divisor) {
        Some((divisor, unit)) => {
            let tenths = u128::from(bytes) * 10 / u128::from(*divisor);
            format!("{}.{} {unit}", tenths / 10, tenths % 10)
        }
        None => format!("{bytes} B"),
    }
}

/// Escapes control characters so tool output cannot rewrite the terminal or log lines.
pub fn escape_controls(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().collect::<String>()
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn query_reason(run: &OwnerRun) -> String {
    run.reason
        .as_deref()
        .map_or_else(|| "owner query failed".to_owned(), escape_controls)
}

fn unavailable(message: String) -> TaskReport {
    TaskReport {
        outcome: Outcome::Unavailable,
        message,
    }
}
