use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use rustix::fs::statvfs;

use crate::adapters::run_owner_action;
use crate::core::oplog::{OpLog, Session, SessionSummary};
use crate::core::outcome::Outcome;
use crate::core::paths::XdgPaths;
use crate::core::privilege::{
    Consent, OwnerCommand, OwnerRun, OwnerRunner, RunMode, ask_owner_consent,
};
use crate::core::proc::{self, ProcessState};
use crate::core::sink::{DeleteMode, Identity, SinkContext, delete};
use crate::core::validate::validate_path_for_deletion;
use crate::core::whitelist::Whitelist;
use crate::rules::clean::{
    Candidate, Category, CleanRule, DiscoveryStatus, RecoveryMode, RuleContext, catalog,
};
use crate::scan::{self, Node, NodeStatus};

static CANCEL_SIGNAL: AtomicI32 = AtomicI32::new(0);
static CANCELLED: AtomicBool = AtomicBool::new(false);
const SIZE_SCAN_BUDGET: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct CleanArgs {
    pub dry_run: bool,
    pub yes: bool,
    pub categories: Option<String>,
    pub select: Option<String>,
    pub exclude: Option<String>,
    pub external: bool,
    pub whitelist: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanSummary {
    pub discovered: u64,
    pub applied: u64,
    pub unchanged: u64,
    pub skipped: u64,
    pub unavailable: u64,
    pub attention: u64,
    pub failed: u64,
    pub protected: u64,
    pub allocated_applied: Option<u64>,
    pub cancelled: bool,
}

impl Default for CleanSummary {
    fn default() -> Self {
        Self {
            discovered: 0,
            applied: 0,
            unchanged: 0,
            skipped: 0,
            unavailable: 0,
            attention: 0,
            failed: 0,
            protected: 0,
            allocated_applied: Some(0),
            cancelled: false,
        }
    }
}

pub fn run(args: CleanArgs) -> io::Result<()> {
    let xdg = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    if args.whitelist {
        println!("Whitelist: {}", display_path(&Whitelist::path(&xdg)));
        return Ok(());
    }
    if args.external {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external targets are not registered by the Linux clean catalog",
        ));
    }
    let whitelist = Whitelist::load(&xdg)
        .map_err(|error| io::Error::other(format!("failed to load whitelist: {error:?}")))?;
    for warning in &whitelist.warnings {
        eprintln!(
            "mole-linux: whitelist line {} ignored: {}",
            warning.line, warning.reason
        );
    }
    let process_probe = |name: &str| proc::is_process_name_running(Path::new("/proc"), name);
    CANCEL_SIGNAL.store(0, Ordering::Relaxed);
    CANCELLED.store(false, Ordering::Relaxed);
    install_signal_handlers()?;
    let context = RuleContext {
        home: &xdg.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &CANCELLED,
    };
    let rules = catalog();
    let rule_refs: Vec<&dyn CleanRule> = rules.iter().map(Box::as_ref).collect();
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let before = available_bytes(&xdg.home);
    let mut session = OpLog::new()?.start_session("clean", args.dry_run)?;
    let result = execute_rules(&args, &rule_refs, &context, &sink, &mut session);
    let after = available_bytes(&xdg.home);
    let mut summary = match result {
        Ok(summary) => summary,
        Err(error) => {
            let empty = CleanSummary::default();
            session.end(&session_summary(&empty))?;
            return Err(error);
        }
    };
    summary.cancelled |= CANCEL_SIGNAL.load(Ordering::Relaxed) != 0;
    session.end(&session_summary(&summary))?;
    render_summary(args.dry_run, &summary, before, after);
    if summary.cancelled {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "clean interrupted",
        ));
    }
    Ok(())
}

/// Discovers, previews, validates, confirms, and sends eligible items through the sink.
/// Owner actions run through the system `OwnerRunner`; `--yes` grants their consent up front.
pub fn execute_rules(
    args: &CleanArgs,
    rules: &[&dyn CleanRule],
    context: &RuleContext<'_>,
    sink: &SinkContext,
    session: &mut Session,
) -> io::Result<CleanSummary> {
    let consent = if args.yes {
        Consent::Granted
    } else {
        Consent::Unasked
    };
    let mut owner = OwnerRunner::system(consent);
    execute_rules_with_owner(
        args,
        rules,
        context,
        sink,
        session,
        &mut owner,
        &ask_owner_consent,
    )
}

