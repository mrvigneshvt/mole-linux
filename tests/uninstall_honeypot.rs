use std::process::Command;

#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

#[allow(dead_code)]
#[path = "honeypot/fixtures.rs"]
mod fixtures;

#[test]
fn uninstall_list_keeps_every_honeypot_canary() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");
    let scenario = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/honeypot/scenarios/uninstall-list.sh");
    let output = Command::new("bash")
        .arg(scenario)
        .arg(env!("CARGO_BIN_EXE_mole-linux"))
        .isolated_home(&home)
        .output()
        .expect("run uninstall list scenario");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fixture
        .assert_canaries_intact()
        .expect("canaries intact after uninstall inventory");
}
