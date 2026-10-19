//! M6.4 contracts: orphan packages are a report, never an action. `pacman -Qdtq` and
//! `pacman -Qi` run unprivileged through `OwnerRunner`; no removal and no sudo ever run.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::commands::clean::{CleanArgs, execute_rules_with_owner};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Category, CleanRule, DiscoveryStatus, Report, RuleContext, catalog,
};
use mole_linux::rules::orphans::OrphanPackagesRule;

/// Writes an executable shell script that logs `name<TAB>argv...` to `log`, then runs `body`.
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

/// A pacman stand-in. `query` runs for `-Qdtq`, `info` runs for `-Qi`; anything else fails loudly.
fn pacman_shim(bin: &Path, log: &Path, query: &str, info: &str) -> PathBuf {
    write_shim(
        bin,
        "pacman",
        log,
        &format!(
            "case \"$1\" in\n  -Qdtq)\n{query}\n  ;;\n  -Qi)\n{info}\n  ;;\n  *) exit 9 ;;\nesac"
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

fn xdg_for(home: &Path) -> XdgPaths {
    XdgPaths::from_values(home.to_path_buf(), None, None, None, None).expect("XDG paths")
}

fn never_running(_: &str) -> ProcessState {
    ProcessState::NotRunning
}

const TWO_ORPHANS_QUERY: &str = "    printf 'gtk-doc\\nlibfoo\\n'\n    exit 0";
const TWO_ORPHANS_INFO: &str = "    printf 'Name            : gtk-doc\\nInstalled Size  : 1.50 MiB\\n\\nName            : libfoo\\nInstalled Size  : 512.00 KiB\\n'\n    exit 0";

fn report_for(bin: PathBuf) -> Report {
    let runner = OwnerRunner::new(bin, PathBuf::from("/nonexistent/sudo"), Consent::Unasked);
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    OrphanPackagesRule
        .report_with_owner(&ctx, &runner)
        .expect("orphan rule reports")
}

#[test]
fn orphans_are_reported_with_sizes_and_only_read_queries_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    pacman_shim(&bin, &log, TWO_ORPHANS_QUERY, TWO_ORPHANS_INFO);
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let report = OrphanPackagesRule
        .report_with_owner(&ctx, &runner)
        .expect("orphan rule reports");

    let Report::Findings(lines) = report else {
        panic!("expected findings, got {report:?}");
    };
    let text = lines.join("\n");
    assert!(text.contains("2 orphan package(s)"), "{text}");
    assert!(text.contains("gtk-doc"), "{text}");
    assert!(text.contains("libfoo"), "{text}");
    // 1.50 MiB = 1572864 B -> 1.5 MB; 512.00 KiB = 524288 B -> 524.2 kB; total of known sizes 2.0 MB.
    assert!(text.contains("1.5 MB"), "{text}");
    assert!(text.contains("524.2 kB"), "{text}");
    assert!(text.contains("2.0 MB"), "{text}");
    assert!(text.contains("nothing removed"), "{text}");
    assert_eq!(
        read_log(&log),
        vec![
            "pacman\t-Qdtq".to_owned(),
            "pacman\t-Qi\tgtk-doc\tlibfoo".to_owned(),
        ],
        "only read-only queries, no sudo, no removal"
    );
}

#[test]
fn no_orphans_is_a_clear_report_when_pacman_exits_one_silently() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    pacman_shim(&bin, &log, "    exit 1", "    exit 9");

    let report = report_for(bin);

    assert_eq!(report, Report::Clear);
    assert_eq!(read_log(&log), vec!["pacman\t-Qdtq".to_owned()]);
}

#[test]
fn pacman_stderr_output_on_a_no_match_exit_stays_unknown_not_clear() {
    // The shared runner treats a no-match exit as empty only when stderr is also empty.
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    pacman_shim(
        &bin,
        &log,
        "    printf \"warning: database file for 'core' does not exist\\n\" >&2\n    exit 1",
        "    exit 9",
    );

    let report = report_for(bin);

    assert!(matches!(report, Report::Unavailable(_)), "{report:?}");
}

#[test]
fn pacman_error_while_listing_is_unavailable_never_clear() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    pacman_shim(
        &bin,
        &log,
        "    printf 'error: failed to lock database\\n' >&2\n    exit 1",
        "    exit 9",
    );

    let report = report_for(bin);

    assert!(matches!(report, Report::Unavailable(_)), "{report:?}");
}