/// `execute_rules` with the owner runner and consent prompt injected. The prompt receives the
/// exact argv of every owner command selected for this run and is asked at most once.
pub fn execute_rules_with_owner(
    args: &CleanArgs,
    rules: &[&dyn CleanRule],
    context: &RuleContext<'_>,
    sink: &SinkContext,
    session: &mut Session,
    owner: &mut OwnerRunner,
    prompt: &dyn Fn(&[Vec<OsString>]) -> bool,
) -> io::Result<CleanSummary> {
    let categories = parse_filter(args.categories.as_deref());
    let selected = parse_filter(args.select.as_deref());
    let excluded = parse_filter(args.exclude.as_deref());
    let known = rules
        .iter()
        .flat_map(|rule| [rule.id().to_owned(), rule.category().as_str().to_owned()])
        .collect::<BTreeSet<_>>();
    for value in categories
        .iter()
        .chain(selected.iter())
        .chain(excluded.iter())
    {
        if !known.is_empty() && !known.contains(value) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown clean rule or category: {value}"),
            ));
        }
    }

    let mut summary = CleanSummary::default();
    let mut candidates: Vec<(String, Candidate)> = Vec::new();
    for rule in rules {
        if context.cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let id = rule.id();
        let category = rule.category();
        if excluded.contains(id) || excluded.contains(category.as_str()) {
            continue;
        }
        if !categories.is_empty() && !categories.contains(category.as_str()) {
            continue;
        }
        if !selected.is_empty() && !selected.contains(id) && !selected.contains(category.as_str()) {
            continue;
        }
        let discovery = rule.discover_with_owner(context, owner);
        match &discovery.status {
            DiscoveryStatus::Complete => {}
            DiscoveryStatus::Partial(reason) => println!(
                "{}: partial discovery ({})",
                category.as_str(),
                escape_text(reason)
            ),
            DiscoveryStatus::Unavailable(reason) => {
                println!(
                    "{}: unavailable ({})",
                    category.as_str(),
                    escape_text(reason)
                );
                record_outcome(&mut summary, "unavailable");
                continue;
            }
        }
        for candidate in discovery.candidates {
            candidates.push((id.to_owned(), candidate));
        }
    }
    deduplicate_candidates(&mut candidates);
    summary.discovered = u64::try_from(candidates.len())
        .map_err(|_| io::Error::other("candidate count does not fit summary"))?;
    let size_scan_deadline = Instant::now() + SIZE_SCAN_BUDGET;

    let mut apply = Vec::new();
    for (rule_id, mut candidate) in candidates {
        if context.cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            record(&mut summary, session, "skipped", &candidate.path)?;
            continue;
        }
        if let Some(reason) = protected_reason(&candidate, context) {
            summary.protected = summary.protected.saturating_add(1);
            summary.skipped = summary.skipped.saturating_add(1);
            println!(
                "Protected: {} ({})",
                display_path(&candidate.path),
                escape_text(&reason)
            );
            record(&mut summary, session, "skipped", &candidate.path)?;
            continue;
        }
        // Owner-native targets are whole cache directories the owner tool trims, so a directory size would overstate the action.
        if candidate.recovery != RecoveryMode::OwnerNative
            && (candidate.apparent_size.is_none() || candidate.allocated_size.is_none())
        {
            measure_candidate_size(&mut candidate, size_scan_deadline, context.cancel);
        }
        if context.cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            record(&mut summary, session, "skipped", &candidate.path)?;
            continue;
        }
        if candidate.owner_state != ProcessState::NotRunning {
            candidate.selected_by_default = false;
            let (outcome, state_reason) = match candidate.owner_state {
                ProcessState::Running => ("skipped", "owner process is running"),
                ProcessState::Unknown => ("unavailable", "owner process state is unknown"),
                ProcessState::NotRunning => unreachable!(),
            };
            record(&mut summary, session, outcome, &candidate.path)?;
            println!(
                "{}: {} ({}) · logical={} · allocated={}",
                display_path(&candidate.path),
                escape_text(&candidate.reason),
                state_reason,
                format_size(candidate.apparent_size),
                format_size(candidate.allocated_size)
            );
            continue;
        }
        let explicitly_selected = selected.contains(&rule_id)
            || selected.contains(category_for_rule(rules, &rule_id).as_str())
            || selected.contains(candidate.path.to_string_lossy().as_ref());
        // Owner actions run with sudo, so only an explicit --select runs them; sink items keep the default.
        let selected_for_action = explicitly_selected
            || (candidate.selected_by_default && candidate.recovery != RecoveryMode::OwnerNative);
        println!(
            "{} [{}] {} · {} · logical={} · allocated={}",
            if selected_for_action {
                "Selected"
            } else {
                "Not selected"
            },
            category_for_rule(rules, &rule_id).as_str(),
            display_path(&candidate.path),
            escape_text(&candidate.reason),
            format_size(candidate.apparent_size),
            format_size(candidate.allocated_size)
        );
        if selected_for_action {
            apply.push((rule_id, candidate));
        } else if args.dry_run {
            record(&mut summary, session, "unchanged", &candidate.path)?;
        } else {
            record(&mut summary, session, "skipped", &candidate.path)?;
        }
    }

    if !args.dry_run && !apply.is_empty() && !args.yes && !confirm()? {
        summary.cancelled = true;
        for (_, candidate) in apply {
            record(&mut summary, session, "skipped", &candidate.path)?;
        }
        return Ok(summary);
    }

    if args.dry_run {
        for (_, candidate) in apply {
            record(&mut summary, session, "unchanged", &candidate.path)?;
        }
    } else {
        let listing: Vec<Vec<OsString>> = apply
            .iter()
            .filter_map(|(rule_id, candidate)| owner_command_for(rules, rule_id, candidate))
            .map(|command| owner.argv(&command))
            .collect();
        let mut sized_objects = BTreeSet::new();
        for (rule_id, candidate) in apply {
            if context.cancel.load(Ordering::Relaxed) {
                summary.cancelled = true;
                record(&mut summary, session, "skipped", &candidate.path)?;
                continue;
            }
            let owner_state = context.check_owner(&candidate.owner_check);
            if owner_state != ProcessState::NotRunning {
                let (outcome, reason) = match owner_state {
                    ProcessState::Running => ("skipped", "owner process is running"),
                    ProcessState::Unknown => ("unavailable", "owner process state is unknown"),
                    ProcessState::NotRunning => unreachable!(),
                };
                record(&mut summary, session, outcome, &candidate.path)?;
                println!("{}: {}", display_path(&candidate.path), reason);
                continue;
            }
            if let Some(reason) = final_guard_refusal(rules, &rule_id, &candidate, context) {
                record(&mut summary, session, "skipped", &candidate.path)?;
                println!(
                    "{}: {}",
                    display_path(&candidate.path),
                    escape_text(&reason)
                );
                continue;
            }
            if let Some(action) = candidate.owner_action.clone() {
                // The owner command acts on the directory that was previewed; refuse if it changed.
                if Identity::capture(&candidate.path).ok().as_ref() != Some(&candidate.identity) {
                    record(&mut summary, session, "skipped", &candidate.path)?;
                    println!(
                        "{}: changed since preview; owner command skipped",
                        display_path(&candidate.path)
                    );
                    continue;
                }
                let (outcome, reason) = run_owner_action(&action);
                let name = match outcome {
                    Outcome::Applied => "applied",
                    Outcome::Unavailable => "unavailable",
                    _ => "failed",
                };
                record_outcome(&mut summary, name);
                session.record("CLEAN", name, &candidate.path)?;
                println!(
                    "{}: {}",
                    display_path(&candidate.path),
                    escape_text(&reason)
                );
                continue;
            }
            let mode = match candidate.recovery {
                RecoveryMode::OwnerNative => {
                    let run = match owner_command_for(rules, &rule_id, &candidate) {
                        Some(command) => owner.run(&command, RunMode::Apply, || prompt(&listing)),
                        None => OwnerRun {
                            outcome: Outcome::Unavailable,
                            argv: Vec::new(),
                            reason: Some("owner-native action not registered".to_owned()),
                        },
                    };
                    record_owner_run(&mut summary, session, &candidate.path, &run)?;
                    if run.outcome == Outcome::Applied {
                        // Owner tools expose no per-item sizes, so the summed total is unknown.
                        summary.allocated_applied = None;
                    }
                    continue;
                }
                RecoveryMode::Irreversible => DeleteMode::Permanent,
                RecoveryMode::Regenerate | RecoveryMode::Trash => DeleteMode::Trash,
            };
            match delete(&candidate.path, &candidate.identity, mode, sink) {
                Ok(_) => {
                    record_outcome(&mut summary, "applied");
                    if sized_objects.insert(candidate.identity.object_key()) {
                        summary.allocated_applied =
                            match (summary.allocated_applied, candidate.allocated_size) {
                                (Some(total), Some(size)) => total.checked_add(size),
                                _ => None,
                            };
                    }
                    session.record("CLEAN", "applied", &candidate.path)?;
                }
                Err(error) => {
                    record_outcome(&mut summary, "failed");
                    println!(
                        "Failed: {} ({})",
                        display_path(&candidate.path),
                        escape_text(&error.to_string())
                    );
                    session.record("CLEAN", "failed", &candidate.path)?;
                }
            }
        }
    }
    Ok(summary)
}

