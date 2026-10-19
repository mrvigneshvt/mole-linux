use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mole_linux::core::outcome::Outcome;
use mole_linux::core::privilege::{Consent, OwnerCommand, OwnerProgram, OwnerRunner};

/// Shim files are written and then exec'd; a sibling test thread forking in that window
/// makes exec fail with ETXTBSY. Tests in this file therefore run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_shim(dir: &Path, name: &str, log: &Path, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' '{name}' >> '{log}'\n{body}\n",
        log = log.display(),
    );
    let path = dir.join(name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    path
}

fn read_log(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn query_returns_stdout_of_an_unprivileged_owner_tool() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    write_shim(&bin, "pacman", &log, "printf 'bash 5.2-1\\n'");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qe"], false);

    let stdout = runner.query(&cmd).expect("query succeeds");

    assert_eq!(stdout, "bash 5.2-1\n");
}

#[test]
fn query_refuses_root_commands_and_never_runs_sudo() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let sudo_dir = temp.path().join("sudo-bin");
    let log = temp.path().join("calls.log");
    let sudo = write_shim(&sudo_dir, "sudo", &log, "exit 0");
    write_shim(&bin, "pacman", &log, "exit 0");
    let runner = OwnerRunner::new(bin, sudo, Consent::Granted);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);

    let refused = runner.query(&cmd).expect_err("root query must be refused");

    assert_eq!(refused.outcome, Outcome::Skipped);
    assert!(
        refused
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("sudo"),
        "reason: {:?}",
        refused.reason
    );
    assert!(
        read_log(&log).is_empty(),
        "refused query runs nothing, sudo included"
    );
}

#[test]
fn query_of_a_missing_owner_tool_is_unavailable() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = OwnerRunner::new(
        temp.path().join("usr-bin"),
        temp.path().join("no-sudo"),
        Consent::Unasked,
    );
    let cmd = OwnerCommand::new(OwnerProgram::Flatpak, ["list"], false);

    let missing = runner.query(&cmd).expect_err("missing tool");

    assert_eq!(missing.outcome, Outcome::Unavailable);
    assert!(
        missing
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("missing or not executable"),
        "reason: {:?}",
        missing.reason
    );
}

#[test]
fn query_nonzero_exit_is_failed_not_empty() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    write_shim(&bin, "pacman", &log, "printf 'partial\\n'; exit 1");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qe"], false);

    let failed = runner
        .query(&cmd)
        .expect_err("exit 1 must not look like an empty list");

    assert_eq!(failed.outcome, Outcome::Failed);
    assert!(
        failed
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("exited")
    );
}

#[test]
fn query_output_over_the_limit_is_failed() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    write_shim(
        &bin,
        "pacman",
        &log,
        "head -c 2097152 /dev/zero | tr '\\0' a",
    );
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qe"], false);

    let oversized = runner.query(&cmd).expect_err("output over 1 MiB must fail");

    assert_eq!(oversized.outcome, Outcome::Failed);
    assert!(
        oversized
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("limit"),
        "reason: {:?}",
        oversized.reason
    );
}

#[test]
fn query_that_hangs_is_stopped_and_reported_failed() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    write_shim(&bin, "flatpak", &log, "sleep 30");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked)
        .with_query_timeout(Duration::from_millis(300));
    let cmd = OwnerCommand::new(OwnerProgram::Flatpak, ["list"], false);

    let started = Instant::now();
    let hung = runner.query(&cmd).expect_err("hung query must fail");

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "timeout must stop the tool"
    );
    assert_eq!(hung.outcome, Outcome::Failed);
    assert!(
        hung.reason
            .as_deref()
            .unwrap_or_default()
            .contains("timed out")
    );
}

#[test]
fn silent_exit_on_a_declared_no_match_code_is_an_empty_answer() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    // pacman -Qm exits 1 with no output when no package matches.
    write_shim(&bin, "pacman", &log, "exit 1");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qm"], false).no_match_on_exit(1);

    let stdout = runner.query(&cmd).expect("no matches is not a failure");

    assert_eq!(stdout, "");
}

#[test]
fn no_match_code_with_stderr_is_a_failure_with_the_tool_message() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    let log = temp.path().join("calls.log");
    write_shim(
        &bin,
        "pacman",
        &log,
        "echo 'error: failed to open database' >&2; exit 1",
    );
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qm"], false).no_match_on_exit(1);

    let failed = runner
        .query(&cmd)
        .expect_err("a locked database is not an empty list");

    assert_eq!(failed.outcome, Outcome::Failed);
    assert!(
        failed
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("failed to open database"),
        "reason: {:?}",
        failed.reason
    );
}
