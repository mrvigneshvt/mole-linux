//! M6.5 contracts: Flatpak unused refs. Preview is an unprivileged `flatpak list`; apply is an
//! owner-native `flatpak uninstall --unused` (system scope through sudo, user scope without).
//! `--delete-data` is never passed, and flatpak absent means Unavailable, never zero.

#[path = "honeypot/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

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
    Category, CleanRule, DiscoveryStatus, OwnerCheck, RecoveryMode, RuleContext,
};
use mole_linux::rules::flatpak::{FlatpakUnusedRule, Scope};

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

/// A flatpak stand-in: `list` prints the file `refs` holds, `uninstall` succeeds.
fn flatpak_shim(bin: &Path, log: &Path, refs: &Path) -> PathBuf {
    write_shim(
        bin,
        "flatpak",
        log,
        &format!(
            "case \"$1\" in\n  list) cat '{refs}' ;;\n  uninstall) exit 0 ;;\n  *) exit 9 ;;\nesac",
            refs = refs.display(),
        ),
    )
}

fn write_refs(dir: &Path, contents: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("refs dir");
    let path = dir.join("refs.txt");
    fs::write(&path, contents).expect("write refs");
    path
}

fn read_log(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn never_running(_: &str) -> ProcessState {
    ProcessState::NotRunning
}

const TWO_REFS: &str =
    "org.freedesktop.Platform.GL.default/x86_64/24.08\norg.gnome.Platform/x86_64/47\n";

#[test]
fn preview_counts_installed_refs_through_unprivileged_list_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let install = temp.path().join("flatpak-system");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&install).expect("installation");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), TWO_REFS);
    flatpak_shim(&bin, &log, &refs);
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    let runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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
    let rule = FlatpakUnusedRule::new(Scope::System, install.clone());

    let discovery = rule.discover_with_owner(&ctx, &runner);

    assert_eq!(discovery.status, DiscoveryStatus::Complete, "{discovery:?}");
    assert_eq!(discovery.candidates.len(), 1, "{discovery:?}");
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.path, install);
    assert_eq!(candidate.recovery, RecoveryMode::OwnerNative);
    assert_eq!(
        candidate.owner_check,
        OwnerCheck::ProcessName("flatpak".to_owned())
    );
    assert!(
        candidate.reason.contains("2 installed runtime"),
        "{}",
        candidate.reason
    );
    assert_eq!(rule.id(), "flatpak-unused-system");
    assert_eq!(rule.category(), Category::System);
    assert_eq!(
        read_log(&log),
        vec!["flatpak\tlist\t--system\t--runtime\t--columns=ref".to_owned()],
        "preview is one read-only list and never runs sudo"
    );
}

#[test]
fn empty_installation_is_complete_with_no_candidates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), "");
    flatpak_shim(&bin, &log, &refs);
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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

    let discovery = FlatpakUnusedRule::new(Scope::User, temp.path().join("user-flatpak"))
        .discover_with_owner(&ctx, &runner);

    assert_eq!(discovery.status, DiscoveryStatus::Complete, "{discovery:?}");
    assert!(discovery.candidates.is_empty());
}

#[test]
fn missing_flatpak_is_unavailable_never_zero_refs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let runner = OwnerRunner::new(
        temp.path().join("empty-bin"),
        temp.path().join("no-sudo"),
        Consent::Unasked,
    );
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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

    let discovery = FlatpakUnusedRule::new(Scope::System, temp.path().join("flatpak-system"))
        .discover_with_owner(&ctx, &runner);

    match &discovery.status {
        DiscoveryStatus::Unavailable(reason) => {
            assert!(reason.contains("missing"), "{reason}");
        }
        other => panic!("absent flatpak must be unavailable, got {other:?}"),
    }
    assert!(discovery.candidates.is_empty());
}

#[test]
fn unrecognised_list_output_is_unavailable_never_zero_refs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), "error: something unexpected\n");
    flatpak_shim(&bin, &log, &refs);
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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

    let discovery = FlatpakUnusedRule::new(Scope::User, temp.path().join("user-flatpak"))
        .discover_with_owner(&ctx, &runner);

    assert!(
        matches!(discovery.status, DiscoveryStatus::Unavailable(_)),
        "{discovery:?}"
    );
    assert!(discovery.candidates.is_empty());
}

#[test]
fn system_apply_runs_unused_removal_through_sudo_after_one_consent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let install = temp.path().join("flatpak-system");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&install).expect("installation");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), TWO_REFS);
    flatpak_shim(&bin, &log, &refs);
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    let mut runner = OwnerRunner::new(bin.clone(), sudo.clone(), Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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
    let rules_owned: Vec<Box<dyn CleanRule>> =
        vec![Box::new(FlatpakUnusedRule::new(Scope::System, install))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let prompts = std::cell::RefCell::new(Vec::<Vec<Vec<OsString>>>::new());
    let prompt = |listing: &[Vec<OsString>]| {
        prompts.borrow_mut().push(listing.to_vec());
        true
    };

    let summary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("flatpak-unused-system".to_owned()),
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
    let rendered: Vec<String> = prompts[0][0]
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        rendered,
        vec![
            sudo.display().to_string(),
            "--".to_owned(),
            bin.join("flatpak").display().to_string(),
            "uninstall".to_owned(),
            "--system".to_owned(),
            "--unused".to_owned(),
            "--assumeyes".to_owned(),
        ],
        "the prompt lists the exact owner argv"
    );
    assert!(
        !rendered.iter().any(|arg| arg.contains("delete-data")),
        "{rendered:?}"
    );
    assert_eq!(
        read_log(&log),
        vec![
            "flatpak\tlist\t--system\t--runtime\t--columns=ref".to_owned(),
            format!(
                "sudo\t--\t{}\tuninstall\t--system\t--unused\t--assumeyes",
                bin.join("flatpak").display()
            ),
            "flatpak\tuninstall\t--system\t--unused\t--assumeyes".to_owned(),
        ]
    );
}

