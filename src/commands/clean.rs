use std::collections::BTreeSet;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rustix::fs::statvfs;

use crate::core::oplog::{OpLog, Session, SessionSummary};
use crate::core::paths::XdgPaths;
use crate::core::proc::ProcessState;
use crate::core::sink::{DeleteMode, Identity, SinkContext, delete};
use crate::core::whitelist::Whitelist;
use crate::rules::clean::{OwnerCheck, RecoveryMode, Rule, catalog};

#[derive(Debug, Default)]
pub struct CleanArgs {
    pub dry_run: bool,
    pub categories: Option<String>,
    pub select: Option<String>,
    pub exclude: Option<String>,
    pub external: bool,
    pub whitelist: bool,
}

pub fn run(args: CleanArgs) -> io::Result<()> {
    let xdg = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    if args.whitelist {
        let path = Whitelist::path(&xdg);
        println!("Whitelist: {}", display_path(&path));
        return Ok(());
    }
    if args.external {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--external requires an explicit path, which is not supported yet",
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
    let sink = SinkContext::new(xdg.clone(), whitelist);
    let selected = select_rules(catalog(), &args)?;
    let before = available_bytes(&xdg.home);
    let log = OpLog::new()?;
    let mut session = log.start_session("clean", args.dry_run)?;
    let mut summary = Summary::default();
    let result = run_rules(
        &selected,
        args.dry_run,
        &sink,
        &xdg,
        &mut session,
        &mut summary,
    );
    let after = available_bytes(&xdg.home);
    finish_session(&mut session, &summary)?;
    render_summary(args.dry_run, &summary, before, after);
    result
}

#[derive(Default)]
struct Summary {
    rules: u64,
    items: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    outcomes: [u64; 6],
    cancelled: bool,
}

fn run_rules(
    rules: &[Rule],
    dry_run: bool,
    sink: &SinkContext,
    xdg: &XdgPaths,
    session: &mut Session,
    summary: &mut Summary,
) -> io::Result<()> {
    for rule in rules {
        summary.rules = summary.rules.saturating_add(1);
        println!("{}", rule.category);
        for path in &rule.targets {
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    record(session, summary, "unavailable", path)?;
                    eprintln!("  {}: {error}", display_path(path));
                    continue;
                }
            };
            match owner_state(rule.owner_check) {
                ProcessState::Running => {
                    record(session, summary, "skipped", path)?;
                    continue;
                }
                ProcessState::Unknown => {
                    record(session, summary, "unavailable", path)?;
                    continue;
                }
                ProcessState::NotRunning => {}
            }
            let identity = match Identity::capture(path) {
                Ok(identity) => identity,
                Err(error) => {
                    record(session, summary, "unavailable", path)?;
                    eprintln!("  {}: {error}", display_path(path));
                    continue;
                }
            };
            let logical = if metadata.is_file() {
                metadata.len()
            } else {
                0
            };
            let allocated = metadata.blocks().saturating_mul(512);
            summary.logical_bytes = summary.logical_bytes.saturating_add(logical);
            summary.allocated_bytes = summary.allocated_bytes.saturating_add(allocated);
            let mode = if dry_run {
                DeleteMode::DryRun
            } else {
                DeleteMode::Trash
            };
            match delete(path, &identity, mode, sink) {
                Ok(_) => {
                    summary.items = summary.items.saturating_add(1);
                    record(session, summary, "applied", path)?;
                    println!(
                        "  {} ({})",
                        display_path(path),
                        recovery_name(rule.recovery)
                    );
                }
                Err(error) => {
                    record(session, summary, "failed", path)?;
                    eprintln!("  {}: {}", display_path(path), error);
                }
            }
        }
    }
    let _ = xdg;
    Ok(())
}

