use std::fs;
use std::time::{Duration, SystemTime};
use tempfile::tempdir;

use mole_linux::purge::{
    Activity, SizeEstimate, classify_activity_with_budget, measure_size_with_budget, scan_artifacts,
};

#[test]
fn activity_detects_recent_project_and_recent_child_under_old_artifact() {
    let home = tempdir().expect("home");
    let recent = home.path().join("recent");
    fs::create_dir_all(&recent).expect("recent dir");
    assert_eq!(
        classify_activity_with_budget(&recent, SystemTime::now(), Duration::from_secs(2)),
        Activity::Recent
    );
    fs::write(recent.join("package.json"), "{}").expect("project marker");
    fs::create_dir(recent.join("node_modules")).expect("recent artifact");
    let candidates = scan_artifacts(&recent, 2).expect("scan recent project");
    assert!(!candidates[0].selected_by_default);

    let artifact = home.path().join("old/node_modules");
    fs::create_dir_all(&artifact).expect("old artifact");
    let child = artifact.join("active.js");
    fs::write(&child, "active").expect("active file");
    filetime_old(&artifact);
    assert_eq!(
        classify_activity_with_budget(&artifact, SystemTime::now(), Duration::from_secs(2)),
        Activity::Recent
    );
}

#[test]
fn activity_marks_old_tree_old_and_probe_budget_or_errors_uncertain() {
    let home = tempdir().expect("home");
    let old = home.path().join("old");
    fs::create_dir_all(&old).expect("old dir");
    fs::write(old.join("old.js"), "old").expect("old file");
    filetime_old(&old);
    filetime_old(&old.join("old.js"));
    assert_eq!(
        classify_activity_with_budget(&old, SystemTime::now(), Duration::from_secs(2)),
        Activity::Old
    );
    assert_eq!(
        classify_activity_with_budget(&old, SystemTime::now(), Duration::ZERO),
        Activity::Uncertain
    );
    assert_eq!(
        classify_activity_with_budget(
            &home.path().join("missing"),
            SystemTime::now(),
            Duration::from_secs(2)
        ),
        Activity::Uncertain
    );
}

#[test]
fn size_timeout_and_filesystem_errors_are_unknown_not_zero() {
    let home = tempdir().expect("home");
    let path = home.path().join("tree");
    fs::create_dir_all(&path).expect("tree");
    fs::write(path.join("file"), [0_u8; 8]).expect("file");
    use std::os::unix::fs::MetadataExt;
    let expected_allocated = fs::metadata(&path)
        .expect("tree metadata")
        .blocks()
        .checked_add(
            fs::metadata(path.join("file"))
                .expect("file metadata")
                .blocks(),
        )
        .and_then(|blocks| blocks.checked_mul(512))
        .expect("allocated bytes");
    assert_eq!(
        measure_size_with_budget(&path, Duration::from_secs(2)),
        SizeEstimate::Known(expected_allocated)
    );
    assert!(matches!(
        measure_size_with_budget(&path, Duration::ZERO),
        SizeEstimate::Unknown(reason) if reason.contains("time budget")
    ));
    assert!(matches!(
        measure_size_with_budget(&home.path().join("missing"), Duration::from_secs(2)),
        SizeEstimate::Unknown(reason) if reason.contains("metadata")
    ));
}

fn filetime_old(path: &std::path::Path) {
    use std::process::Command;
    let status = Command::new("touch")
        .args(["-t", "202001010000"])
        .arg(path)
        .status()
        .expect("touch timestamp");
    assert!(status.success());
}