/// The owning rule re-checks a candidate right before the sink; `Some` keeps it.
fn final_guard_refusal(
    rules: &[&dyn CleanRule],
    rule_id: &str,
    candidate: &Candidate,
    context: &RuleContext<'_>,
) -> Option<String> {
    let rule = rules.iter().find(|rule| rule.id() == rule_id)?;
    rule.final_guard(candidate, context).err()
}

fn protected_reason(candidate: &Candidate, context: &RuleContext<'_>) -> Option<String> {
    if context.whitelist.matches(&candidate.path)
        || candidate
            .path
            .canonicalize()
            .ok()
            .is_some_and(|resolved| context.whitelist.matches(&resolved))
    {
        return Some("whitelist".to_owned());
    }
    if candidate.recovery == RecoveryMode::OwnerNative {
        // Owner tools choose their own targets, outside HOME; the HOME policy guards sink deletes only.
        return None;
    }
    validate_path_for_deletion(&candidate.path, context.home, context.xdg)
        .err()
        .map(|error| format!("policy: {error:?}"))
}

fn measure_candidate_size(candidate: &mut Candidate, deadline: Instant, cancel: &AtomicBool) {
    match scan_candidate(&candidate.path, deadline, cancel) {
        Ok((apparent, allocated)) => {
            candidate.apparent_size.get_or_insert(apparent);
            candidate.allocated_size.get_or_insert(allocated);
        }
        Err(reason) => {
            candidate.reason.push_str("; size unavailable: ");
            candidate.reason.push_str(&reason);
        }
    }
}

