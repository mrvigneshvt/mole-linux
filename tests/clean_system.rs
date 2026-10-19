//! M6.2 contracts: pacman/AUR cache rules run owner tools through `OwnerRunner`,
//! preview without sudo, apply only after one consent, and never touch git checkouts.

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
use mole_linux::core::privilege::{Consent, OwnerCommand, OwnerProgram, OwnerRunner};
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode, RuleContext, catalog,
};
use mole_linux::rules::system::{PaccacheRule, YayBuildRule};

/// Shim tests in one binary run one at a time. A sibling test's fork can hold a freshly written
/// shim open for write, and exec then fails with ETXTBSY ("Text file busy").
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

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

/// A sudo stand-in that drops a leading `--` and execs the rest, as sudo would.
fn sudo_shim(dir: &Path, log: &Path) -> PathBuf {
    write_shim(
        dir,
        "sudo",
        log,
        "if [ \"$1\" = -- ]; then shift; fi\nexec \"$@\"",
    )
}

/// A paccache stand-in: `-dk3`/`-duk0` print the given preview, `-rk3`/`-ruk0` succeed.
fn paccache_shim(bin: &Path, log: &Path, preview: &str) -> PathBuf {
    write_shim(
        bin,
        "paccache",
        log,
        &format!(
            "case \"$1\" in\n  -d*) printf '%s\\n' '{preview}' ;;\n  -r*) exit 0 ;;\n  *) exit 9 ;;\nesac"
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

const TWO_CANDIDATES: &str = "==> finished dry run: 2 candidates (disk space saved: 8 MiB)";

#[test]
fn owner_query_returns_stdout_for_unprivileged_queries_without_sudo() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let query = OwnerCommand::new(OwnerProgram::Paccache, ["-dk3"], false);

    let output = runner.query(&query).expect("query succeeds");

    assert!(
        output.contains("finished dry run: 2 candidates"),
        "{output}"
    );
    assert_eq!(read_log(&log), vec!["paccache\t-dk3".to_owned()]);
}

#[test]
fn owner_query_refuses_root_commands_and_missing_tools() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let runner = OwnerRunner::new(bin.clone(), sudo, Consent::Granted);

    let root = runner
        .query(&OwnerCommand::new(OwnerProgram::Paccache, ["-dk3"], true))
        .expect_err("queries never run as root")
        .reason
        .unwrap_or_default();
    assert!(root.contains("sudo"), "{root}");

    let missing = OwnerRunner::new(
        temp.path().join("empty-bin"),
        temp.path().join("no-sudo"),
        Consent::Unasked,
    )
    .query(&OwnerCommand::new(OwnerProgram::Paccache, ["-dk3"], false))
    .expect_err("missing tool is an error")
    .reason
    .unwrap_or_default();
    assert!(missing.contains("missing"), "{missing}");
    assert!(read_log(&log).is_empty(), "refused queries execute nothing");
}

#[test]
fn keep_three_discovery_reports_one_owner_action_from_the_preview() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rule = PaccacheRule::keep_three(cache.clone());

    let discovery = rule.discover_with_owner(&ctx, &runner);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert_eq!(discovery.candidates.len(), 1);
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.path, cache);
    assert_eq!(candidate.recovery, RecoveryMode::OwnerNative);
    assert_eq!(
        candidate.owner_check,
        OwnerCheck::ProcessName("pacman".to_owned())
    );
    assert!(
        candidate.reason.contains("2 package file"),
        "{}",
        candidate.reason
    );
    assert_eq!(rule.category(), Category::System);
}

#[test]
fn keep_three_discovery_with_no_candidates_is_complete_and_empty() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    paccache_shim(&bin, &log, "==> no candidate packages found for pruning");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);

    let discovery = PaccacheRule::keep_three(cache).discover_with_owner(&ctx, &runner);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert!(discovery.candidates.is_empty());
}

#[test]
fn unrecognised_preview_output_is_unavailable_never_zero() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    paccache_shim(&bin, &log, "something unexpected");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);

    let discovery = PaccacheRule::keep_three(cache).discover_with_owner(&ctx, &runner);

    assert!(matches!(discovery.status, DiscoveryStatus::Unavailable(_)));
    assert!(discovery.candidates.is_empty());
}

#[test]
fn missing_paccache_is_unavailable_for_both_pacman_rules() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let runner = OwnerRunner::new(
        temp.path().join("empty-bin"),
        temp.path().join("no-sudo"),
        Consent::Unasked,
    );
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);

    for rule in [
        PaccacheRule::keep_three(cache.clone()),
        PaccacheRule::uninstalled(cache),
    ] {
        let discovery = rule.discover_with_owner(&ctx, &runner);
        assert!(
            matches!(discovery.status, DiscoveryStatus::Unavailable(_)),
            "{}: {:?}",
            rule.id(),
            discovery.status
        );
    }
}

#[test]
fn apply_runs_keep_three_through_sudo_after_one_consent_with_exact_argv() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let mut runner = OwnerRunner::new(bin.clone(), sudo, Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(PaccacheRule::keep_three(cache))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let prompts = std::cell::RefCell::new(Vec::<Vec<Vec<OsString>>>::new());
    let prompt = |listing: &[Vec<OsString>]| {
        prompts.borrow_mut().push(listing.to_vec());
        true
    };

    let summary: CleanSummary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("pacman-cache-keep3".to_owned()),
            ..CleanArgs::default()
        },
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &prompt,
    )
    .expect("apply");

    assert_eq!(summary.applied, 1, "{summary:?}");
    let prompts = prompts.into_inner();
    assert_eq!(prompts.len(), 1, "one consent prompt per run");
    assert_eq!(prompts[0].len(), 1, "prompt lists the exact owner argv");
    assert!(
        prompts[0][0].iter().any(|arg| arg == "-rk3"),
        "{:?}",
        prompts[0]
    );
    let calls = read_log(&log);
    assert_eq!(
        calls,
        vec![
            "paccache\t-dk3".to_owned(),
            format!("sudo\t--\t{}\t-rk3", bin.join("paccache").display()),
            "paccache\t-rk3".to_owned(),
        ]
    );
}

