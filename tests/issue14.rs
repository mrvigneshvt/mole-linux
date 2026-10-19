use std::fs;
use std::path::Path;

use mole_linux::core::outcome::{Outcome, OutcomeLedger};
use mole_linux::core::proc::{ProcessState, is_process_running};

#[test]
fn destructive_filesystem_calls_are_confined_to_the_sink() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowlist = [
        source_root.join("core/fsutil.rs"),
        source_root.join("core/sink.rs"),
    ];
    let mut violations = Vec::new();
    inspect_sources(&source_root, &allowlist, &mut violations);
    assert!(
        violations.is_empty(),
        "destructive filesystem calls outside the approved sink allowlist: {violations:?}"
    );
}

fn inspect_sources(
    directory: &Path,
    allowlist: &[std::path::PathBuf],
    violations: &mut Vec<String>,
) {
    for entry in fs::read_dir(directory).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("inspect source entry type");
        if file_type.is_dir() {
            inspect_sources(&path, allowlist, violations);
        } else if file_type.is_symlink() {
            violations.push(format!(
                "{}: symlinked source was not audited",
                path.display()
            ));
        } else if file_type.is_file()
            && path.extension().is_some_and(|extension| extension == "rs")
            && !allowlist.contains(&path)
        {
            let source_size = fs::metadata(&path).expect("inspect source metadata").len();
            assert!(
                source_size <= 1_048_576,
                "source file too large: {}",
                path.display()
            );
            let source = fs::read_to_string(&path).expect("read Rust source");
            for (line_number, line) in source.lines().enumerate() {
                if [
                    "remove_file(",
                    "remove_dir(",
                    "remove_dir_all(",
                    "::rename(",
                ]
                .iter()
                .any(|call| line.contains(call))
                {
                    violations.push(format!("{}:{}: {line}", path.display(), line_number + 1));
                }
            }
        }
    }
}

#[test]
fn process_probe_distinguishes_running_missing_and_unknown() {
    let temp = tempfile::tempdir().expect("temporary proc root");
    let proc_root = temp.path().join("proc");
    fs::create_dir_all(proc_root.join("1234")).expect("running process directory");
    fs::write(proc_root.join("1234/stat"), "1234 (worker) S 1 0 0\n")
        .expect("running process stat");
    fs::create_dir_all(&proc_root).expect("proc root");

    assert_eq!(is_process_running(&proc_root, 1234), ProcessState::Running);
    assert_eq!(
        is_process_running(&proc_root, 5678),
        ProcessState::NotRunning
    );
    assert_eq!(
        is_process_running(&temp.path().join("missing-proc"), 1234),
        ProcessState::Unknown
    );
    fs::create_dir_all(proc_root.join("99")).expect("malformed process directory");
    fs::write(proc_root.join("99/stat"), "malformed\n").expect("malformed proc stat");
    assert_eq!(is_process_running(&proc_root, 99), ProcessState::Unknown);
    fs::create_dir_all(proc_root.join("100")).expect("oversized process directory");
    let oversized_stat = format!("100 ({}) S 1 0 0\n", "x".repeat(5000));
    fs::write(proc_root.join("100/stat"), oversized_stat).expect("oversized proc stat");
    assert_eq!(is_process_running(&proc_root, 100), ProcessState::Unknown);
}

#[test]
fn process_probe_reads_the_host_procfs_for_a_live_process() {
    let pid = std::process::id();
    assert_eq!(
        is_process_running(Path::new("/proc"), pid),
        ProcessState::Running
    );
}

#[test]
fn outcome_ledger_records_one_validated_result_per_action() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start task");
    ledger.report(Outcome::Applied).expect("report applied");
    ledger.finish("network_optimization").expect("finish task");
    assert_eq!(ledger.count(Outcome::Applied), 1);
    assert_eq!(ledger.count(Outcome::Unchanged), 0);
    assert_eq!(ledger.total(), 1);

    ledger.start().expect("start second task");
    ledger.report(Outcome::Attention).expect("report attention");
    ledger
        .finish("login_items_audit")
        .expect("finish second task");
    assert_eq!(ledger.count(Outcome::Attention), 1);
    assert!(ledger.succeeded());
}

#[test]
fn outcome_ledger_validates_transitions_counts_and_unique_action_names() {
    let mut ledger = OutcomeLedger::default();
    assert!(ledger.report(Outcome::Applied).is_err());
    ledger.start().expect("start task");
    assert!(ledger.start().is_err());
    assert!(ledger.finish("missing_result").is_err());
    assert!(ledger.report_from_counts(2, 1, 0).is_ok());
    assert!(ledger.report(Outcome::Applied).is_err());
    assert!(ledger.finish("invalid-action").is_err());
    ledger.finish("sqlite_vacuum").expect("finish task");
    assert_eq!(ledger.count(Outcome::Failed), 1);
    assert_eq!(ledger.count(Outcome::Applied), 0);
    assert_eq!(ledger.failed_actions(), ["sqlite_vacuum"]);
    assert!(!ledger.succeeded());

    ledger.start().expect("start next task");
    ledger
        .report(Outcome::Unavailable)
        .expect("report unavailable");
    assert!(ledger.finish("sqlite_vacuum").is_err());
    ledger.finish("disk_check").expect("finish next task");
}

#[test]
fn outcome_count_resolution_prefers_failure_then_change_then_skip() {
    let mut ledger = OutcomeLedger::default();
    ledger.start().expect("start failed task");
    ledger
        .report_from_counts(1, 1, 0)
        .expect("failure wins over partial change");
    ledger
        .finish("partial_failure")
        .expect("finish failed task");
    ledger.start().expect("start changed task");
    ledger
        .report_from_counts(1, 0, 4)
        .expect("change wins over skip");
    ledger.finish("changed").expect("finish changed task");
    ledger.start().expect("start skipped task");
    ledger
        .report_from_counts(0, 0, 1)
        .expect("resolve skipped task");
    ledger.finish("skipped").expect("finish skipped task");
    ledger.start().expect("start unchanged task");
    ledger
        .report_from_counts(0, 0, 0)
        .expect("resolve unchanged task");
    ledger.finish("unchanged").expect("finish unchanged task");
    assert_eq!(ledger.count(Outcome::Failed), 1);
    assert_eq!(ledger.count(Outcome::Applied), 1);
    assert_eq!(ledger.count(Outcome::Skipped), 1);
    assert_eq!(ledger.count(Outcome::Unchanged), 1);
}