fn scan_candidate(
    path: &Path,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<(u64, u64), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("scan cancelled".to_owned());
    }
    if Instant::now() >= deadline {
        return Err("per-run size scan deadline exceeded".to_owned());
    }

    let scan_cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&scan_cancel);
    let scan_path = path.to_path_buf();
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("clean-size-scan".to_owned())
        .spawn(move || {
            let report = scan::scan(&scan_path, &worker_cancel);
            if sender.send(report).is_err() {
                worker_cancel.store(true, Ordering::Relaxed);
            }
        })
        .map_err(|error| format!("could not start size scan: {error}"))?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            scan_cancel.store(true, Ordering::Relaxed);
            return Err("scan cancelled".to_owned());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            scan_cancel.store(true, Ordering::Relaxed);
            return Err("per-run size scan deadline exceeded".to_owned());
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(report) => {
                if Instant::now() >= deadline {
                    scan_cancel.store(true, Ordering::Relaxed);
                    return Err("per-run size scan deadline exceeded".to_owned());
                }
                match report.root.status {
                    NodeStatus::Complete => {
                        return Ok((report.root.apparent, report.root.allocated));
                    }
                    NodeStatus::Unavailable(reason) => {
                        return Err(format!("scan unavailable: {reason}"));
                    }
                    NodeStatus::Partial => {
                        return Err(incomplete_scan_reason(&report.root, deadline));
                    }
                    NodeStatus::Excluded(reason) => {
                        return Err(format!("scan excluded: {reason}"));
                    }
                    NodeStatus::Scanning => {
                        return Err("scan did not complete".to_owned());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("size scan worker stopped without a result".to_owned());
            }
        }
    }
}

