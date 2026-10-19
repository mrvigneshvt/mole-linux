use std::fs;
use std::process::Command;
#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

#[test]
fn clean_dry_run_reports_preview_and_measured_space_without_mutating_home() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("create temporary home");
    let cache = home.join(".cache/thumbnails");
    fs::create_dir_all(&cache).expect("create thumbnail cache");
    fs::write(cache.join("canary"), b"keep during preview").expect("seed cache file");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--dry-run"])
        .isolated_home(&home)
        .output()
        .expect("run clean preview");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Clean preview"), "{stdout}");
    assert!(
        stdout.contains("Measured statvfs free-space delta"),
        "{stdout}"
    );
    assert!(cache.join("canary").exists(), "dry-run changed the target");
    let operations = fs::read_to_string(home.join(".test-xdg/state/mole-linux/operations.log"))
        .expect("operation log written under temporary HOME");
    assert!(operations.contains("clean-dry-run run="));
    assert!(operations.contains("session started"));
    assert!(
        operations.contains("Z =========="),
        "timestamps need a UTC marker: {operations}"
    );
}

#[test]
fn clean_accepts_yes_for_confirmed_apply_mode() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("create temporary home");
    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--yes", "--dry-run"])
        .isolated_home(&home)
        .output()
        .expect("run clean");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn clean_accepts_rule_filters_and_records_a_zero_rule_preview() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("create temporary home");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args([
            "clean",
            "--dry-run",
            "--categories",
            "user-cache",
            "--select",
            "thumbnail-cache",
            "--exclude",
            "browser-cache",
        ])
        .isolated_home(&home)
        .output()
        .expect("run clean preview");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Discovered: 0"));
}