fn select_rules(rules: Vec<Rule>, args: &CleanArgs) -> io::Result<Vec<Rule>> {
    let categories = parse_filter(args.categories.as_deref())?;
    let selected = parse_filter(args.select.as_deref())?;
    let excluded = parse_filter(args.exclude.as_deref())?;
    let mut known = BTreeSet::new();
    for rule in &rules {
        known.insert(rule.category.to_owned());
        known.insert(rule.id.to_owned());
    }
    for requested in categories
        .iter()
        .chain(selected.iter())
        .chain(excluded.iter())
    {
        if !known.is_empty() && !known.contains(requested) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown clean rule or category: {requested}"),
            ));
        }
    }
    Ok(rules
        .into_iter()
        .filter(|rule| {
            let in_categories = categories.is_empty() || categories.contains(rule.category);
            let in_selected = selected.is_empty()
                || selected.contains(rule.id)
                || selected.contains(rule.category);
            in_categories
                && in_selected
                && !excluded.contains(rule.id)
                && !excluded.contains(rule.category)
        })
        .collect())
}

fn parse_filter(value: Option<&str>) -> io::Result<BTreeSet<String>> {
    let mut filters = BTreeSet::new();
    if let Some(value) = value {
        filters.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned),
        );
    }
    Ok(filters)
}

fn record(
    session: &mut Session,
    summary: &mut Summary,
    outcome: &str,
    path: &Path,
) -> io::Result<()> {
    let index = match outcome {
        "applied" => 0,
        "unchanged" => 1,
        "skipped" => 2,
        "unavailable" => 3,
        "attention" => 4,
        _ => 5,
    };
    summary.outcomes[index] = summary.outcomes[index].saturating_add(1);
    session.record("CLEAN", outcome, path)?;
    Ok(())
}

fn finish_session(session: &mut Session, summary: &Summary) -> io::Result<()> {
    session.end(&SessionSummary {
        items: summary.items,
        size: format!("{}B", summary.logical_bytes),
    })
}

fn owner_state(owner: OwnerCheck) -> ProcessState {
    match owner {
        OwnerCheck::None => ProcessState::NotRunning,
        OwnerCheck::ProcessName(_) => ProcessState::Unknown,
    }
}

fn recovery_name(mode: RecoveryMode) -> &'static str {
    match mode {
        RecoveryMode::Regenerate => "regenerable",
        RecoveryMode::Trash => "Trash",
        RecoveryMode::OwnerNative => "owner-managed",
        RecoveryMode::Irreversible => "irreversible",
    }
}

fn available_bytes(path: &Path) -> io::Result<u128> {
    let stats = statvfs(path).map_err(io::Error::from)?;
    Ok(u128::from(stats.f_bavail) * u128::from(stats.f_frsize))
}

fn render_summary(
    dry_run: bool,
    summary: &Summary,
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
    println!("Rules: {} · items: {}", summary.rules, summary.items);
    println!(
        "Logical size: {} bytes · allocated size: {} bytes",
        summary.logical_bytes, summary.allocated_bytes
    );
    match (before, after) {
        (Ok(before), Ok(after)) if after >= before => {
            println!("Measured space change: +{} bytes", after - before)
        }
        (Ok(before), Ok(after)) => println!("Measured space change: -{} bytes", before - after),
        _ => println!("Measured space change: unavailable"),
    }
    if summary.cancelled {
        println!("Cleanup cancelled; remaining rules were skipped");
    }
    println!(
        "Outcomes: applied={} unchanged={} skipped={} unavailable={} attention={} failed={}",
        summary.outcomes[0],
        summary.outcomes[1],
        summary.outcomes[2],
        summary.outcomes[3],
        summary.outcomes[4],
        summary.outcomes[5]
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_match_ids_and_categories_and_exclusions_win() {
        let rules = vec![
            Rule {
                id: "one",
                category: "User",
                evidence: "evidence",
                non_targets: &[],
                owner_check: OwnerCheck::None,
                recovery: RecoveryMode::Trash,
                targets: vec![],
            },
            Rule {
                id: "two",
                category: "User",
                evidence: "evidence",
                non_targets: &[],
                owner_check: OwnerCheck::None,
                recovery: RecoveryMode::Regenerate,
                targets: vec![],
            },
        ];
        let args = CleanArgs {
            categories: Some("User".to_owned()),
            select: Some("one,two".to_owned()),
            exclude: Some("two".to_owned()),
            ..CleanArgs::default()
        };
        let filtered = select_rules(rules, &args).expect("filter rules");
        assert_eq!(
            filtered.iter().map(|rule| rule.id).collect::<Vec<_>>(),
            vec!["one"]
        );
    }
}
