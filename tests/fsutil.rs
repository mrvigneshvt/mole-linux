use std::os::unix::fs::PermissionsExt;

use mole_linux::core::fsutil::atomic_replace;
use tempfile::tempdir;

#[test]
fn atomic_replace_preserves_existing_mode_and_replaces_contents() {
    let dir = tempdir().expect("temp directory");
    let path = dir.path().join("rc");
    std::fs::write(&path, "old contents\n").expect("seed file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("set mode");

    atomic_replace(&path, b"new contents\n").expect("atomic replace");

    assert_eq!(
        std::fs::read(&path).expect("read replacement"),
        b"new contents\n"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("replacement metadata")
            .permissions()
            .mode()
            & 0o7777,
        0o640
    );
    assert_eq!(
        std::fs::read_dir(dir.path())
            .expect("list directory")
            .count(),
        1
    );
}

#[test]
fn atomic_replace_cleans_temporary_file_when_rename_fails() {
    let dir = tempdir().expect("temp directory");
    let target = dir.path().join("target");
    std::fs::create_dir(&target).expect("target directory");
    std::fs::write(target.join("keep"), b"keep").expect("seed target");

    assert!(atomic_replace(&target, b"replacement").is_err());

    assert_eq!(
        std::fs::read(target.join("keep")).expect("preserved child"),
        b"keep"
    );
    assert_eq!(
        std::fs::read_dir(dir.path())
            .expect("list directory")
            .count(),
        1
    );
}
