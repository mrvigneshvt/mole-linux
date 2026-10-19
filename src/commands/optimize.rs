// Ported from tw93/Mole bin/optimize.sh (run loop, dry-run summary, history records) (GPL-3.0).
// Six-state outcomes come from core::outcome; the task catalog lives in rules::optimize.

use std::io::{self, Write};
use std::path::Path;

use crate::core::oplog::{OpLog, SessionSummary};
use crate::core::outcome::{Outcome, OutcomeError, OutcomeLedger};
use crate::core::paths::XdgPaths;
use crate::core::privilege::{Consent, OwnerRunner, RunMode};
use crate::rules::optimize::{self, Exclusions, TaskContext, TaskReport, escape_controls};

pub fn run(dry_run: bool, whitelist: bool) -> io::Result<()> {
    let xdg = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    if whitelist {
        println!("Whitelist: {}", Exclusions::path(&xdg).display());
        return Ok(());
    }
    let mut runner = OwnerRunner::system(Consent::Unasked);
    let log = OpLog::new()?;
    let mut out = io::stdout().lock();
    let ledger = execute(dry_run, &xdg, &mut runner, &log, &mut out)?;
    let failed = ledger.count(Outcome::Failed);
    if failed > 0 {
        return Err(io::Error::other(format!(
            "{failed} optimize task(s) failed"
        )));
    }
    Ok(())
}

/// Runs every catalog task once, in catalog order, and returns the six-state ledger.
/// Failed tasks are reported in the output and history, not as an error here.
pub fn execute(
    dry_run: bool,
    xdg: &XdgPaths,
    runner: &mut OwnerRunner,
    log: &OpLog,
    out: &mut dyn Write,
) -> io::Result<OutcomeLedger> {
    optimize::validate(optimize::catalog())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let exclusions = Exclusions::load(xdg)?;
    for warning in &exclusions.warnings {
        writeln!(out, "Warning: whitelist_optimize {warning}")?;
    }

    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Apply
    };
    let mut session = log.start_session("optimize", dry_run)?;
    let mut ledger = OutcomeLedger::default();
    for task in optimize::catalog() {
        let report = if exclusions.excludes(task.action) {
            TaskReport {
                outcome: Outcome::Skipped,
                message: "excluded by whitelist_optimize".to_owned(),
            }
        } else {
            let mut ctx = TaskContext {
                xdg,
                runner: &mut *runner,
                mode,
            };
            optimize::execute(task, &mut ctx)
        };

        ledger.start().map_err(ledger_error)?;
        ledger.report(report.outcome).map_err(ledger_error)?;
        ledger.finish(task.action).map_err(ledger_error)?;
        let action = if report.outcome == Outcome::Failed {
            "TASK_FAILED"
        } else {
            "TASK"
        };
        session.record(action, report.outcome.as_str(), Path::new(task.action))?;
        writeln!(
            out,
            "{}: {} - {}",
            task.label,
            row_label(report.outcome, dry_run),
            escape_controls(&report.message)
        )?;
    }

    let applied = ledger.count(Outcome::Applied);
    let counts = format!(
        "{} unchanged, {} skipped, {} unavailable, {} attention, {} failed",
        ledger.count(Outcome::Unchanged),
        ledger.count(Outcome::Skipped),
        ledger.count(Outcome::Unavailable),
        ledger.count(Outcome::Attention),
        ledger.count(Outcome::Failed),
    );
    let verb = if dry_run {
        "Dry run: would apply"
    } else {
        "Applied"
    };
    writeln!(out, "{verb} {applied} optimizations, {counts}")?;

    session.end(&SessionSummary {
        items: u64::try_from(applied).map_err(io::Error::other)?,
        // No optimize task frees measured space; the operation log requires a size field.
        size: "0B".to_owned(),
    })?;
    Ok(ledger)
}

/// A preview row says what would happen, never what happened. History and the ledger keep
/// the six-state vocabulary; only the displayed row differs in dry runs.
fn row_label(outcome: Outcome, dry_run: bool) -> &'static str {
    match (outcome, dry_run) {
        (Outcome::Applied, true) => "would apply",
        _ => outcome.as_str(),
    }
}

fn ledger_error(error: OutcomeError) -> io::Error {
    io::Error::other(format!("optimize ledger: {error:?}"))
}
