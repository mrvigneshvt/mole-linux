//! Port of tw93/Mole tests/optimize_summary.bats (dry-run summary, failure and history
//! contracts) against shimmed owner tools on a test PATH and a temporary HOME.
//! Upstream: bin/optimize.sh, lib/optimize/outcomes.sh (GPL-3.0).

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

use mole_linux::commands::optimize::execute;
use mole_linux::core::oplog::OpLog;
use mole_linux::core::outcome::Outcome;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};

/// A shim that appends its argv to `log` and then runs `body`.
fn write_shim(dir: &Path, name: &str, log: &Path, body: &str) {
    fs::create_dir_all(dir).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{name}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = log.display(),
    );
    let path = dir.join(name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    log: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let fixture = Self {
            home: root.join("home"),
            bin: root.join("usr-bin"),
            log: root.join("calls.log"),
            state: root.join("state"),
            _temp: temp,
        };
        fs::create_dir_all(&fixture.home).expect("home");
        // Healthy defaults: every owner tool present, no failed units, timer enabled.
        let log = fixture.log.clone();
        write_shim(&fixture.bin, "fc-cache", &log, "exit 0");
        write_shim(&fixture.bin, "update-desktop-database", &log, "exit 0");
        write_shim(&fixture.bin, "update-mime-database", &log, "exit 0");
        write_shim(
            &fixture.bin,
            "systemctl",
            &log,
            "case \"$*\" in *is-enabled*) echo enabled ;; esac",
        );
        write_shim(
            &fixture.bin,
            "journalctl",
            &log,
            "echo 'Archived and active journals take up 1.0M in the file system.'",
        );
        fixture
    }

    fn replace_shim(&self, name: &str, body: &str) {
        write_shim(&self.bin, name, &self.log, body);
    }

    fn xdg(&self) -> XdgPaths {
        XdgPaths::from_values(self.home.clone(), None, None, None, None).expect("xdg")
    }

    fn write_exclusions(&self, contents: &str) {
        let dir = self.xdg().config.join("mole-linux");
        fs::create_dir_all(&dir).expect("config dir");
        fs::write(dir.join("whitelist_optimize"), contents).expect("whitelist");
    }

    fn run(&self, dry_run: bool) -> io::Result<(mole_linux::core::outcome::OutcomeLedger, String)> {
        let mut runner = OwnerRunner::new(
            self.bin.clone(),
            self.bin.join("sudo-not-used"),
            Consent::Unasked,
        );
        let log = OpLog::with_state_home(&self.state);
        let mut out = Vec::new();
        let ledger = execute(dry_run, &self.xdg(), &mut runner, &log, &mut out)?;
        Ok((ledger, String::from_utf8(out).expect("utf8 output")))
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn history(&self) -> String {
        fs::read_to_string(self.state.join("mole-linux/operations.log")).unwrap_or_default()
    }
}

#[test]
fn dry_run_reports_outcomes_and_never_runs_a_mutating_tool() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.xdg().data.join("applications")).expect("applications dir");

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Applied), 2, "output: {output}");
    assert_eq!(ledger.count(Outcome::Unchanged), 3, "output: {output}");
    assert_eq!(ledger.count(Outcome::Failed), 0);
    assert!(
        output.contains("Dry run: would apply 2 optimizations"),
        "output: {output}"
    );
    let calls = fx.calls();
    assert!(
        calls.iter().all(|call| !call.starts_with("fc-cache")
            && !call.starts_with("update-desktop-database")
            && !call.starts_with("update-mime-database")),
        "dry run executed a mutating tool: {calls:?}"
    );
}