fn incomplete_scan_reason(root: &Node, deadline: Instant) -> String {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if Instant::now() >= deadline {
            return "per-run size scan deadline exceeded".to_owned();
        }
        match &node.status {
            NodeStatus::Unavailable(reason) => {
                return format!("scan incomplete: {reason}");
            }
            _ => pending.extend(node.children.iter()),
        }
    }
    "scan incomplete; an entry could not be read or permission was denied".to_owned()
}

fn deduplicate_candidates(candidates: &mut Vec<(String, Candidate)>) {
    candidates.sort_by_key(|(_, candidate)| candidate.path.components().count());
    let mut accepted: Vec<(String, Candidate)> = Vec::new();
    for entry in candidates.drain(..) {
        // Owner actions share a cache directory but are distinct commands, so they never collapse.
        let owner_native = entry.1.recovery == RecoveryMode::OwnerNative;
        if !owner_native
            && accepted.iter().any(|(_, existing)| {
                existing.recovery != RecoveryMode::OwnerNative
                    && entry.1.path.starts_with(&existing.path)
            })
        {
            continue;
        }
        accepted.push(entry);
    }
    *candidates = accepted;
}

fn owner_command_for(
    rules: &[&dyn CleanRule],
    rule_id: &str,
    candidate: &Candidate,
) -> Option<OwnerCommand> {
    if candidate.recovery != RecoveryMode::OwnerNative {
        return None;
    }
    rules
        .iter()
        .find(|rule| rule.id() == rule_id)?
        .owner_command(candidate)
}

fn record_owner_run(
    summary: &mut CleanSummary,
    session: &mut Session,
    path: &Path,
    run: &OwnerRun,
) -> io::Result<()> {
    let outcome = match run.outcome {
        Outcome::Applied => "applied",
        Outcome::Unchanged => "unchanged",
        Outcome::Skipped => "skipped",
        Outcome::Unavailable => "unavailable",
        Outcome::Attention => "attention",
        Outcome::Failed => "failed",
    };
    if let Some(reason) = &run.reason {
        println!(
            "{}: {} ({})",
            display_path(path),
            outcome,
            escape_text(reason)
        );
    }
    record(summary, session, outcome, path)
}

fn category_for_rule(rules: &[&dyn CleanRule], id: &str) -> Category {
    rules
        .iter()
        .find(|rule| rule.id() == id)
        .map_or(Category::User, |rule| rule.category())
}

