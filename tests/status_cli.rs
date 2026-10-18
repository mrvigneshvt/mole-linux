use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(args)
        .output()
        .expect("run mole-linux")
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
}

#[test]
fn status_watch_emits_one_json_snapshot_per_line() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["status", "--watch", "--interval", "1s"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("run status watch");
    let stdout = child.stdout.take().expect("watch stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read first NDJSON line");
    let first: serde_json::Value = serde_json::from_str(&line).expect("first line is JSON");
    assert!(first.get("cpu").is_some());
    line.clear();
    reader
        .read_line(&mut line)
        .expect("read second NDJSON line");
    let second: serde_json::Value = serde_json::from_str(&line).expect("second line is JSON");
    assert!(second.get("cpu").is_some());
    child.kill().expect("stop watch child");
    child.wait().expect("reap watch child");
}

#[test]
fn status_watch_rejects_zero_interval() {
    let output = run(&["status", "--watch", "--interval", "0s"]);
    assert_eq!(output.status.code(), Some(2));
}