#[test]
fn apply_refreshes_user_caches_with_exact_argv() {
    let fx = Fixture::new();
    let applications = fx.xdg().data.join("applications");
    // update-mime-database rebuilds from <dir>/packages, so that is the directory it is given.
    let mime = fx.xdg().data.join("mime/packages");
    fs::create_dir_all(&applications).expect("applications dir");
    fs::create_dir_all(&mime).expect("mime packages dir");

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(ledger.count(Outcome::Applied), 2, "output: {output}");
    assert!(
        output.contains("Applied 2 optimizations"),
        "output: {output}"
    );
    let calls = fx.calls();
    assert!(calls.contains(&"fc-cache".to_owned()), "calls: {calls:?}");
    assert!(
        calls.contains(&format!(
            "update-desktop-database\t{}",
            applications.display()
        )),
        "calls: {calls:?}"
    );
    assert!(
        calls.contains(&format!("update-mime-database\t{}", mime.display())),
        "calls: {calls:?}"
    );
}

#[test]
fn mime_directory_without_packages_is_unchanged_not_failed() {
    // Real update-mime-database exits 1 when <dir>/packages is missing, which is the normal
    // state of a fresh ~/.local/share/mime. Nothing to rebuild means unchanged, not failed.
    let fx = Fixture::new();
    fx.replace_shim(
        "update-mime-database",
        "if [ ! -d \"$1/packages\" ]; then echo \"Directory '$1/packages' does not exist!\" >&2; exit 1; fi\nexit 0",
    );
    fs::create_dir_all(fx.xdg().data.join("mime")).expect("mime dir without packages");

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(ledger.count(Outcome::Failed), 0, "output: {output}");
    assert!(
        output.contains("Desktop & MIME Database Refresh: unchanged"),
        "output: {output}"
    );
    assert!(
        fx.calls()
            .iter()
            .all(|call| !call.starts_with("update-mime-database")),
        "no packages directory means no MIME refresh"
    );
}

#[test]
fn desktop_mime_message_names_which_database_failed() {
    let fx = Fixture::new();
    fx.replace_shim("update-mime-database", "exit 1");
    fs::create_dir_all(fx.xdg().data.join("applications")).expect("applications dir");
    fs::create_dir_all(fx.xdg().data.join("mime/packages")).expect("mime packages dir");

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(ledger.count(Outcome::Failed), 1, "output: {output}");
    assert!(
        output.contains("desktop entry: database rebuilt"),
        "output: {output}"
    );
    assert!(output.contains("MIME: exited"), "output: {output}");
}

#[test]
fn missing_cache_directories_are_unchanged_not_applied() {
    let fx = Fixture::new();

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(
        ledger.count(Outcome::Applied),
        1,
        "only the font cache runs"
    );
    assert!(
        output.contains("Desktop & MIME Database Refresh: unchanged"),
        "output: {output}"
    );
    assert!(
        fx.calls()
            .iter()
            .all(|call| !call.starts_with("update-desktop-database")
                && !call.starts_with("update-mime-database")),
        "no directory means no refresh"
    );
}

#[test]
fn whitelisted_actions_are_skipped_without_running_their_tool() {
    let fx = Fixture::new();
    fx.write_exclusions("# keep fonts\nfont_cache_refresh\n");

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(ledger.count(Outcome::Skipped), 1, "output: {output}");
    assert!(
        output.contains("Font Cache Refresh: skipped"),
        "output: {output}"
    );
    assert!(
        fx.calls().iter().all(|call| !call.starts_with("fc-cache")),
        "excluded task ran its tool"
    );
}

#[test]
fn missing_owner_tool_is_unavailable_rather_than_unchanged() {
    let fx = Fixture::new();
    fs::remove_file(fx.bin.join("fc-cache")).expect("remove fc-cache");

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(ledger.count(Outcome::Unavailable), 1, "output: {output}");
    assert!(
        output.contains("Font Cache Refresh: unavailable"),
        "output: {output}"
    );
}

#[test]
fn failing_refresh_is_failed_and_recorded_in_history() {
    let fx = Fixture::new();
    fx.replace_shim("fc-cache", "exit 3");

    let (ledger, output) = fx.run(false).expect("run completes and reports failure");

    assert_eq!(ledger.count(Outcome::Failed), 1, "output: {output}");
    assert!(!ledger.succeeded());
    assert!(output.contains("1 failed"), "output: {output}");
    assert!(
        fx.history().contains("TASK_FAILED"),
        "history: {}",
        fx.history()
    );
}

