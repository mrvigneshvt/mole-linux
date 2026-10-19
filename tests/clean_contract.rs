use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::time::SystemTime;

use mole_linux::commands::clean::{self, CleanArgs};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::{Identity, SinkContext};
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode,
    RuleContext,
};

struct TestRule {
    targets: Vec<PathBuf>,
    owner_check: OwnerCheck,
}

impl CleanRule for TestRule {
    fn id(&self) -> &str {
        "test-rule"
    }
    fn category(&self) -> Category {
        Category::User
    }
    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let candidates = self
            .targets
            .iter()
            .map(|path| {
                let meta = fs::symlink_metadata(path).expect("test candidate metadata");
                Candidate {
                    path: path.clone(),
                    identity: Identity::capture(path).expect("candidate identity"),
                    apparent_size: Some(meta.len()),
                    allocated_size: Some(meta.blocks().saturating_mul(512)),
                    reason: "test rule evidence".to_owned(),
                    recovery: RecoveryMode::Trash,
                    selected_by_default: true,
                    owner_state: ctx.check_owner(&self.owner_check),
                    owner_check: self.owner_check.clone(),
                }
            })
            .collect();
        Discovery {
            candidates,
            status: DiscoveryStatus::Complete,
        }
    }
}

fn setup(home: &Path) -> (XdgPaths, Whitelist, impl Fn(&str) -> ProcessState) {
    fs::create_dir_all(home).expect("create HOME");
    let xdg = XdgPaths::from_values(home.to_path_buf(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("load whitelist");
    (xdg, whitelist, |_| ProcessState::NotRunning)
}

#[test]
fn runner_discovers_and_applies_candidates_only_after_confirmation() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let target = home.join(".cache/test-target");
    fs::create_dir_all(target.parent().expect("parent")).expect("create target parent");
    fs::write(&target, b"safe test payload").expect("seed target");
    let (xdg, whitelist, process_probe) = setup(&home);
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rule = TestRule {
        targets: vec![target.clone()],
        owner_check: OwnerCheck::None,
    };
    let rules: [&dyn CleanRule; 1] = [&rule];
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("start session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let report = clean::execute_rules(
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        &rules,
        &context,
        &sink,
        &mut session,
    )
    .expect("execute rules");

    assert_eq!(report.applied, 1);
    assert_eq!(report.allocated_applied, Some(4096));
    assert!(!target.exists());
    assert!(home.join(".local/share/Trash/files/test-target").exists());
    let operations = fs::read_to_string(temp.path().join("state/mole-linux/operations.log"))
        .expect("operation log");
    assert!(operations.contains("CLEAN applied"));
}

#[test]
fn runner_lists_outside_home_candidates_as_protected_without_sink_mutation() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let target = temp.path().join("outside-canary");
    fs::create_dir_all(&home).expect("create HOME");
    fs::write(&target, b"must remain").expect("seed outside canary");
    let (xdg, whitelist, process_probe) = setup(&home);
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rule = TestRule {
        targets: vec![target.clone()],
        owner_check: OwnerCheck::None,
    };
    let rules: [&dyn CleanRule; 1] = [&rule];
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", true).expect("start session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let report = clean::execute_rules(
        &CleanArgs {
            dry_run: true,
            ..CleanArgs::default()
        },
        &rules,
        &context,
        &sink,
        &mut session,
    )
    .expect("preview rules");

    assert_eq!(report.protected, 1);
    assert!(target.exists());
}

#[test]
fn unknown_owner_state_is_visible_and_never_applied_even_with_yes() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let target = home.join(".cache/owned");
    fs::create_dir_all(target.parent().expect("parent")).expect("create target parent");
    fs::write(&target, b"keep while owner state is unknown").expect("seed target");
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("load whitelist");
    let process_probe = |_: &str| ProcessState::Unknown;
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rule = TestRule {
        targets: vec![target.clone()],
        owner_check: OwnerCheck::ProcessName("unknown-owner".to_owned()),
    };
    let rules: [&dyn CleanRule; 1] = [&rule];
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("start session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let report = clean::execute_rules(
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        &rules,
        &context,
        &sink,
        &mut session,
    )
    .expect("unknown owner remains protected");
    assert_eq!(report.unavailable, 1);
    assert!(target.exists());
}

#[test]
fn cancellation_before_discovery_returns_a_cancelled_summary_without_mutation() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let target = home.join(".cache/cancelled");
    fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
    fs::write(&target, b"keep").expect("seed target");
    let (xdg, whitelist, process_probe) = setup(&home);
    let cancel = AtomicBool::new(true);
    let context = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rule = TestRule {
        targets: vec![target.clone()],
        owner_check: OwnerCheck::None,
    };
    let rules: [&dyn CleanRule; 1] = [&rule];
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("start session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let report = clean::execute_rules(
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        &rules,
        &context,
        &sink,
        &mut session,
    )
    .expect("cancel clean");

    assert!(report.cancelled);
    assert_eq!(report.applied, 0);
    assert!(target.exists());
}

#[test]
fn owner_starting_after_discovery_is_rechecked_before_sink() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let target = home.join(".cache/owned");
    fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
    fs::write(&target, b"owner is about to start").expect("seed target");
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("load whitelist");
    let calls = AtomicUsize::new(0);
    let process_probe = |_: &str| {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            ProcessState::NotRunning
        } else {
            ProcessState::Running
        }
    };
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rule = TestRule {
        targets: vec![target.clone()],
        owner_check: OwnerCheck::ProcessName("owner-worker".to_owned()),
    };
    let rules: [&dyn CleanRule; 1] = [&rule];
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("start session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let report = clean::execute_rules(
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        &rules,
        &context,
        &sink,
        &mut session,
    )
    .expect("owner recheck");

    assert_eq!(report.skipped, 1);
    assert!(target.exists());
}