#[test]
fn declined_consent_runs_no_sudo_and_reports_skipped() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(PaccacheRule::keep_three(cache))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let summary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("pacman-cache-keep3".to_owned()),
            ..CleanArgs::default()
        },
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &|_| false,
    )
    .expect("declined apply");

    assert_eq!(summary.applied, 0);
    assert_eq!(summary.skipped, 1);
    assert_eq!(read_log(&log), vec!["paccache\t-dk3".to_owned()]);
}

#[test]
fn dry_run_previews_owner_action_without_prompt_or_execution() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(PaccacheRule::keep_three(cache))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", true).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let summary = execute_rules_with_owner(
        &CleanArgs {
            dry_run: true,
            select: Some("pacman-cache-keep3".to_owned()),
            ..CleanArgs::default()
        },
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &|_| panic!("dry run must not prompt"),
    )
    .expect("dry run");

    assert_eq!(summary.applied, 0);
    assert_eq!(summary.discovered, 1);
    assert_eq!(read_log(&log), vec!["paccache\t-dk3".to_owned()]);
}

#[test]
fn owner_actions_are_opt_in_and_never_prompt_without_an_explicit_select() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(PaccacheRule::keep_three(cache))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

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
        &|_| panic!("unselected owner action must not prompt"),
    )
    .expect("bare clean");

    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(read_log(&log), vec!["paccache\t-dk3".to_owned()]);
}

#[test]
fn category_select_opts_into_owner_actions() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    paccache_shim(&bin, &log, TWO_CANDIDATES);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(PaccacheRule::keep_three(cache))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let summary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("system".to_owned()),
            ..CleanArgs::default()
        },
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &|_| true,
    )
    .expect("category select");

    assert_eq!(summary.applied, 1, "{summary:?}");
}

#[test]
fn owner_runner_exposes_the_exact_argv_it_would_execute() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let sudo = temp.path().join("sudo-bin/sudo");
    let runner = OwnerRunner::new(bin.clone(), sudo.clone(), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Paccache, ["-ruk0"], true);

    assert_eq!(
        runner.argv(&cmd),
        vec![
            sudo.into_os_string(),
            "--".into(),
            bin.join("paccache").into_os_string(),
            "-ruk0".into(),
        ]
    );
}

#[test]
fn yay_build_dirs_without_git_are_candidates_and_git_checkouts_are_kept() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = home.join(".cache/yay");
    fs::create_dir_all(cache.join("clean-pkg/src/build-tree")).expect("clean build");
    fs::write(cache.join("clean-pkg/src/build-tree/object"), b"artifact").expect("artifact");
    fs::create_dir_all(cache.join("vcs-pkg/src/upstream/.git")).expect("git checkout");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);

    let discovery = YayBuildRule::new(cache.clone()).discover(&ctx);

    assert_eq!(discovery.candidates.len(), 1, "{discovery:?}");
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.path, cache.join("clean-pkg/src"));
    assert_eq!(candidate.recovery, RecoveryMode::Regenerate);
    assert!(
        matches!(&discovery.status, DiscoveryStatus::Partial(reason) if reason.contains("git")),
        "{:?}",
        discovery.status
    );
    assert!(cache.join("vcs-pkg/src/upstream/.git").exists());
}

#[test]
fn yay_build_dir_apply_moves_to_trash_through_the_sink_only() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = home.join(".cache/yay");
    let build = cache.join("clean-pkg/src");
    fs::create_dir_all(build.join("tree")).expect("build tree");
    fs::write(build.join("tree/object"), b"regenerable").expect("artifact");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let rules_owned: Vec<Box<dyn CleanRule>> = vec![Box::new(YayBuildRule::new(cache.clone()))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let mut runner = OwnerRunner::new(
        temp.path().join("bin"),
        temp.path().join("sudo"),
        Consent::Refused,
    );

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
        &|_| false,
    )
    .expect("apply yay build dir");

    assert_eq!(summary.applied, 1, "{summary:?}");
    assert!(!build.exists());
    assert!(home.join(".local/share/Trash/files/src").exists());
}

#[test]
fn system_catalog_registers_pacman_and_aur_rules_in_the_system_category() {
    let _serial = serial();
    let rules = catalog();
    let system: Vec<&str> = rules
        .iter()
        .filter(|rule| rule.category() == Category::System)
        .map(|rule| rule.id())
        .collect();

    assert_eq!(
        system,
        vec![
            "coredumps",
            "pacman-cache-keep3",
            "pacman-cache-uninstalled",
            "orphan-packages",
            "aur-build-dirs",
            "flatpak-unused-system",
            "flatpak-unused-user",
            "journal",
        ]
    );
}

#[test]
fn discovery_defaults_do_not_need_an_owner_runner_for_plain_rules() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let xdg = xdg_for(&home);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = context(&home, &xdg, &whitelist, &probe, &cancel);
    let runner = OwnerRunner::new(
        temp.path().join("bin"),
        temp.path().join("sudo"),
        Consent::Refused,
    );
    let plain = YayBuildRule::new(home.join(".cache/yay"));

    let via_owner: Discovery = plain.discover_with_owner(&ctx, &runner);

    assert_eq!(via_owner.status, DiscoveryStatus::Complete);
}
