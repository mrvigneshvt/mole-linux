use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use mole_linux::core::outcome::Outcome;
use mole_linux::core::privilege::{
    Consent, OwnerCommand, OwnerProgram, OwnerRunner, PrivilegeError, RunMode, ensure_not_root,
};

/// Writes an executable shell script that appends its argv to `log` and runs `body`.
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

/// A sudo shim that logs its argv, drops a leading `--`, and execs the rest so the
/// owner shim really runs, the way sudo would.
fn sudo_shim(dir: &Path, log: &Path) -> PathBuf {
    write_shim(
        dir,
        "sudo",
        log,
        "if [ \"$1\" = -- ]; then shift; fi\nexec \"$@\"",
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
/// shim open for write, and exec then fails with ETXTBSY ("Text file busy"). Other binaries are
/// separate processes and never share that descriptor, so only same-binary tests need this.
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _serial: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    bin: PathBuf,
    sudo_dir: PathBuf,
    log: PathBuf,
}

fn fixture() -> Fixture {
    let serial = SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    Fixture {
        _serial: serial,
        bin: root.join("usr-bin"),
        sudo_dir: root.join("sudo-bin"),
        log: root.join("calls.log"),
        _temp: temp,
    }
}

#[test]
fn inherited_root_is_refused_with_a_clear_message() {
    let error = ensure_not_root(0).expect_err("euid 0 must be refused");
    assert_eq!(error, PrivilegeError::InheritedRoot);
    let message = error.to_string();
    assert!(message.contains("without sudo"), "message: {message}");
    assert!(ensure_not_root(1000).is_ok());
}

#[test]
fn argv_wraps_root_commands_in_sudo_double_dash_with_resolved_program() {
    let fx = fixture();
    let sudo = fx.sudo_dir.join("sudo");
    let cmd = OwnerCommand::new(OwnerProgram::Paccache, ["-dk3"], true);

    let argv = cmd.argv(&fx.bin, &sudo);

    assert_eq!(
        argv,
        vec![
            sudo.clone().into_os_string(),
            "--".into(),
            fx.bin.join("paccache").into_os_string(),
            "-dk3".into(),
        ]
    );
}

#[test]
fn argv_runs_unprivileged_commands_without_sudo() {
    let fx = fixture();
    let sudo = fx.sudo_dir.join("sudo");
    let cmd = OwnerCommand::new(OwnerProgram::Journalctl, ["--disk-usage"], false);

    let argv = cmd.argv(&fx.bin, &sudo);

    assert_eq!(
        argv,
        vec![
            fx.bin.join("journalctl").into_os_string(),
            "--disk-usage".into(),
        ]
    );
}

#[test]
fn dry_run_prints_exact_argv_and_never_invokes_sudo() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "pacman", &fx.log, "exit 0");
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo.clone(), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);
    let asked = Cell::new(false);

    let run = runner.run(&cmd, RunMode::DryRun, || {
        asked.set(true);
        true
    });

    assert_eq!(run.outcome, Outcome::Skipped);
    assert_eq!(run.argv, cmd.argv(&fx.bin, &sudo));
    assert!(!asked.get(), "dry run must not ask for consent");
    assert!(
        read_log(&fx.log).is_empty(),
        "dry run must not execute anything"
    );
}

#[test]
fn apply_runs_owner_tool_through_sudo_after_consent() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "pacman", &fx.log, "exit 0");
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);
    let asked = Cell::new(0);

    let run = runner.run(&cmd, RunMode::Apply, || {
        asked.set(asked.get() + 1);
        true
    });

    assert_eq!(run.outcome, Outcome::Applied);
    assert_eq!(asked.get(), 1);
    assert_eq!(
        read_log(&fx.log),
        vec![
            format!("sudo\t--\t{}\t-Qq", fx.bin.join("pacman").display()),
            "pacman\t-Qq".to_owned(),
        ]
    );
}

#[test]
fn consent_is_asked_once_per_run_and_declined_consent_never_runs_sudo() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "paccache", &fx.log, "exit 0");
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Paccache, ["-rk3"], true);
    let asked = Cell::new(0);
    let ask = || {
        asked.set(asked.get() + 1);
        false
    };

    let first = runner.run(&cmd, RunMode::Apply, ask);
    let second = runner.run(&cmd, RunMode::Apply, || {
        asked.set(asked.get() + 1);
        true
    });

    assert_eq!(first.outcome, Outcome::Skipped);
    assert_eq!(
        second.outcome,
        Outcome::Skipped,
        "refusal sticks for the run"
    );
    assert_eq!(asked.get(), 1, "no prompt loop after refusal");
    assert!(read_log(&fx.log).is_empty(), "refused consent runs nothing");
}

#[test]
fn yes_flag_grants_consent_without_prompting() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "coredumpctl", &fx.log, "exit 0");
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Granted);
    let cmd = OwnerCommand::new(OwnerProgram::Coredumpctl, ["list"], true);

    let run = runner.run(&cmd, RunMode::Apply, || panic!("--yes must not prompt"));

    assert_eq!(run.outcome, Outcome::Applied);
}

#[test]
fn unprivileged_command_never_touches_sudo_even_when_applied() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "flatpak", &fx.log, "exit 0");
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Flatpak, ["list"], false);

    let run = runner.run(&cmd, RunMode::Apply, || panic!("no root needed, no prompt"));

    assert_eq!(run.outcome, Outcome::Applied);
    assert_eq!(read_log(&fx.log), vec!["flatpak\tlist".to_owned()]);
}

#[test]
fn missing_sudo_is_unavailable_and_never_prompts() {
    let fx = fixture();
    write_shim(&fx.bin, "pacman", &fx.log, "exit 0");
    let mut runner = OwnerRunner::new(fx.bin.clone(), fx.sudo_dir.join("sudo"), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);

    let run = runner.run(&cmd, RunMode::Apply, || panic!("no sudo means no prompt"));

    assert_eq!(run.outcome, Outcome::Unavailable);
    assert!(run.reason.as_deref().unwrap_or_default().contains("sudo"));
}

#[test]
fn missing_owner_tool_is_unavailable_not_applied() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Granted);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);

    let run = runner.run(&cmd, RunMode::Apply, || true);

    assert_eq!(run.outcome, Outcome::Unavailable);
    assert!(
        read_log(&fx.log).is_empty(),
        "absent tool means sudo is never invoked"
    );
}

#[test]
fn owner_tool_failure_is_reported_as_failed() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "paccache", &fx.log, "exit 3");
    let mut runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Granted);
    let cmd = OwnerCommand::new(OwnerProgram::Paccache, ["-rk3"], true);

    let run = runner.run(&cmd, RunMode::Apply, || true);

    assert_eq!(run.outcome, Outcome::Failed);
}
