//! M6.3 contracts: the `journal` system rule previews usage through `OwnerRunner` without
//! sudo, vacuums archived journals only after an explicit `--select` and one consent, and
//! reports every state it cannot decide as unavailable.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use mole_linux::commands::clean::{CleanArgs, CleanSummary, execute_rules_with_owner};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode, RuleContext,
};
use mole_linux::rules::journal::JournalRule;

const DISK_USAGE: &str = "Archived and active journals take up 286.8M in the file system.";

fn write_shim(dir: &Path, name: &str, log: &Path, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{name}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = log.display(),
    );
    let path = dir.join(name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    path
}

/// A sudo stand-in that drops a leading `--` and execs the rest, as sudo would.
fn sudo_shim(dir: &Path, log: &Path) -> PathBuf {
    write_shim(
        dir,
        "sudo",
        log,
        "if [ \"$1\" = -- ]; then shift; fi\nexec \"$@\"",
    )
}

/// `--disk-usage` prints the usage line; any `--vacuum-*` succeeds; anything else is a test bug.
fn journalctl_shim(bin: &Path, log: &Path) -> PathBuf {
    write_shim(
        bin,
        "journalctl",
        log,
        &format!(
            "case \"$1\" in\n  --disk-usage) printf '%s\\n' '{DISK_USAGE}' ;;\n  --vacuum-*) exit 0 ;;\n  *) exit 9 ;;\nesac"
        ),
    )
}

fn read_log(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Shim tests in one binary run one at a time. A sibling test's fork can hold a freshly written
/// shim open for write, and exec then fails with ETXTBSY ("Text file busy").
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _serial: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    sudo: PathBuf,
    log: PathBuf,
    journal: PathBuf,
}

/// A journal directory with one active and one archived file, plus shimmed journalctl and sudo.
fn fixture() -> Fixture {
    let serial = SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let log = root.join("calls.log");
    let bin = root.join("usr-bin");
    let sudo = sudo_shim(&root.join("sudo-bin"), &log);
    journalctl_shim(&bin, &log);
    let journal = root
        .join("journal")
        .join("fc919a235e10479284adc8a1a11a0bfd");
    fs::create_dir_all(&journal).expect("journal dir");
    fs::write(journal.join("system.journal"), b"active").expect("active file");
    fs::write(
        journal.join("system@ed80-0000000000000001-00065d07.journal"),
        b"archived",
    )
    .expect("archived file");
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    Fixture {
        _serial: serial,
        home,
        bin,
        sudo,
        log,
        journal,
        _temp: temp,
    }
}

fn xdg_for(home: &Path) -> XdgPaths {
    XdgPaths::from_values(home.to_path_buf(), None, None, None, None).expect("XDG paths")
}

fn never_running(_: &str) -> ProcessState {
    ProcessState::NotRunning
}

fn context<'a>(
    home: &'a Path,
    xdg: &'a XdgPaths,
    whitelist: &'a Whitelist,
    probe: &'a dyn Fn(&str) -> ProcessState,
    cancel: &'a AtomicBool,
) -> RuleContext<'a> {
    RuleContext {
        home,
        xdg,
        whitelist,
        proc_probe: probe,
        now: SystemTime::now(),
        cancel,
    }
}

fn discover(fx: &Fixture, dir: PathBuf, consent: Consent) -> Discovery {
    let runner = OwnerRunner::new(fx.bin.clone(), fx.sudo.clone(), consent);
    let xdg = xdg_for(&fx.home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&fx.home, &xdg, &whitelist, &probe, &cancel);
    JournalRule::with_dir(dir).discover_with_owner(&ctx, &runner)
}

