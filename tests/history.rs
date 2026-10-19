use std::fs;
use std::process::Command;

use tempfile::tempdir;
#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

fn run_history(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["history"])
        .args(args)
        .isolated_home(home)
        .output()
        .expect("run history")
}

fn run_history_at(
    home: &std::path::Path,
    state: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["history"])
        .args(args)
        .isolated_home(home)
        .env("XDG_STATE_HOME", state)
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
        .isolated_home(home.path())
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
    assert_eq!(value["sessions"][0]["run_id"], "");
    assert_eq!(value["sessions"][0]["attribution"], "command");
    assert_eq!(value["sessions"][0]["started_at"], "2026-10-19T10:00:00Z");
    assert_eq!(value["sessions"][0]["ended_at"], "2026-10-19T10:00:02Z");
    assert_eq!(value["sessions"][0]["items"], 1);
    assert_eq!(value["sessions"][0]["size"], "2KB");
    assert_eq!(value["sessions"][0]["operation_count"], 1);
    assert_eq!(value["sessions"][0]["actions"]["trashed"], 1);
    assert_eq!(value["deletions"][0]["path"], "/tmp/a");
    assert_eq!(value["deletions"][0]["mode"], "trash");
    assert_eq!(value["deletions"][0]["status"], "ok");
    assert_eq!(value["deletions"][0]["size_kb"], 2);
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
        .isolated_home(home.path())
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

#[test]
fn history_text_escapes_controls_while_json_preserves_paths_and_summaries() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    fs::write(
        directory.join("operations.log"),
        concat!(
            "# ========== clean session started at 2026-10-19 10:00:00 ==========\n",
            "[2026-10-19 10:00:01] [clean] TASK_FAILED check (task outcome)\n",
            "# ========== clean session ended at malformed summary ==========\n",
            "# ========== clean-dry-run session started at 2026-10-19 10:00:03 ==========\n",
            "[2026-10-19 10:00:04] [clean-dry-run] SKIPPED /tmp/preview (dry-run)\n",
        ),
    )
    .expect("operations log");
    let unusual_path = "/tmp/back\\slash\tquote\"bell\u{7}end";
    fs::write(
        directory.join("deletions.log"),
        format!(
            "2026-10-19T10:00:01+0000\tpermanent\t10\tdry-run\t{}\n",
            encode_test_field(unusual_path)
        ),
    )
    .expect("deletions log");

    let text = run_history_at(home.path(), &state, &[]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).expect("text output");
    assert!(text.contains("clean-dry-run"));
    assert!(text.contains("back\\slash"));
    assert!(text.contains("\\x07"));
    assert!(!text.as_bytes().contains(&7));
    assert!(text.contains("1 optimize tasks failed") || text.contains("1 tasks failed"));
    assert!(text.contains("malformed"));

    let json = run_history_at(home.path(), &state, &["--json"]);
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON output");
    assert_eq!(value["sessions"][0]["command"], "clean-dry-run");
    assert_eq!(value["sessions"][1]["failed_tasks"], 1);
    assert_eq!(value["sessions"][1]["malformed_summaries"], 1);
    assert_eq!(value["deletions"][0]["path"], unusual_path);
    assert_eq!(value["deletions"][0]["status"], "dry-run");
}

#[test]
fn history_text_keeps_explicit_utc_timestamps() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    fs::write(
        directory.join("operations.log"),
        concat!(
            "# ========== clean session started at 2026-10-19T00:07:00Z ==========\n",
            "# ========== clean session ended at 2026-10-19T00:07:01Z, 0 items, 0B ==========\n"
        ),
    )
    .expect("operations log");
    let output = run_history_at(home.path(), &state, &[]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("history output");
    assert!(text.contains("2026-10-19T00:07:00Z"), "{text}");
    assert!(text.contains("2026-10-19T00:07:01Z"), "{text}");
}

fn encode_test_field(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if character.is_control() {
            for byte in character.to_string().bytes() {
                escaped.push_str(&format!("\\x{byte:02x}"));
            }
        } else {
            escaped.push(character);
        }
    }
    escaped
}

