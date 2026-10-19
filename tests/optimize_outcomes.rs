//! Port of tw93/Mole tests/optimize_outcomes.bats (six-state outcome contract).
//! Upstream: lib/optimize/outcomes.sh (GPL-3.0). Rust makes invalid outcome names
//! unrepresentable, so the upstream "invalid outcome" case is covered by the type.

use mole_linux::core::oplog::OUTCOME_NAMES;
use mole_linux::core::outcome::{Outcome, OutcomeError, OutcomeLedger};

#[test]
fn ledger_records_one_outcome_per_task() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start");
    ledger.report(Outcome::Applied).expect("report");
    ledger.finish("font_cache_refresh").expect("finish");

    assert_eq!(ledger.count(Outcome::Applied), 1);
    assert_eq!(ledger.count(Outcome::Unchanged), 0);
    assert_eq!(ledger.total(), 1);
}

#[test]
fn attention_is_reported_separately_from_failure() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start");
    ledger.report(Outcome::Attention).expect("report");
    ledger.finish("failed_user_units_report").expect("finish");

    assert_eq!(ledger.count(Outcome::Attention), 1);
    assert_eq!(ledger.count(Outcome::Failed), 0);
}

#[test]
fn failures_take_precedence_over_partial_changes() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start");
    ledger.report_from_counts(2, 1, 0).expect("counts report");
    ledger.finish("desktop_mime_refresh").expect("finish");

    assert_eq!(ledger.count(Outcome::Failed), 1);
    assert_eq!(ledger.count(Outcome::Applied), 0);
}

#[test]
fn run_succeeds_with_attention_but_not_with_failure() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start");
    ledger.report(Outcome::Attention).expect("report");
    ledger.finish("journal_health_report").expect("finish");
    assert!(ledger.succeeded());

    ledger.start().expect("start");
    ledger.report(Outcome::Failed).expect("report");
    ledger.finish("font_cache_refresh").expect("finish");
    assert!(!ledger.succeeded());
}

#[test]
fn ledger_rejects_results_outside_an_active_task() {
    let mut ledger = OutcomeLedger::default();

    assert_eq!(
        ledger.report(Outcome::Applied),
        Err(OutcomeError::TaskNotActive)
    );
}

#[test]
fn ledger_rejects_missing_and_duplicate_task_records() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start");
    assert_eq!(
        ledger.finish("journal_health_report"),
        Err(OutcomeError::MissingOutcome)
    );

    ledger.report(Outcome::Unavailable).expect("report");
    ledger.finish("journal_health_report").expect("finish");
    ledger.start().expect("start");
    ledger.report(Outcome::Unchanged).expect("report");
    assert_eq!(
        ledger.finish("journal_health_report"),
        Err(OutcomeError::DuplicateAction(
            "journal_health_report".to_owned()
        ))
    );
}

#[test]
fn failed_actions_are_listed_in_run_order() {
    let mut ledger = OutcomeLedger::default();
    for (action, outcome) in [
        ("font_cache_refresh", Outcome::Applied),
        ("desktop_mime_refresh", Outcome::Failed),
        ("failed_user_units_report", Outcome::Attention),
        ("journal_health_report", Outcome::Failed),
    ] {
        ledger.start().expect("start");
        ledger.report(outcome).expect("report");
        ledger.finish(action).expect("finish");
    }

    assert_eq!(
        ledger.failed_actions(),
        vec!["desktop_mime_refresh", "journal_health_report"]
    );
}

#[test]
fn outcome_names_are_the_history_vocabulary() {
    for outcome in [
        Outcome::Applied,
        Outcome::Unchanged,
        Outcome::Skipped,
        Outcome::Unavailable,
        Outcome::Attention,
        Outcome::Failed,
    ] {
        assert!(
            OUTCOME_NAMES.contains(&outcome.as_str()),
            "{outcome:?} is not accepted by the operation log"
        );
    }
}

#[test]
fn outcome_from_counts_gives_failure_precedence_then_change_then_skip() {
    assert_eq!(Outcome::from_counts(2, 1, 0), Outcome::Failed);
    assert_eq!(Outcome::from_counts(1, 0, 3), Outcome::Applied);
    assert_eq!(Outcome::from_counts(0, 0, 2), Outcome::Skipped);
    assert_eq!(Outcome::from_counts(0, 0, 0), Outcome::Unchanged);
}
