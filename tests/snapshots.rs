use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tempfile::tempdir;

fn install_shim(bin: &std::path::Path, name: &str, script: &str) {
    let path = bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("write shim");
    let mut permissions = std::fs::metadata(&path)
        .expect("shim metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("make shim executable");
}

fn run_with_path(path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .arg("snapshots")
        .env("PATH", path)
        .output()
        .expect("run snapshots")
}

#[test]
fn snapshots_lists_available_tools_and_explains_retained_space_read_only() {
    let temp = tempdir().expect("temp dir");
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).expect("bin dir");
    let calls = temp.path().join("calls");
    install_shim(
        &bin,
        "btrfs",
        &format!(
            "printf 'btrfs %s\\n' \"$*\" >> '{}'; printf '%s\\n' 'ID 256 gen 10 top level 5 path @' 'ID 257 gen 11 top level 5 path @snapshots/1/snapshot'",
            calls.display()
        ),
    );
    install_shim(
        &bin,
        "snapper",
        &format!(
            "printf 'snapper %s\\n' \"$*\" >> '{}'; printf '%s\\n' ' # | Type   | Description' '1  | single | pre-update' '2  | single | post-update'",
            calls.display()
        ),
    );

    let output = run_with_path(&bin);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Btrfs subvolumes/snapshots (2 entries)"),
        "{stdout}"
    );
    assert!(stdout.contains("Snapper snapshots (2 entries)"), "{stdout}");
    assert!(
        stdout.contains("Snapshots can retain shared filesystem extents"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(calls).expect("shim calls"),
        "btrfs subvolume list /\nsnapper list\n"
    );
}

#[test]
fn snapshots_reports_unavailable_when_tools_are_missing() {
    let temp = tempdir().expect("temp dir");
    let output = run_with_path(temp.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Unavailable: neither btrfs nor snapper is installed"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn snapshots_marks_tool_errors_unavailable_instead_of_reporting_zero() {
    let temp = tempdir().expect("temp dir");
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).expect("bin dir");
    install_shim(&bin, "btrfs", "exit 1");
    let output = run_with_path(&bin);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Btrfs subvolumes/snapshots: Unavailable"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("Btrfs subvolumes/snapshots (0 entries)"),
        "{stdout}"
    );
}