#[test]
fn history_limit_caps_both_lists_accepts_leading_zeroes_and_rejects_bad_values() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    let mut operations = String::new();
    let mut deletions = String::new();
    for index in 0..3 {
        operations.push_str(&format!(
            "# ========== cmd{index} session started at 2026-10-19 10:00:0{index} ==========\n"
        ));
        deletions.push_str(&format!(
            "2026-10-19T10:00:0{index}+0000\ttrash\t1\tok\t/tmp/{index}\n"
        ));
    }
    fs::write(directory.join("operations.log"), operations).expect("operations log");
    fs::write(directory.join("deletions.log"), deletions).expect("deletions log");
    let output = run_history_at(home.path(), &state, &["--json", "--limit", "002"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["limit"], 2);
    assert_eq!(value["sessions"].as_array().expect("sessions").len(), 2);
    assert_eq!(value["deletions"].as_array().expect("deletions").len(), 2);
    let output = run_history_at(home.path(), &state, &["--json", "--limit", "007"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["limit"], 7);

    for value in ["0", "201", "-1", "abc", ""] {
        assert!(
            !run_history_at(home.path(), &state, &["--limit", value])
                .status
                .success(),
            "accepted invalid limit {value:?}"
        );
    }
    assert!(
        !run_history_at(home.path(), &state, &["--bad-option"])
            .status
            .success()
    );
}

#[test]
fn empty_history_does_not_create_state_directories_or_logs() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("missing-state");
    let output = run_history_at(home.path(), &state, &[]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("No operation history yet"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("No deletion audit entries yet"));
    assert!(!state.exists());
}

#[test]
fn same_second_sessions_follow_marker_sequence_and_unended_runs_are_retained() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    fs::write(
        directory.join("operations.log"),
        concat!(
            "# ========== clean session started at 2026-10-19 10:00:00 ==========\n",
            "[2026-10-19 10:00:01] [clean] REMOVED /tmp/a (1KB)\n",
            "# ========== purge session started at 2026-10-19 10:00:00 ==========\n",
            "[2026-10-19 10:00:02] [purge] TRASHED /tmp/b (1KB)\n",
            "# ========== clean session started at 2026-10-19 10:00:00 ==========\n",
            "[2026-10-19 10:00:03] [clean] SKIPPED /tmp/c (policy)\n",
        ),
    )
    .expect("operations log");
    let output = run_history_at(home.path(), &state, &["--json"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    let sessions = value["sessions"].as_array().expect("sessions");
    assert_eq!(sessions[0]["command"], "clean");
    assert_eq!(sessions[1]["command"], "purge");
    assert_eq!(sessions[2]["command"], "clean");
    assert_eq!(sessions[2]["attribution"], "ambiguous");
    assert_eq!(sessions[2]["run_id"], "");
    assert!(
        sessions
            .iter()
            .filter(|session| session["command"] == "clean")
            .all(|session| session["attribution"] == "ambiguous")
    );
    assert_eq!(
        sessions
            .iter()
            .map(|session| session["actions"]["removed"].as_u64().unwrap_or(0))
            .sum::<u64>(),
        1
    );
    assert_eq!(
        sessions
            .iter()
            .map(|session| session["actions"]["skipped"].as_u64().unwrap_or(0))
            .sum::<u64>(),
        1
    );
    assert!(
        sessions
            .iter()
            .all(|session| !session["ended_at"].is_null())
    );
}

#[test]
fn an_unended_identified_run_closes_at_its_next_start_marker() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    fs::write(
        directory.join("operations.log"),
        concat!(
            "# ========== clean run=same session started at 2026-10-19 10:00:00 ==========\n",
            "[2026-10-19 10:00:01] [clean run=same] REMOVED /tmp/first (1KB)\n",
            "# ========== clean run=same session started at 2026-10-19 10:00:02 ==========\n",
            "[2026-10-19 10:00:03] [clean run=same] TRASHED /tmp/second (1KB)\n",
            "# ========== clean run=same session ended at 2026-10-19 10:00:04, 1 items, 1KB ==========\n",
        ),
    )
    .expect("operations log");
    let output = run_history_at(home.path(), &state, &["--json"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    let sessions = value["sessions"].as_array().expect("sessions");
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0]["ended_at"], "2026-10-19T10:00:04Z");
    assert_eq!(sessions[0]["actions"]["trashed"], 1);
    assert_eq!(sessions[1]["ended_at"], "2026-10-19T10:00:02Z");
    assert_eq!(sessions[1]["actions"]["removed"], 1);
}

#[test]
fn interleaved_commands_keep_actions_with_their_command_session() {
    let home = tempdir().expect("temporary home");
    let state = home.path().join("state");
    let directory = state.join("mole-linux");
    fs::create_dir_all(&directory).expect("state directory");
    fs::write(
        directory.join("operations.log"),
        concat!(
            "# ========== clean session started at 2026-10-19 10:00:00 ==========\n",
            "[2026-10-19 10:00:01] [clean] REMOVED /tmp/a (1KB)\n",
            "# ========== purge session started at 2026-10-19 10:00:02 ==========\n",
            "[2026-10-19 10:00:03] [clean] SKIPPED /tmp/b (policy)\n",
            "[2026-10-19 10:00:04] [purge] TRASHED /tmp/c (1KB)\n",
            "# ========== purge session ended at 2026-10-19 10:00:05, 1 items, 1KB ==========\n",
            "# ========== clean session ended at 2026-10-19 10:00:06, 2 items, 1KB ==========\n",
        ),
    )
    .expect("operations log");
    let output = run_history_at(home.path(), &state, &["--json"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    let sessions = value["sessions"].as_array().expect("sessions");
    assert_eq!(sessions[0]["command"], "purge");
    assert_eq!(sessions[0]["actions"]["trashed"], 1);
    assert_eq!(sessions[0]["actions"]["skipped"], 0);
    assert_eq!(sessions[1]["command"], "clean");
    assert_eq!(sessions[1]["actions"]["removed"], 1);
    assert_eq!(sessions[1]["actions"]["skipped"], 1);
}
