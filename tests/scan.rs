use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use mole_linux::scan;

fn fixture() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("mole-linux-scan-{nonce}"));
    fs::create_dir(&path).expect("create fixture");
    path
}

#[test]
fn sparse_file_separates_apparent_and_allocated_size() {
    let dir = fixture();
    let file = fs::File::create(dir.join("sparse")).expect("create sparse file");
    file.set_len(16 * 1024 * 1024).expect("set sparse length");
    let root_apparent = fs::symlink_metadata(&dir)
        .expect("stat root directory")
        .len();

    let report = scan::scan(&dir, &AtomicBool::new(false));

    assert_eq!(report.root.apparent, 16 * 1024 * 1024 + root_apparent);
    assert!(report.root.allocated < report.root.apparent);
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn hardlinks_in_separate_subdirectories_count_once() {
    let dir = fixture();
    fs::create_dir(dir.join("one")).expect("create first subdirectory");
    fs::create_dir(dir.join("two")).expect("create second subdirectory");
    fs::write(dir.join("one/file"), vec![7_u8; 4096]).expect("write file");
    fs::hard_link(dir.join("one/file"), dir.join("two/file")).expect("create hardlink");
    let file_metadata = fs::symlink_metadata(dir.join("one/file")).expect("stat file");
    let one_metadata = fs::symlink_metadata(dir.join("one")).expect("stat first directory");
    let two_metadata = fs::symlink_metadata(dir.join("two")).expect("stat second directory");
    let root_metadata = fs::symlink_metadata(&dir).expect("stat root directory");
    let expected_allocated = (file_metadata.blocks()
        + one_metadata.blocks()
        + two_metadata.blocks()
        + root_metadata.blocks())
        * 512;
    let expected_apparent =
        file_metadata.len() + one_metadata.len() + two_metadata.len() + root_metadata.len();

    let report = scan::scan(&dir, &AtomicBool::new(false));

    assert_eq!(report.root.allocated, expected_allocated);
    assert_eq!(report.root.apparent, expected_apparent);
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn symlinks_count_the_link_and_loops_do_not_recurse() {
    use std::os::unix::fs::symlink;

    let dir = fixture();
    fs::write(dir.join("large"), vec![3_u8; 1024 * 1024]).expect("write target");
    symlink("large", dir.join("to-large")).expect("create target symlink");
    symlink("loop", dir.join("loop")).expect("create symlink loop");
    let link_size = fs::symlink_metadata(dir.join("to-large"))
        .expect("stat symlink")
        .len();

    let report = scan::scan(&dir, &AtomicBool::new(false));
    let target_link = report
        .root
        .children
        .iter()
        .find(|node| node.name == "to-large")
        .expect("link node");
    let loop_link = report
        .root
        .children
        .iter()
        .find(|node| node.name == "loop")
        .expect("loop node");

    assert_eq!(target_link.kind, scan::EntryKind::Symlink);
    assert_eq!(target_link.apparent, link_size);
    assert!(target_link.children.is_empty());
    assert_eq!(loop_link.kind, scan::EntryKind::Symlink);
    assert!(loop_link.children.is_empty());
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn unreadable_subdirectory_is_unavailable_and_parent_is_partial() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fixture();
    let blocked = dir.join("blocked");
    fs::create_dir(&blocked).expect("create blocked directory");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000))
        .expect("remove directory access");
    if fs::read_dir(&blocked).is_ok() {
        eprintln!("SKIP unreadable_subdirectory: process can read the chmod-000 fixture directory");
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))
            .expect("restore fixture access");
        fs::remove_dir_all(dir).expect("remove fixture");
        return;
    }

    let report = scan::scan(&dir, &AtomicBool::new(false));
    let blocked_node = report.root.children.first().expect("blocked node");

    assert!(matches!(
        blocked_node.status,
        scan::NodeStatus::Unavailable(_)
    ));
    assert_eq!(report.root.status, scan::NodeStatus::Partial);
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))
        .expect("restore fixture access");
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn filenames_preserve_non_utf8_and_newline_names() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let dir = fixture();
    let non_utf8 = std::ffi::OsString::from_vec(vec![b'n', b'a', b'm', b'e', 0xff]);
    let newline = std::ffi::OsString::from("line\nbreak");
    fs::write(dir.join(&non_utf8), b"a").expect("write non-UTF-8 name");
    fs::write(dir.join(&newline), b"b").expect("write newline name");

    let report = scan::scan(&dir, &AtomicBool::new(false));
    let names: Vec<_> = report
        .root
        .children
        .iter()
        .map(|node| node.name.as_os_str().as_bytes())
        .collect();

    assert!(names.contains(&non_utf8.as_bytes()));
    assert!(names.contains(&newline.as_bytes()));
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn pre_cancelled_scan_returns_partial_root_without_walking_children() {
    let dir = fixture();
    fs::write(dir.join("child"), b"content").expect("write child");
    let cancel = AtomicBool::new(true);

    let report = scan::scan(&dir, &cancel);

    assert_eq!(report.root.status, scan::NodeStatus::Partial);
    assert!(report.root.children.is_empty());
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn children_sort_by_allocated_descending_then_name() {
    let dir = fixture();
    fs::write(dir.join("z-tie"), vec![1_u8; 1]).expect("write tied file");
    fs::write(dir.join("a-tie"), vec![1_u8; 1]).expect("write tied file");
    fs::write(dir.join("largest"), vec![1_u8; 32 * 1024]).expect("write larger file");

    let report = scan::scan(&dir, &AtomicBool::new(false));
    let names: Vec<_> = report
        .root
        .children
        .iter()
        .map(|node| node.name.to_string_lossy().into_owned())
        .collect();

    assert_eq!(names, ["largest", "a-tie", "z-tie"]);
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn mount_boundary_decision_uses_injected_device_ids() {
    assert!(!scan::is_mount_boundary(42, 42));
    assert!(scan::is_mount_boundary(42, 99));
}

#[test]
fn injected_mount_boundary_is_excluded_without_making_parent_partial() {
    let dir = fixture();
    fs::create_dir(dir.join("mounted")).expect("create mount fixture");
    fs::write(dir.join("counted"), b"visible").expect("write regular child");
    fs::write(dir.join("mounted/hidden"), b"excluded").expect("write excluded child");

    let report = scan::scan_with_device_ids(&dir, &AtomicBool::new(false), |path, metadata| {
        if path.file_name().is_some_and(|name| name == "mounted") {
            metadata.dev().saturating_add(1)
        } else {
            metadata.dev()
        }
    });
    let mounted = report
        .root
        .children
        .iter()
        .find(|node| node.name == "mounted")
        .expect("mount node");
    let counted = fs::metadata(dir.join("counted")).expect("stat counted child");
    let root_metadata = fs::symlink_metadata(&dir).expect("stat root directory");
    let root_allocated = root_metadata.blocks() * 512;
    let counted_allocated = counted.blocks() * 512;

    assert_eq!(
        mounted.status,
        scan::NodeStatus::Excluded("mount boundary".to_owned())
    );
    assert_eq!(mounted.apparent, 0);
    assert_eq!(mounted.allocated, 0);
    assert_eq!(report.root.status, scan::NodeStatus::Complete);
    assert_eq!(report.root.apparent, root_metadata.len() + counted.len());
    assert_eq!(report.root.allocated, root_allocated + counted_allocated);
    fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn deep_tree_scan_completes_on_small_thread_stack() {
    const CHILD_DIR: &str = "MOLE_SCAN_DEEP_FIXTURE";

    if let Some(path) = std::env::var_os(CHILD_DIR) {
        let path = std::path::PathBuf::from(path);
        let result = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || scan::scan(&path, &AtomicBool::new(false)))
            .expect("spawn small-stack scanner")
            .join();
        assert!(result.is_ok(), "scanner thread panicked");
        return;
    }

    let dir = fixture();
    let mut nested = dir.clone();
    for _ in 0..700 {
        nested.push("x");
        fs::create_dir(&nested).expect("create deep directory");
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("deep_tree_scan_completes_on_small_thread_stack")
        .env(CHILD_DIR, &dir)
        .output()
        .expect("run isolated small-stack test");
    fs::remove_dir_all(&dir).expect("remove deep fixture");

    assert!(
        output.status.success(),
        "small-stack subprocess failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
