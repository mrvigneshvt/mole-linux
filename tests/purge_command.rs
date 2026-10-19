use std::fs::{self, File, FileTimes};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use mole_linux::commands::purge::{ReviewedCandidate, apply_selected};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::outcome::Outcome;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::sink::{DeleteMode, SinkContext};
use mole_linux::core::whitelist::Whitelist;
use mole_linux::purge::{Activity, Candidate, SizeEstimate};
use tempfile::tempdir;

fn context(home: &std::path::Path) -> SinkContext {
    let xdg =
        XdgPaths::from_values(home.to_path_buf(), None, None, None, None).expect("valid temp home");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    SinkContext::new(xdg, whitelist)
}

fn reviewed(path: PathBuf, activity: Activity, protected: bool) -> ReviewedCandidate {
    ReviewedCandidate::capture(Candidate {
        path,
        activity,
        protected,
        size: SizeEstimate::Known(4096),
        cachedir_tag: false,
        selected_by_default: activity == Activity::Old && !protected,
    })
    .expect("capture identity")
}

fn age(paths: &[&std::path::Path]) {
    let modified = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
    for path in paths {
        File::open(path)
            .expect("open fixture path")
            .set_times(FileTimes::new().set_modified(modified))
            .expect("age fixture");
    }
}

#[test]
fn apply_selected_trashes_only_explicit_paths_and_reports_trash_not_freed_space() {
    let temp = tempdir().expect("temp home");
    let home = temp.path().join("home");
    let artifact = home.join("work/app/target");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(home.join("work/app/Cargo.toml"), "[package]\n").expect("project marker");
    fs::write(artifact.join("cache"), "cache").expect("artifact content");
    age(&[&artifact, &artifact.join("cache")]);
    let candidate = reviewed(artifact.clone(), Activity::Old, false);
    let log = OpLog::with_state_home(&temp.path().join("state"));

    let report = apply_selected(
        vec![candidate],
        std::slice::from_ref(&artifact),
        DeleteMode::Trash,
        &context(&home),
        &log,
    )
    .expect("apply purge");

    assert_eq!(report.trashed, 1);
    assert_eq!(report.outcome, Some(Outcome::Applied));
    assert_eq!(report.skipped, 0);
    assert_eq!(report.failed, 0);
    assert!(!artifact.exists());
    let trash = home.join(".local/share/Trash/files");
    assert_eq!(fs::read_dir(trash).expect("trash").count(), 1);
    let operations = fs::read_to_string(temp.path().join("state/mole-linux/operations.log"))
        .expect("operation log");
    assert!(operations.contains("purge run="));
    assert!(operations.contains("session started"));
    assert!(operations.contains("trash applied"));
    assert!(operations.contains("1 items, 1 paths"));
    assert!(
        report
            .summary(false)
            .contains("space is not freed until Trash is emptied")
    );
}

#[test]
fn recent_protected_and_unselected_paths_are_never_sent_to_the_sink() {
    let temp = tempdir().expect("temp home");
    let home = temp.path().join("home");
    let root = home.join("work/app");
    fs::create_dir_all(root.join("recent")).expect("recent artifact");
    fs::create_dir_all(root.join("protected")).expect("protected artifact");
    fs::create_dir_all(root.join("old")).expect("old artifact");
    fs::write(root.join("package.json"), "{}").expect("project marker");
    age(&[&root.join("old")]);
    let candidates = vec![
        reviewed(root.join("recent"), Activity::Recent, false),
        reviewed(root.join("protected"), Activity::Old, true),
        reviewed(root.join("old"), Activity::Old, false),
    ];
    let selected = vec![
        root.join("recent"),
        root.join("protected"),
        root.join("old"),
    ];
    let log = OpLog::with_state_home(&temp.path().join("state"));

    let report = apply_selected(
        candidates,
        &selected,
        DeleteMode::DryRun,
        &context(&home),
        &log,
    )
    .expect("review purge");

    assert_eq!(report.previewed, 2);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.outcome, Some(Outcome::Skipped));
    assert!(root.join("recent").exists());
    assert!(root.join("protected").exists());
    assert!(root.join("old").exists());
}

#[test]
fn changed_artifact_identity_is_skipped_at_sink() {
    let temp = tempdir().expect("temp home");
    let home = temp.path().join("home");
    let artifact = home.join("work/app/target");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(home.join("work/app/Cargo.toml"), "[package]\n").expect("project marker");
    fs::write(artifact.join("old"), "old").expect("old content");
    age(&[&artifact, &artifact.join("old")]);
    let candidate = reviewed(artifact.clone(), Activity::Old, false);
    fs::rename(&artifact, home.join("work/app/target-original")).expect("replace previewed path");
    fs::create_dir(&artifact).expect("replacement artifact");
    age(&[&artifact]);
    let log = OpLog::with_state_home(&temp.path().join("state"));

    let report = apply_selected(
        vec![candidate],
        std::slice::from_ref(&artifact),
        DeleteMode::Trash,
        &context(&home),
        &log,
    )
    .expect("apply with changed identity");

    assert_eq!(report.skipped, 1);
    assert!(report.skipped_paths[0].1.contains("identity changed"));
    assert!(artifact.exists());
    assert!(!home.join(".local/share/Trash/files").exists());
}