fn parse_filter(value: Option<&str>) -> BTreeSet<String> {
    value
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn confirm() -> io::Result<bool> {
    if !io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clean apply requires a terminal confirmation or --yes",
        ));
    }
    print!("Apply cleanup to the selected items? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}

fn record_outcome(summary: &mut CleanSummary, outcome: &str) {
    let counter = match outcome {
        "applied" => &mut summary.applied,
        "unchanged" => &mut summary.unchanged,
        "skipped" => &mut summary.skipped,
        "unavailable" => &mut summary.unavailable,
        "attention" => &mut summary.attention,
        _ => &mut summary.failed,
    };
    *counter = counter.saturating_add(1);
}

fn record(
    summary: &mut CleanSummary,
    session: &mut Session,
    outcome: &str,
    path: &Path,
) -> io::Result<()> {
    record_outcome(summary, outcome);
    session.record("CLEAN", outcome, path)
}

fn session_summary(summary: &CleanSummary) -> SessionSummary {
    SessionSummary {
        items: summary.applied,
        size: summary
            .allocated_applied
            .map_or_else(|| "unknown".to_owned(), |size| format!("{size}B")),
    }
}

fn available_bytes(path: &Path) -> io::Result<u128> {
    let stats = statvfs(path).map_err(io::Error::from)?;
    Ok(u128::from(stats.f_bavail) * u128::from(stats.f_frsize))
}

fn render_summary(
    dry_run: bool,
    summary: &CleanSummary,
    before: io::Result<u128>,
    after: io::Result<u128>,
) {
    println!(
        "{}",
        if dry_run {
            "Clean preview complete"
        } else {
            "Clean complete"
        }
    );
    println!(
        "Discovered: {} · applied: {} · unchanged: {} · skipped: {} · unavailable: {} · attention: {} · failed: {}",
        summary.discovered,
        summary.applied,
        summary.unchanged,
        summary.skipped,
        summary.unavailable,
        summary.attention,
        summary.failed
    );
    println!(
        "Summed allocated size of applied items: {}",
        format_size(summary.allocated_applied)
    );
    if !dry_run {
        match (before, after) {
            (Ok(before), Ok(after)) if after >= before => println!(
                "Measured statvfs free-space delta: +{} bytes",
                after - before
            ),
            (Ok(before), Ok(after)) => println!(
                "Measured statvfs free-space delta: -{} bytes",
                before - after
            ),
            _ => println!("Measured statvfs free-space delta: unavailable"),
        }
    }
    if summary.cancelled {
        println!("Cleanup cancelled; remaining items were skipped");
    }
}

fn format_size(size: Option<u64>) -> String {
    size.map_or_else(|| "unknown".to_owned(), |bytes| format!("{bytes} bytes"))
}

fn display_path(path: &Path) -> String {
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

fn escape_text(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

fn install_signal_handlers() -> io::Result<()> {
    unsafe extern "C" {
        fn signal(signal: i32, handler: usize) -> usize;
    }
    extern "C" fn mark_cancelled(signal_number: i32) {
        CANCELLED.store(true, Ordering::Relaxed);
        CANCEL_SIGNAL.store(signal_number, Ordering::Relaxed);
    }
    // SAFETY: the handler only writes to an atomic integer and remains valid for the process lifetime.
    let previous = unsafe { signal(2, mark_cancelled as *const () as usize) };
    if previous == usize::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn comma_filter_trims_and_ignores_empty_parts() {
        assert_eq!(
            parse_filter(Some(" a, ,b ")),
            BTreeSet::from(["a".to_owned(), "b".to_owned()])
        );
    }

    #[test]
    fn expired_size_scan_deadline_leaves_sizes_unknown_with_a_reason() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("cache");
        std::fs::create_dir(&path).expect("create cache");
        let mut candidate = crate::rules::clean::candidate(
            &path,
            "test cache",
            crate::rules::clean::RecoveryMode::Regenerate,
            ProcessState::NotRunning,
        )
        .expect("capture cache");

        measure_candidate_size(
            &mut candidate,
            Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("past instant"),
            &AtomicBool::new(false),
        );

        assert_eq!(candidate.apparent_size, None);
        assert_eq!(candidate.allocated_size, None);
        assert!(candidate.reason.contains("size unavailable"));
        assert!(candidate.reason.contains("deadline exceeded"));
    }

    #[test]
    fn unavailable_size_scan_leaves_sizes_unknown_with_scan_error() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("missing-cache");
        let mut candidate = crate::rules::clean::candidate(
            temp.path(),
            "test cache",
            crate::rules::clean::RecoveryMode::Regenerate,
            ProcessState::NotRunning,
        )
        .expect("capture cache");
        candidate.path = path;

        measure_candidate_size(
            &mut candidate,
            Instant::now() + Duration::from_secs(1),
            &AtomicBool::new(false),
        );

        assert_eq!(candidate.apparent_size, None);
        assert_eq!(candidate.allocated_size, None);
        assert!(candidate.reason.contains("size unavailable"));
        assert!(candidate.reason.contains("scan unavailable"));
    }
}