#[test]
fn unexpected_package_names_are_refused_before_sizes_are_queried() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    pacman_shim(
        &bin,
        &log,
        "    printf '%s\\n' '-Rns'\n    exit 0",
        "    exit 9",
    );

    let report = report_for(bin);

    assert!(matches!(report, Report::Unavailable(_)), "{report:?}");
    assert_eq!(
        read_log(&log),
        vec!["pacman\t-Qdtq".to_owned()],
        "a name that could be read as a pacman option must never reach argv"
    );
}

#[test]
fn package_without_an_installed_size_is_unknown_not_zero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    pacman_shim(
        &bin,
        &log,
        "    printf 'gtk-doc\\n'\n    exit 0",
        "    printf 'Name            : gtk-doc\\nVersion         : 1-1\\n'\n    exit 0",
    );

    let report = report_for(bin);

    let Report::Findings(lines) = report else {
        panic!("expected findings, got {report:?}");
    };
    let text = lines.join("\n");
    assert!(text.contains("gtk-doc"), "{text}");
    assert!(text.contains("size unknown"), "{text}");
    assert!(
        !text.contains(" 0 B"),
        "unknown must not read as zero: {text}"
    );
}

#[test]
fn missing_pacman_is_unavailable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = OwnerRunner::new(
        temp.path().join("empty-bin"),
        temp.path().join("no-sudo"),
        Consent::Unasked,
    );
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let report = OrphanPackagesRule
        .report_with_owner(&ctx, &runner)
        .expect("orphan rule reports");

    assert!(matches!(report, Report::Unavailable(_)), "{report:?}");
}

#[test]
fn orphan_rule_never_offers_candidates_and_has_no_owner_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let discovery = OrphanPackagesRule.discover(&ctx);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert!(discovery.candidates.is_empty());
    assert_eq!(OrphanPackagesRule.category(), Category::System);
    let candidate = dummy_candidate(&home);
    assert!(OrphanPackagesRule.owner_command(&candidate).is_none());
}

fn dummy_candidate(home: &Path) -> mole_linux::rules::clean::Candidate {
    let path = home.join("not-a-candidate");
    fs::write(&path, b"x").expect("file");
    mole_linux::rules::clean::candidate(
        &path,
        "unused",
        mole_linux::rules::clean::RecoveryMode::Irreversible,
        ProcessState::NotRunning,
    )
    .expect("candidate")
}

#[test]
fn clean_pipeline_reports_orphans_as_attention_and_never_prompts_or_removes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = write_shim(
        &temp.path().join("sudo-bin"),
        "sudo",
        &log,
        "if [ \"$1\" = -- ]; then shift; fi\nexec \"$@\"",
    );
    pacman_shim(&bin, &log, TWO_ORPHANS_QUERY, TWO_ORPHANS_INFO);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(OrphanPackagesRule)];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let prompt = |_: &[Vec<OsString>]| -> bool { panic!("a report must never ask for consent") };

    let summary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &prompt,
    )
    .expect("orphan report run");

    assert_eq!(summary.attention, 1, "{summary:?}");
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(summary.failed, 0, "{summary:?}");
    let calls = read_log(&log);
    assert!(
        calls.iter().all(|call| !call.starts_with("sudo")),
        "no sudo: {calls:?}"
    );
    assert!(
        calls.iter().all(|call| !call.contains("\t-R")),
        "no removal: {calls:?}"
    );
}

#[test]
fn orphan_report_is_registered_in_the_system_category() {
    let rules = catalog();
    let system: Vec<&str> = rules
        .iter()
        .filter(|rule| rule.category() == Category::System)
        .map(|rule| rule.id())
        .collect();

    assert!(system.contains(&"orphan-packages"), "{system:?}");
}