#[test]
fn replaced_configured_root_is_skipped_by_sink_identity_revalidation() {
    let temp = tempdir().expect("temp home");
    let home = temp.path().join("home");
    let configured_root = home.join("www");
    let artifact = configured_root.join("app/node_modules");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(configured_root.join("app/package.json"), "{}").expect("project marker");
    fs::write(artifact.join("old"), "old").expect("artifact content");
    age(&[&artifact, &artifact.join("old")]);
    let candidate = reviewed(artifact.clone(), Activity::Old, false);
    let displaced_root = home.join("www-original");
    let replacement = home.join("www-replacement");
    fs::rename(&configured_root, &displaced_root).expect("move configured root");
    fs::create_dir_all(replacement.join("app/node_modules")).expect("replacement artifact");
    fs::write(replacement.join("app/package.json"), "{}").expect("replacement marker");
    std::os::unix::fs::symlink(&replacement, &configured_root).expect("replace root with symlink");
    age(&[&artifact]);
    let log = OpLog::with_state_home(&temp.path().join("state"));

    let report = apply_selected(
        vec![candidate],
        std::slice::from_ref(&artifact),
        DeleteMode::Trash,
        &context(&home),
        &log,
    )
    .expect("apply after root replacement");

    assert_eq!(report.skipped, 1);
    assert!(report.skipped_paths[0].1.contains("identity changed"));
    assert!(displaced_root.join("app/node_modules/old").exists());
    assert!(replacement.join("app/node_modules").exists());
    assert!(!home.join(".local/share/Trash/files").exists());
}

#[test]
fn activity_added_after_review_is_skipped_before_sink() {
    let temp = tempdir().expect("temp home");
    let home = temp.path().join("home");
    let artifact = home.join("work/app/target");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(home.join("work/app/Cargo.toml"), "[package]\n").expect("project marker");
    let content = artifact.join("cache");
    fs::write(&content, "old").expect("old content");
    let old_time = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
    for path in [
        &artifact,
        &content,
        &home.join("work/app/Cargo.toml"),
        &home.join("work/app"),
    ] {
        File::open(path)
            .expect("open fixture path")
            .set_times(FileTimes::new().set_modified(old_time))
            .expect("age fixture");
    }
    let candidate = reviewed(artifact.clone(), Activity::Old, false);
    File::open(&content)
        .expect("open cache file")
        .set_times(FileTimes::new().set_modified(SystemTime::now()))
        .expect("mark activity recent");
    let log = OpLog::with_state_home(&temp.path().join("state"));

    let report = apply_selected(
        vec![candidate],
        std::slice::from_ref(&artifact),
        DeleteMode::Trash,
        &context(&home),
        &log,
    )
    .expect("apply after activity changed");

    assert_eq!(report.skipped, 1);
    assert!(report.skipped_paths[0].1.contains("recently active"));
    assert!(artifact.exists());
    assert!(!home.join(".local/share/Trash/files").exists());
}

#[test]
fn purge_cli_dry_run_reviews_defaults_and_yes_moves_only_to_trash() {
    let temp = tempdir().expect("temp home");
    let home = temp.path().join("home");
    let artifact = home.join("www/app/node_modules");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(home.join("www/app/package.json"), "{}").expect("project marker");
    fs::write(artifact.join("cache"), "cache").expect("artifact content");
    let old_time = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
    for path in [
        &artifact,
        &artifact.join("cache"),
        &home.join("www/app/package.json"),
        &home.join("www/app"),
    ] {
        File::open(path)
            .expect("open fixture path")
            .set_times(FileTimes::new().set_modified(old_time))
            .expect("age fixture");
    }
    let binary = env!("CARGO_BIN_EXE_mole-linux");
    let preview = std::process::Command::new(binary)
        .args(["purge", "--dry-run"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .output()
        .expect("run purge preview");
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Dry run:"));
    assert!(artifact.exists());

    let unattended = std::process::Command::new(binary)
        .args(["purge"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .output()
        .expect("run unattended purge");
    assert!(!unattended.status.success());
    assert!(String::from_utf8_lossy(&unattended.stderr).contains("requires --yes"));
    assert!(artifact.exists());

    let applied = std::process::Command::new(binary)
        .args(["purge", "--yes"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .output()
        .expect("run purge apply");
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(
        String::from_utf8_lossy(&applied.stdout)
            .contains("space is not freed until Trash is emptied")
    );
    assert!(!artifact.exists());
    assert_eq!(
        fs::read_dir(home.join(".local/share/Trash/files"))
            .expect("Trash")
            .count(),
        1
    );
}
