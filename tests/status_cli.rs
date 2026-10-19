use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

fn run(args: &[&str]) -> std::process::Output {
    home_support::output(args)
}

#[test]
fn status_piped_stdout_emits_json_without_flag() {
    let output = run(&["status"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("piped status output is JSON");
    assert!(value.get("cpu").is_some());
    assert!(value.get("memory").is_some());
    assert!(value["cpu"]["usage_percent"].is_null());
}

#[test]
fn status_watch_emits_one_json_snapshot_per_line() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["status", "--watch", "--interval", "1s"])
        .isolated_home(home.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run status watch");
    let stdout = child.stdout.take().expect("watch stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let first_started = std::time::Instant::now();
    reader.read_line(&mut line).expect("read first NDJSON line");
    let first_read = std::time::Instant::now();
    let first: serde_json::Value = serde_json::from_str(&line).expect("first line is JSON");
    assert!(first.get("cpu").is_some());
    assert!(first["cpu"]["usage_percent"].is_null());
    line.clear();
    reader
        .read_line(&mut line)
        .expect("read second NDJSON line");
    let second_read = std::time::Instant::now();
    let second: serde_json::Value = serde_json::from_str(&line).expect("second line is JSON");
    assert!(second.get("cpu").is_some());
    assert!(second["cpu"]["usage_percent"].is_number());
    assert!(
        second_read.duration_since(first_read) >= std::time::Duration::from_secs(1),
        "watch interval elapsed only {:?} after first sample (startup took {:?})",
        second_read.duration_since(first_read),
        first_read.duration_since(first_started)
    );
    child.kill().expect("stop watch child");
    child.wait().expect("reap watch child");
}

#[test]
fn status_watch_rejects_zero_interval() {
    let output = run(&["status", "--watch", "--interval", "0s"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn status_watch_exits_cleanly_when_stdout_closes() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["status", "--watch", "--interval", "10ms"])
        .isolated_home(home.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run status watch");
    drop(child.stdout.take());
    let result = child.wait().expect("wait for status watch");
    assert!(result.success(), "closed stdout exited with {result}");
}