fn run_clean(fx: &Fixture, args: &CleanArgs, consent: bool) -> CleanSummary {
    let mut runner = OwnerRunner::new(fx.bin.clone(), fx.sudo.clone(), Consent::Unasked);
    let xdg = xdg_for(&fx.home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&fx.home, &xdg, &whitelist, &probe, &cancel);
    let rule = JournalRule::with_dir(fx.journal.clone());
    let rules: Vec<&dyn CleanRule> = vec![&rule];
    let oplog = OpLog::with_state_home(&fx.home.join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let asked = RefCell::new(0usize);
    let prompt = |_: &[Vec<OsString>]| {
        *asked.borrow_mut() += 1;
        consent
    };
    execute_rules_with_owner(
        args,
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &prompt,
    )
    .expect("clean run")
}

#[test]
fn preview_reports_usage_and_archived_split_and_runs_only_the_usage_read() {
    let fx = fixture();

    let discovery = discover(&fx, fx.journal.clone(), Consent::Unasked);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert_eq!(discovery.candidates.len(), 1);
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.recovery, RecoveryMode::OwnerNative);
    assert_eq!(candidate.owner_check, OwnerCheck::None);
    assert!(candidate.reason.contains("286.8M"), "{}", candidate.reason);
    assert!(
        candidate.reason.contains("1 archived"),
        "{}",
        candidate.reason
    );
    assert!(
        candidate.reason.contains("1 active"),
        "{}",
        candidate.reason
    );
    assert!(candidate.reason.contains("512 MiB"), "{}", candidate.reason);
    assert_eq!(
        read_log(&fx.log),
        vec!["journalctl\t--disk-usage".to_owned()]
    );
}

#[test]
fn rule_is_a_system_rule_with_the_journal_id() {
    let rule = JournalRule::with_dir(PathBuf::from("/nonexistent"));

    assert_eq!(rule.id(), "journal");
    assert_eq!(rule.category(), Category::System);
}

#[test]
fn dry_run_selecting_journal_executes_nothing_but_the_usage_read() {
    let fx = fixture();

    let summary = run_clean(
        &fx,
        &CleanArgs {
            dry_run: true,
            select: Some("journal".to_owned()),
            ..CleanArgs::default()
        },
        true,
    );

    assert_eq!(summary.discovered, 1);
    assert_eq!(summary.applied, 0);
    assert_eq!(
        read_log(&fx.log),
        vec!["journalctl\t--disk-usage".to_owned()]
    );
}

#[test]
fn apply_after_explicit_select_and_consent_vacuums_archived_only_through_sudo() {
    let fx = fixture();

    let summary = run_clean(
        &fx,
        &CleanArgs {
            yes: true,
            select: Some("journal".to_owned()),
            ..CleanArgs::default()
        },
        true,
    );

    assert_eq!(summary.applied, 1, "{summary:?}");
    let calls = read_log(&fx.log);
    assert!(
        calls.contains(&format!(
            "sudo\t--\t{}\t--vacuum-size=512M",
            fx.bin.join("journalctl").display()
        )),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"journalctl\t--vacuum-size=512M".to_owned()),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .all(|call| !call.contains("--rotate") && !call.contains("--flush")),
        "active journals must stay untouched: {calls:?}"
    );
}

#[test]
fn journal_is_never_applied_without_an_explicit_select() {
    let fx = fixture();

    let summary = run_clean(
        &fx,
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        true,
    );

    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(
        read_log(&fx.log),
        vec!["journalctl\t--disk-usage".to_owned()]
    );
}

#[test]
fn declined_consent_runs_no_vacuum() {
    let fx = fixture();

    // `--yes` skips the batch confirmation; the owner consent prompt is what declines here.
    let summary = run_clean(
        &fx,
        &CleanArgs {
            yes: true,
            select: Some("journal".to_owned()),
            ..CleanArgs::default()
        },
        false,
    );

    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(
        read_log(&fx.log)
            .iter()
            .all(|call| !call.contains("--vacuum")),
        "declined consent must not vacuum"
    );
}

#[test]
fn no_archived_files_means_nothing_to_offer() {
    let fx = fixture();
    fs::remove_file(
        fx.journal
            .join("system@ed80-0000000000000001-00065d07.journal"),
    )
    .expect("remove archived file");

    let discovery = discover(&fx, fx.journal.clone(), Consent::Unasked);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert!(discovery.candidates.is_empty());
}

#[test]
fn missing_journalctl_is_unavailable_not_empty() {
    let fx = fixture();
    fs::remove_file(fx.bin.join("journalctl")).expect("remove shim");

    let discovery = discover(&fx, fx.journal.clone(), Consent::Unasked);

    match discovery.status {
        DiscoveryStatus::Unavailable(reason) => {
            assert!(reason.contains("journalctl"), "reason: {reason}");
        }
        other => panic!("missing journalctl must be unavailable, got {other:?}"),
    }
    assert!(discovery.candidates.is_empty());
}

#[test]
fn unreadable_journal_directory_is_unavailable() {
    let fx = fixture();

    let discovery = discover(&fx, fx.journal.join("does-not-exist"), Consent::Unasked);

    assert!(
        matches!(discovery.status, DiscoveryStatus::Unavailable(_)),
        "{:?}",
        discovery.status
    );
}
