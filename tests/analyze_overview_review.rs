use std::process::{Command, Stdio};
use tempfile::tempdir;
#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

#[test]
fn analyze_json_exits_successfully_when_stdout_is_closed() {
    let home = tempdir().expect("temporary home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["analyze", "--json"])
        .isolated_home(home.path())
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn analyze --json");
    drop(child.stdout.take());

    let status = child.wait().expect("wait for analyze --json");

    assert!(
        status.success(),
        "closed stdout should exit cleanly: {status}"
    );
}
