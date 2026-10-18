use std::fs;
use std::process::Command;

use tempfile::tempdir;

fn run_history(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["history"])
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_STATE_HOME")
        .output()
        .expect("run history")
}

#[test]
fn history_reads_xdg_state_logs_and_emits_recent_sessions_as_json() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let app_state = state.join("mole-linux");
    fs::create_dir_all(&app_state).expect("state directory");
    fs::write(
        app_state.join("operations.log"),
        "# ========== clean session started at 2026-10-19 10:00:00 ==========\n[2026-10-19 10:00:01] [clean] TRASHED /tmp/a (2KB)\n# ========== clean session ended at 2026-10-19 10:00:02, 1 items, 2KB ==========\n",
    )
    .expect("operations log");
    fs::write(
        app_state.join("deletions.log"),
        "2026-10-19T10:00:01+0000\ttrash\t2\tok\t/tmp/a\n",
    )
    .expect("deletions log");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["history", "--json", "--limit", "1"])
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", &state)
        .output()
        .expect("run history");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["sessions"][0]["command"], "clean");
    assert_eq!(value["sessions"][0]["actions"]["trashed"], 1);
    assert_eq!(value["deletions"][0]["path"], "/tmp/a");
    assert_eq!(value["limit"], 1);
}

#[test]
fn history_rejects_limit_outside_one_to_two_hundred() {
    let home = tempdir().expect("temporary home");
    let output = run_history(home.path(), &["--limit", "201"]);
    assert!(!output.status.success());
}

#[test]
fn history_keeps_overlapping_runs_of_the_same_command_separate() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    fs::write(
        directory.join("operations.log"),
        "# ========== clean run=first session started at 2026-10-19 10:00:00 ==========\n# ========== clean run=second session started at 2026-10-19 10:00:01 ==========\n[2026-10-19 10:00:02] [clean run=first] REMOVED /tmp/one (1KB)\n[2026-10-19 10:00:03] [clean run=second] TRASHED /tmp/two (1KB)\n# ========== clean run=first session ended at 2026-10-19 10:00:04, 1 items, 1KB ==========\n# ========== clean run=second session ended at 2026-10-19 10:00:05, 1 items, 1KB ==========\n",
    )
    .expect("operations log");
    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["history", "--json"])
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", &state)
        .output()
        .expect("run history");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["sessions"][0]["command"], "clean");
    assert_eq!(value["sessions"][0]["run_id"], "second");
    assert_eq!(value["sessions"][0]["actions"]["trashed"], 1);
    assert_eq!(value["sessions"][1]["run_id"], "first");
    assert_eq!(value["sessions"][1]["actions"]["removed"], 1);
}
