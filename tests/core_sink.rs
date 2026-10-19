use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::sink::{
    DeleteError, DeleteMode, Identity, SinkContext, delete, delete_batch,
};
use mole_linux::core::whitelist::Whitelist;

fn xdg(home: &Path) -> XdgPaths {
    XdgPaths::from_values(home.to_path_buf(), None, None, None, None)
        .expect("temporary home is absolute")
}

fn identity(path: &Path) -> Identity {
    Identity::capture(path).expect("capture target identity")
}

fn context(home: &Path) -> SinkContext {
    let xdg = xdg(home);
    let whitelist = Whitelist::load(&xdg).expect("load empty temporary whitelist");
    SinkContext::new(xdg, whitelist)
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
        &context(&home),
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
        &context(&home),
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

    let result = delete(&target, &preview, DeleteMode::Permanent, &context(&home));
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

    let results = delete_batch(entries, DeleteMode::Trash, &context(&home), || false);
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
        &context(&home),
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

    let results = delete_batch(entries, DeleteMode::Permanent, &context(&home), || true);
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

    let result = delete(&target, &preview, DeleteMode::Permanent, &context(&home));
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
        &context(&home),
    )
    .expect("remove tree");
    assert!(!target.exists());
}

#[test]
fn ancestor_symlink_swap_after_preview_refuses_decoy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join("ancestor/nested")).expect("source ancestor");
    let target = home.join("ancestor/nested/item");
    fs::write(&target, "approved").expect("preview target");
    let preview = identity(&target);
    let decoy = home.join("decoy/nested");
    fs::create_dir_all(&decoy).expect("decoy dirs");
    fs::write(decoy.join("item"), "decoy survives").expect("decoy target");
    fs::rename(home.join("ancestor"), home.join("moved")).expect("move original ancestor");
    std::os::unix::fs::symlink(home.join("decoy"), home.join("ancestor"))
        .expect("swap ancestor for symlink");

    let result = delete(&target, &preview, DeleteMode::Permanent, &context(&home));
    assert!(result.is_err());
    assert_eq!(
        fs::read(decoy.join("item")).expect("decoy remains"),
        b"decoy survives"
    );
}

#[test]
fn sink_rejects_ssh_key_and_git_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".ssh")).expect("ssh dir");
    fs::create_dir_all(home.join("repo/.git")).expect("repo metadata");
    let key = home.join(".ssh/id_x");
    let git = home.join("repo/.git");
    fs::write(&key, "key").expect("key");
    fs::write(git.join("config"), "repo").expect("git config");

    for target in [&key, &git] {
        let result = delete(
            target,
            &identity(target),
            DeleteMode::Permanent,
            &context(&home),
        );
        assert!(
            matches!(result, Err(DeleteError::InvalidPath(_))),
            "{target:?} must be rejected"
        );
    }
    assert_eq!(fs::read(&key).expect("key untouched"), b"key");
    assert_eq!(
        fs::read(git.join("config")).expect("git untouched"),
        b"repo"
    );
}

#[test]
fn trash_directory_symlink_hijack_refuses_without_moving_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir_all(home.join(".local/share")).expect("xdg share");
    fs::create_dir(&elsewhere).expect("hijack destination");
    std::os::unix::fs::symlink(&elsewhere, home.join(".local/share/Trash"))
        .expect("Trash hijack symlink");
    let target = home.join("item");
    fs::write(&target, "keep").expect("target");

    let result = delete(
        &target,
        &identity(&target),
        DeleteMode::Trash,
        &context(&home),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&target).expect("source stays"), b"keep");
    assert!(
        fs::read_dir(elsewhere)
            .expect("hijack destination intact")
            .next()
            .is_none()
    );
}

#[test]
fn same_basename_trash_items_get_distinct_names_and_correct_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join("one")).expect("one");
    fs::create_dir_all(home.join("two")).expect("two");
    let first = home.join("one/shared.txt");
    let second = home.join("two/shared.txt");
    fs::write(&first, "first").expect("first item");
    fs::write(&second, "second").expect("second item");

    let first_trash = delete(
        &first,
        &identity(&first),
        DeleteMode::Trash,
        &context(&home),
    )
    .expect("first trash");
    let second_trash = delete(
        &second,
        &identity(&second),
        DeleteMode::Trash,
        &context(&home),
    )
    .expect("second trash");
    assert_ne!(first_trash.file_name(), second_trash.file_name());
    let info_dir = home.join(".local/share/Trash/info");
    for (trashed, original) in [(&first_trash, &first), (&second_trash, &second)] {
        let mut info_name = trashed.file_name().expect("trash filename").to_os_string();
        info_name.push(".trashinfo");
        let contents = fs::read_to_string(info_dir.join(info_name)).expect("matching trashinfo");
        assert!(contents.contains(&format!("Path={}", original.display())));
    }
}

#[test]
fn unreadable_parent_after_preview_refuses_delete() {
    if rustix::process::getuid().as_raw() == 0 {
        eprintln!("SKIP unreadable parent: running as root bypasses mode bits");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let parent = home.join("parent");
    fs::create_dir_all(&parent).expect("parent");
    let target = parent.join("item");
    fs::write(&target, "keep").expect("target");
    let preview = identity(&target);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o0)).expect("make parent unreadable");

    let result = delete(&target, &preview, DeleteMode::Permanent, &context(&home));
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).expect("restore parent");
    assert!(result.is_err());
    assert_eq!(fs::read(&target).expect("target remains"), b"keep");
}

#[test]
fn nested_unwritable_directory_returns_partial_error_without_panicking() {
    if rustix::process::getuid().as_raw() == 0 {
        eprintln!("SKIP unwritable nested directory: running as root bypasses mode bits");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let tree = home.join("tree");
    let blocked = tree.join("blocked");
    fs::create_dir_all(&blocked).expect("nested directory");
    fs::write(blocked.join("keep"), "keep").expect("nested payload");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o0))
        .expect("make nested dir unreadable");

    let result = delete(
        &tree,
        &identity(&tree),
        DeleteMode::Permanent,
        &context(&home),
    );
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).expect("restore permissions");
    let message = result
        .expect_err("unreadable nested directory is refused")
        .to_string();
    assert!(
        message.contains("partial"),
        "partial result should be explicit: {message}"
    );
    assert_eq!(
        fs::read(blocked.join("keep")).expect("nested payload preserved"),
        b"keep"
    );
}

#[test]
fn cancellation_after_first_item_marks_only_remaining_items_cancelled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let first = home.join("first");
    let second = home.join("second");
    fs::write(&first, "first").expect("first");
    fs::write(&second, "second").expect("second");
    let entries = vec![
        (first.clone(), identity(&first)),
        (second.clone(), identity(&second)),
    ];
    let mut checks = 0;

    let results = delete_batch(entries, DeleteMode::Permanent, &context(&home), || {
        checks += 1;
        checks > 1
    });
    assert_eq!(results.len(), 2);
    assert!(results[0].1.is_ok());
    assert!(matches!(results[1].1, Err(DeleteError::Cancelled)));
    assert!(!first.exists());
    assert!(second.exists());
}