#[test]
fn user_apply_runs_without_sudo_or_consent_prompt() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let install = home.join(".local/share/flatpak");
    fs::create_dir_all(&install).expect("user installation");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), TWO_REFS);
    flatpak_shim(&bin, &log, &refs);
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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
    let rules_owned: Vec<Box<dyn CleanRule>> =
        vec![Box::new(FlatpakUnusedRule::new(Scope::User, install))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let prompts = std::cell::Cell::new(0_u32);
    let prompt = |_: &[Vec<OsString>]| {
        prompts.set(prompts.get() + 1);
        true
    };

    let summary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("flatpak-unused-user".to_owned()),
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
    assert_eq!(prompts.get(), 0, "user scope needs no root consent");
    let calls = read_log(&log);
    assert_eq!(
        calls.last().map(String::as_str),
        Some("flatpak\tuninstall\t--user\t--unused\t--assumeyes"),
        "{calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("sudo")),
        "{calls:?}"
    );
    assert!(!calls.iter().any(|call| call.contains("delete-data")));
}

#[test]
fn declined_consent_runs_no_sudo_and_no_removal() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let install = temp.path().join("flatpak-system");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&install).expect("installation");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), TWO_REFS);
    flatpak_shim(&bin, &log, &refs);
    let sudo = sudo_shim(&temp.path().join("sudo-bin"), &log);
    let mut runner = OwnerRunner::new(bin, sudo, Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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
    let rules_owned: Vec<Box<dyn CleanRule>> =
        vec![Box::new(FlatpakUnusedRule::new(Scope::System, install))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    let summary = execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("flatpak-unused-system".to_owned()),
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

    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(summary.skipped, 1, "{summary:?}");
    assert_eq!(
        read_log(&log),
        vec!["flatpak\tlist\t--system\t--runtime\t--columns=ref".to_owned()],
        "declined consent runs neither sudo nor flatpak uninstall"
    );
}

#[test]
fn user_apply_leaves_every_honeypot_canary_intact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");
    let install = home.join(".local/share/flatpak");
    fs::create_dir_all(&install).expect("user installation");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), TWO_REFS);
    flatpak_shim(&bin, &log, &refs);
    let mut runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Granted);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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
    let rules_owned: Vec<Box<dyn CleanRule>> =
        vec![Box::new(FlatpakUnusedRule::new(Scope::User, install))];
    let rules: Vec<&dyn CleanRule> = rules_owned.iter().map(Box::as_ref).collect();
    let oplog = OpLog::with_state_home(&temp.path().join("state"));
    let mut session = oplog.start_session("clean", false).expect("session");
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());

    execute_rules_with_owner(
        &CleanArgs {
            yes: true,
            select: Some("flatpak-unused-user".to_owned()),
            ..CleanArgs::default()
        },
        &rules,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &|_| true,
    )
    .expect("apply");

    fixture
        .assert_canaries_intact()
        .expect("canaries intact after flatpak apply");
}

#[test]
fn list_header_row_is_skipped_and_header_only_output_is_zero_refs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    let refs = write_refs(temp.path(), "Ref\norg.gnome.Platform/x86_64/47\n");
    flatpak_shim(&bin, &log, &refs);
    let install = temp.path().join("user-flatpak");
    fs::create_dir_all(&install).expect("user installation");
    let runner = OwnerRunner::new(bin.clone(), temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
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
    let rule = FlatpakUnusedRule::new(Scope::User, install);

    let with_header = rule.discover_with_owner(&ctx, &runner);
    assert_eq!(
        with_header.status,
        DiscoveryStatus::Complete,
        "{with_header:?}"
    );
    assert!(
        with_header.candidates[0]
            .reason
            .contains("1 installed runtime"),
        "{:?}",
        with_header.candidates
    );

    fs::write(&refs, "Ref\n").expect("header only");
    let header_only = rule.discover_with_owner(&ctx, &runner);
    assert_eq!(
        header_only.status,
        DiscoveryStatus::Complete,
        "{header_only:?}"
    );
    assert!(header_only.candidates.is_empty(), "{header_only:?}");

    fs::write(&refs, "org.gnome.Platform/x86_64/47\nnot a ref\n").expect("bad row");
    let bad_row = rule.discover_with_owner(&ctx, &runner);
    assert!(
        matches!(bad_row.status, DiscoveryStatus::Unavailable(_)),
        "a non-ref line after the header is unknown, never skipped: {bad_row:?}"
    );
}