#[test]
fn failed_user_units_are_attention_and_named_in_the_output() {
    let fx = Fixture::new();
    fx.replace_shim(
        "systemctl",
        "case \"$*\" in *--failed*) echo 'broken.service loaded failed failed Broken';; *is-enabled*) echo enabled;; esac",
    );

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Attention), 1, "output: {output}");
    assert!(output.contains("broken.service"), "output: {output}");
}

#[test]
fn systemd_bus_failure_is_unavailable_with_its_reason() {
    let fx = Fixture::new();
    fx.replace_shim(
        "systemctl",
        "case \"$*\" in *--failed*) echo 'Failed to connect to bus' >&2; exit 1;; *is-enabled*) echo enabled;; esac",
    );

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Unavailable), 1, "output: {output}");
    assert!(
        output.contains("Failed to connect to bus"),
        "output: {output}"
    );
}

#[test]
fn disabled_fstrim_timer_is_attention() {
    let fx = Fixture::new();
    fx.replace_shim(
        "systemctl",
        "case \"$*\" in *is-enabled*) echo disabled; exit 1;; esac",
    );

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Attention), 1, "output: {output}");
    assert!(
        output.contains("fstrim.timer is disabled"),
        "output: {output}"
    );
}

#[test]
fn unknown_fstrim_timer_state_is_unavailable_not_safe() {
    let fx = Fixture::new();
    fx.replace_shim(
        "systemctl",
        "case \"$*\" in *is-enabled*) echo not-found; exit 1;; esac",
    );

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Unavailable), 1, "output: {output}");
    assert_eq!(ledger.count(Outcome::Unchanged), 3, "output: {output}");
}

#[test]
fn large_journal_is_attention_and_nothing_is_vacuumed() {
    let fx = Fixture::new();
    fx.replace_shim(
        "journalctl",
        "echo 'Archived and active journals take up 5.0G in the file system.'",
    );

    let (ledger, output) = fx.run(false).expect("apply");

    assert_eq!(ledger.count(Outcome::Attention), 1, "output: {output}");
    assert!(
        fx.calls().iter().all(|call| !call.contains("vacuum")),
        "journal health is report-only"
    );
}

#[test]
fn journal_output_that_cannot_be_read_is_unavailable() {
    let fx = Fixture::new();
    fx.replace_shim("journalctl", "echo 'something unexpected'");

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Unavailable), 1, "output: {output}");
}

#[test]
fn optimize_cli_dry_run_exits_zero_and_logs_only_under_the_temp_home() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["optimize", "--dry-run"])
        .isolated_home(home.path())
        .output()
        .expect("run mole-linux optimize");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Dry run: would apply"), "stdout: {stdout}");
    assert!(
        home.path()
            .join(".test-xdg/state/mole-linux/operations.log")
            .is_file(),
        "dry run records its session under the temp state home"
    );
}

#[test]
fn dry_run_rows_say_would_apply_and_never_applied() {
    let fx = Fixture::new();

    let (ledger, output) = fx.run(true).expect("dry run");

    assert_eq!(ledger.count(Outcome::Applied), 1, "output: {output}");
    assert!(
        output.contains("Font Cache Refresh: would apply - would run fc-cache"),
        "output: {output}"
    );
    assert!(
        output.lines().all(|line| !line.contains(": applied")),
        "a preview row says applied: {output}"
    );
}

#[test]
fn bus_failure_is_one_line_with_a_session_hint() {
    let fx = Fixture::new();
    fx.replace_shim(
        "systemctl",
        "case \"$*\" in *--failed*) printf 'Failed to connect to user scope bus via local transport\\n' >&2; printf 'second line\\n' >&2; exit 1;; *is-enabled*) echo enabled;; esac",
    );

    let (_ledger, output) = fx.run(true).expect("dry run");

    let row = output
        .lines()
        .find(|line| line.starts_with("Failed User Units:"))
        .expect("failed units row");
    assert!(
        row.ends_with("hint: no user session bus (run from your desktop session)"),
        "row: {row}"
    );
    assert!(
        !row.contains("second line"),
        "row carries more than the first line: {row}"
    );
}
