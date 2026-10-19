//! Owner-tool allowlist extensions and the read-only query path used by optimize reports.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use mole_linux::core::outcome::Outcome;
use mole_linux::core::privilege::{Consent, OwnerCommand, OwnerProgram, OwnerRunner};

fn write_shim(dir: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("shim dir");
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    path
}

fn runner(bin: &Path) -> OwnerRunner {
    OwnerRunner::new(
        bin.to_path_buf(),
        bin.join("sudo-not-used"),
        Consent::Unasked,
    )
}

#[test]
fn optimize_tools_resolve_inside_the_allowlisted_bin_dir() {
    let bin = Path::new("/usr/bin");
    let sudo = Path::new("/usr/bin/sudo");

    for (program, file) in [
        (OwnerProgram::FcCache, "fc-cache"),
        (
            OwnerProgram::UpdateDesktopDatabase,
            "update-desktop-database",
        ),
        (OwnerProgram::UpdateMimeDatabase, "update-mime-database"),
        (OwnerProgram::Systemctl, "systemctl"),
    ] {
        let cmd = OwnerCommand::new(program, Vec::<&str>::new(), false);
        assert_eq!(
            cmd.argv(bin, sudo),
            vec![bin.join(file).into_os_string()],
            "{program:?}"
        );
    }
}

#[test]
fn query_returns_stdout_when_the_declared_answer_exit_code_is_used() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    write_shim(&bin, "systemctl", "echo disabled\nexit 1");
    let cmd = OwnerCommand::new(
        OwnerProgram::Systemctl,
        ["is-enabled", "fstrim.timer"],
        false,
    )
    .answer_on_exit(1);

    assert_eq!(
        runner(&bin).query(&cmd).expect("answer on stdout"),
        "disabled\n"
    );
}

#[test]
fn query_without_a_declared_answer_code_is_an_error_with_the_first_stderr_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    write_shim(
        &bin,
        "systemctl",
        "echo disabled\necho 'bus gone' >&2\nexit 1",
    );
    let cmd = OwnerCommand::new(OwnerProgram::Systemctl, ["--user", "--failed"], false);

    let error = runner(&bin)
        .query(&cmd)
        .expect_err("nonzero exit is not an answer");

    assert_eq!(error.outcome, Outcome::Failed);
    assert!(
        error
            .reason
            .as_deref()
            .unwrap_or_default()
            .ends_with("bus gone")
    );
}

#[test]
fn query_reports_a_missing_tool_as_unavailable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = runner(&temp.path().join("empty-bin"));
    let cmd = OwnerCommand::new(OwnerProgram::FcCache, Vec::<&str>::new(), false);

    let error = runner.query(&cmd).expect_err("tool is absent");

    assert_eq!(error.outcome, Outcome::Unavailable);
    assert!(!runner.is_available(OwnerProgram::FcCache));
}

#[test]
fn query_refuses_root_commands() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("usr-bin");
    write_shim(&bin, "pacman", "echo should-not-run\nexit 0");
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);

    let error = runner(&bin).query(&cmd).expect_err("root query refused");

    assert_eq!(error.outcome, Outcome::Skipped);
}
