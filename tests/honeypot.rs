#[path = "honeypot/fixtures.rs"]
mod fixtures;

use std::fs;
use std::process::Command;

#[test]
fn builds_realistic_home_and_canaries_survive() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");

    for relative in [
        ".config",
        ".cache",
        ".local/share",
        ".mozilla/firefox/profile",
        ".ssh/id_ed25519",
        ".env",
        ".npm/_cacache",
        ".cache/pnpm",
        ".cache/uv",
        ".cargo/registry",
        ".gradle/caches",
        "project/node_modules",
        "project/target",
        "project/.git",
        "project-linked/.git",
        "Downloads/installer.AppImage",
        ".local/share/Trash/files/old",
        "browser/Default/Cache",
        "browser/Default/Cookies",
        "browser/Default/Cookies-wal",
    ] {
        assert!(home.join(relative).exists(), "missing {relative}");
    }

    fixture.assert_canaries_intact().expect("canaries intact");
    fs::write(home.join(".ssh/id_ed25519"), "modified").expect("modify canary");
    assert!(fixture.assert_canaries_intact().is_err());
}

#[test]
fn generated_tool_shims_log_argv_and_print_canned_output() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let bin = temp.path().join("bin");
    let log = temp.path().join("calls.log");
    fixtures::generate_tool_shims(&bin, &log).expect("generate shims");

    let output = Command::new(bin.join("pacman"))
        .args(["-Q", "example"])
        .output()
        .expect("run shim");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "mole-linux-shim pacman\n"
    );
    assert_eq!(
        fs::read_to_string(log).expect("read log"),
        "pacman\t-Q\texample\n"
    );
}

#[test]
fn fixture_builder_refuses_to_overlay_an_existing_home() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("create existing home");
    fs::write(home.join("keep"), "unchanged").expect("seed existing file");

    assert!(fixtures::HomeFixture::build(&home).is_err());
    assert_eq!(
        fs::read_to_string(home.join("keep")).expect("read existing file"),
        "unchanged"
    );
}
