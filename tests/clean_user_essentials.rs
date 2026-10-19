use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;
use mole_linux::core::proc::{ProcessState, is_path_open};

#[test]
fn proc_open_path_probe_distinguishes_open_and_idle_files() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let proc_root = temp.path().join("proc");
    let target = temp.path().join("partial.part");
    fs::write(&target, b"partial").expect("partial file");
    fs::create_dir_all(proc_root.join("123/fd")).expect("proc fd directory");
    std::os::unix::fs::symlink(&target, proc_root.join("123/fd/4")).expect("fd link");

    assert_eq!(is_path_open(&proc_root, &target), ProcessState::Running);
    fs::remove_file(proc_root.join("123/fd/4")).expect("remove fake fd link");
    assert_eq!(is_path_open(&proc_root, &target), ProcessState::NotRunning);
}

#[test]
fn dry_run_previews_xdg_user_essentials_without_changing_targets_or_reporting_space_delta() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".test-xdg/cache/thumbnails")).expect("thumbnail directory");
    fs::write(
        home.join(".test-xdg/cache/thumbnails/preview.png"),
        b"thumbnail",
    )
    .expect("thumbnail");
    fs::create_dir_all(home.join(".test-xdg/state/app/log")).expect("user log directory");
    fs::write(home.join(".test-xdg/state/app/log/app.log"), b"old log").expect("log");
    fs::create_dir_all(home.join(".test-xdg/data/Trash/files")).expect("Trash files");
    fs::create_dir_all(home.join(".test-xdg/data/Trash/info")).expect("Trash info");
    fs::write(home.join(".test-xdg/data/Trash/files/old.txt"), b"trashed").expect("Trash item");
    fs::create_dir_all(home.join("Downloads")).expect("Downloads");
    fs::write(home.join("Downloads/active.part"), b"download in progress")
        .expect("partial download");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--dry-run", "--yes"])
        .isolated_home(&home)
        .output()
        .expect("run clean preview");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("thumbnail"), "{stdout}");
    assert!(stdout.contains("app.log"), "{stdout}");
    assert!(stdout.contains("old.txt"), "{stdout}");
    assert!(
        !stdout.contains("Measured statvfs free-space delta"),
        "{stdout}"
    );
    assert!(home.join(".test-xdg/cache/thumbnails/preview.png").exists());
    assert!(home.join(".test-xdg/state/app/log/app.log").exists());
    assert!(home.join(".test-xdg/data/Trash/files/old.txt").exists());
    assert!(home.join("Downloads/active.part").exists());
    assert!(
        !stdout.contains("active.part"),
        "active partial download was offered: {stdout}"
    );
}

#[test]
fn clean_apply_empties_only_trash_and_known_user_cache_targets() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".test-xdg/cache/thumbnails")).expect("thumbnail directory");
    fs::write(
        home.join(".test-xdg/cache/thumbnails/preview.png"),
        b"thumbnail",
    )
    .expect("thumbnail");
    fs::create_dir_all(home.join(".test-xdg/data/Trash/files")).expect("Trash files");
    fs::create_dir_all(home.join(".test-xdg/data/Trash/info")).expect("Trash info");
    fs::write(home.join(".test-xdg/data/Trash/files/old.txt"), b"trashed").expect("Trash item");
    fs::create_dir_all(home.join("Documents")).expect("documents");
    fs::write(home.join("Documents/keep.txt"), b"authored").expect("document");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--yes", "--select", "user-essentials"])
        .isolated_home(&home)
        .output()
        .expect("run clean apply");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!home.join(".test-xdg/cache/thumbnails/preview.png").exists());
    assert!(!home.join(".test-xdg/data/Trash/files/old.txt").exists());
    assert!(home.join("Documents/keep.txt").exists());
}

#[test]
fn partial_downloads_are_kept_when_procfs_cannot_prove_they_are_idle() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join("Downloads")).expect("Downloads");
    let stale = home.join("Downloads/stale.crdownload");
    let recent = home.join("Downloads/recent.part");
    fs::write(&stale, b"stale").expect("stale partial");
    fs::write(&recent, b"recent").expect("recent partial");
    let aged = Command::new("touch")
        .args(["-d", "8 days ago"])
        .arg(&stale)
        .status()
        .expect("age stale partial");
    assert!(aged.success());

    let preview = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--dry-run", "--yes", "--select", "user-essentials"])
        .isolated_home(&home)
        .output()
        .expect("preview stale partial");
    assert!(preview.status.success());
    let stdout = String::from_utf8_lossy(&preview.stdout);
    assert!(stdout.contains("open-file state unavailable"), "{stdout}");
    assert!(
        !stdout.contains("stale.crdownload"),
        "unknown open state offered: {stdout}"
    );
    assert!(
        !stdout.contains("recent.part"),
        "recent download offered: {stdout}"
    );

    let applied = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--yes", "--select", "user-essentials"])
        .isolated_home(&home)
        .output()
        .expect("apply stale partial cleanup");
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(stale.exists());
    assert!(recent.exists());
}

#[test]
fn coredumps_are_reported_as_attention_without_an_apply_action() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("shim directory");
    let log = temp.path().join("coredumpctl-args");
    let shim = bin.join("coredumpctl");
    fs::write(
        &shim,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COREDUMPCTL_LOG\"\nprintf '%s\\n' '[{\"PID\":1},{\"PID\":2}]'\n",
    )
    .expect("write coredumpctl shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("make shim executable");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--yes", "--select", "coredumps"])
        .isolated_home(&home)
        .env("PATH", path)
        .env("COREDUMPCTL_LOG", &log)
        .output()
        .expect("run coredump report");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("coredumpctl list count: 2"), "{stdout}");
    assert!(stdout.contains("readable size:"), "{stdout}");
    assert!(stdout.contains("retention is managed by systemd-tmpfiles (/usr/lib/tmpfiles.d/systemd.conf, default 3d) and coredump.conf MaxUse=; mole-linux does not delete system coredumps"), "{stdout}");
    assert!(stdout.contains("attention: 1"), "{stdout}");
    assert!(stdout.contains("applied: 0"), "{stdout}");
    assert_eq!(
        fs::read_to_string(log).expect("shim args"),
        "list --no-pager --json=short\n"
    );
}

#[test]
fn coredump_report_is_unavailable_when_owner_tool_is_missing_or_denied() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let empty_path = temp.path().join("empty-bin");
    fs::create_dir(&empty_path).expect("empty PATH directory");

    let missing = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--dry-run", "--select", "coredumps"])
        .isolated_home(&home)
        .env("PATH", &empty_path)
        .output()
        .expect("run missing-tool report");
    let missing_stdout = String::from_utf8_lossy(&missing.stdout);
    assert!(
        missing_stdout.contains("system: unavailable"),
        "{missing_stdout}"
    );

    let denied = empty_path.join("coredumpctl");
    fs::write(&denied, "#!/bin/sh\necho 'Permission denied' >&2\nexit 1\n")
        .expect("write denied shim");
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o755)).expect("make shim executable");
    let denied_output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--dry-run", "--select", "coredumps"])
        .isolated_home(&home)
        .env("PATH", &empty_path)
        .output()
        .expect("run denied-tool report");
    let denied_stdout = String::from_utf8_lossy(&denied_output.stdout);
    assert!(
        denied_stdout.contains("system: unavailable"),
        "{denied_stdout}"
    );
}
