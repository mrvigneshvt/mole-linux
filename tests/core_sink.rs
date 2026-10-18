use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::sink::{DeleteMode, Identity, delete, delete_batch};

fn xdg(home: &Path) -> XdgPaths {
    XdgPaths::from_values(home.to_path_buf(), None, None, None, None)
        .expect("temporary home is absolute")
}

fn identity(path: &Path) -> Identity {
    Identity::capture(path).expect("capture target identity")
}

#[test]
fn dry_run_preserves_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("candidate.txt");
    fs::write(&target, "data").expect("target");

    let result = delete(
        &target,
        &identity(&target),
        DeleteMode::DryRun,
        &home,
        &xdg(&home),
    );
    assert!(result.is_ok());
    assert_eq!(fs::read(&target).expect("target remains"), b"data");
}

#[test]
fn trash_moves_item_and_writes_freedesktop_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("candidate.txt");
    fs::write(&target, "data").expect("target");

    let result = delete(
        &target,
        &identity(&target),
        DeleteMode::Trash,
        &home,
        &xdg(&home),
    );
    let trashed_path = result.expect("trash succeeds");
    assert!(!target.exists());
    assert_eq!(fs::read(&trashed_path).expect("trashed payload"), b"data");
    let info = trashed_path
        .parent()
        .expect("files directory")
        .parent()
        .expect("trash root")
        .join("info")
        .join(format!(
            "{}.trashinfo",
            trashed_path.file_name().expect("name").to_string_lossy()
        ));
    let contents = fs::read_to_string(info).expect("trashinfo");
    assert!(contents.starts_with("[Trash Info]\nPath="));
    assert!(contents.contains("\nDeletionDate="));
}

#[test]
fn changed_identity_is_refused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("candidate.txt");
    fs::write(&target, "before").expect("target");
    let preview = identity(&target);
    fs::write(&target, "after and changed").expect("replace content");

    let result = delete(&target, &preview, DeleteMode::Permanent, &home, &xdg(&home));
    assert!(result.is_err());
    assert_eq!(
        fs::read(&target).expect("preserved target"),
        b"after and changed"
    );
}

#[test]
fn batch_orders_descendants_before_parents() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let parent = home.join("parent");
    let child = parent.join("child.txt");
    fs::create_dir(&parent).expect("parent");
    fs::write(&child, "data").expect("child");
    let entries = vec![
        (parent.clone(), identity(&parent)),
        (child.clone(), identity(&child)),
    ];

    let results = delete_batch(entries, DeleteMode::Trash, &home, &xdg(&home), || false);
    assert!(results[0].1.is_ok(), "child should be processed first");
    assert!(
        results[1].1.is_err(),
        "parent mtime changes when its child is moved"
    );
    assert!(
        parent.exists(),
        "changed parent must remain for a fresh preview"
    );
}

#[test]
fn trash_preserves_non_utf8_name() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join(std::ffi::OsString::from_vec(b"odd-\xff-name".to_vec()));
    fs::write(&target, "data").expect("target");

    let trashed = delete(
        &target,
        &identity(&target),
        DeleteMode::Trash,
        &home,
        &xdg(&home),
    )
    .expect("non-UTF-8 names are valid filesystem paths");
    assert_eq!(
        trashed.file_name().expect("trash name").as_bytes(),
        b"odd-\xff-name"
    );
    assert_eq!(fs::read(&trashed).expect("payload"), b"data");
}

#[test]
fn cancellation_stops_batch_before_any_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("candidate.txt");
    fs::write(&target, "data").expect("target");
    let entries = vec![(target.clone(), identity(&target))];

    let results = delete_batch(entries, DeleteMode::Permanent, &home, &xdg(&home), || true);
    assert_eq!(results.len(), 1);
    assert!(matches!(
        results[0].1,
        Err(mole_linux::core::sink::DeleteError::Cancelled)
    ));
    assert_eq!(fs::read(&target).expect("target remains"), b"data");
}

#[test]
fn symlink_replacement_after_preview_is_refused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("candidate");
    let outside = temp.path().join("outside");
    fs::write(&target, "approved").expect("target");
    fs::write(&outside, "protected").expect("outside");
    let preview = identity(&target);
    fs::remove_file(&target).expect("replace target in test fixture");
    std::os::unix::fs::symlink(&outside, &target).expect("replacement symlink");

    let result = delete(&target, &preview, DeleteMode::Permanent, &home, &xdg(&home));
    assert!(result.is_err());
    assert_eq!(fs::read(&outside).expect("outside preserved"), b"protected");
}

#[test]
fn permanent_directory_removal_is_fd_relative_and_recursive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("tree");
    fs::create_dir_all(target.join("nested")).expect("nested directory");
    fs::write(target.join("nested/file"), "data").expect("nested file");

    delete(
        &target,
        &identity(&target),
        DeleteMode::Permanent,
        &home,
        &xdg(&home),
    )
    .expect("remove tree");
    assert!(!target.exists());
}
