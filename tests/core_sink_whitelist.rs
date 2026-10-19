use std::fs;
use std::path::{Path, PathBuf};

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::sink::{
    DeleteError, DeleteMode, Identity, SinkContext, delete, delete_batch,
};
use mole_linux::core::whitelist::Whitelist;

fn xdg(home: &Path) -> XdgPaths {
    XdgPaths::from_values(home.to_path_buf(), None, None, None, None)
        .expect("temporary home is absolute")
}

fn configured_whitelist(home: &Path, patterns: &[PathBuf]) -> (XdgPaths, Whitelist) {
    let xdg_paths = xdg(home);
    Whitelist::save_patterns(
        &xdg_paths,
        &patterns
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    )
    .expect("save fixture whitelist");
    let whitelist = Whitelist::load(&xdg_paths).expect("load fixture whitelist");
    (xdg_paths, whitelist)
}

fn identity(path: &Path) -> Identity {
    Identity::capture(path).expect("capture target identity")
}

#[test]
fn sink_refuses_a_whitelisted_file_after_identity_revalidation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("keep.txt");
    fs::write(&target, "keep").expect("target");
    let (xdg, whitelist) = configured_whitelist(&home, std::slice::from_ref(&target));
    let context = SinkContext::new(xdg, whitelist);

    let result = delete(&target, &identity(&target), DeleteMode::Permanent, &context);

    assert!(matches!(result, Err(DeleteError::Whitelisted)));
    assert_eq!(fs::read(&target).expect("preserved target"), b"keep");

    let stale_identity = identity(&target);
    fs::write(&target, "changed target").expect("change after preview");
    let result = delete(&target, &stale_identity, DeleteMode::Permanent, &context);
    assert!(matches!(result, Err(DeleteError::IdentityChanged)));
}

#[test]
fn sink_refuses_directory_removal_when_a_descendant_is_whitelisted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("tree");
    fs::create_dir(&target).expect("tree");
    let protected_child = target.join("keep.txt");
    fs::write(&protected_child, "keep").expect("protected child");
    let (xdg, whitelist) = configured_whitelist(&home, std::slice::from_ref(&protected_child));
    let context = SinkContext::new(xdg, whitelist);

    let result = delete(&target, &identity(&target), DeleteMode::Permanent, &context);

    assert!(matches!(result, Err(DeleteError::Whitelisted)));
    assert_eq!(
        fs::read(&protected_child).expect("preserved child"),
        b"keep"
    );
}

#[cfg(unix)]
#[test]
fn sink_checks_whitelist_against_the_resolved_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let real = home.join("real");
    fs::create_dir_all(&real).expect("real directory");
    let target = real.join("keep.txt");
    fs::write(&target, "keep").expect("target");
    let alias = home.join("alias");
    std::os::unix::fs::symlink(&real, &alias).expect("directory alias");
    let aliased_target = alias.join("keep.txt");
    let (xdg, whitelist) = configured_whitelist(&home, std::slice::from_ref(&target));
    let context = SinkContext::new(xdg, whitelist);

    let result = delete(
        &aliased_target,
        &identity(&aliased_target),
        DeleteMode::Permanent,
        &context,
    );

    assert!(matches!(result, Err(DeleteError::Whitelisted)));
    assert_eq!(fs::read(&target).expect("preserved target"), b"keep");
}

#[test]
fn batch_refuses_whitelisted_targets_and_deletes_an_unrelated_sibling() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let protected = home.join("keep.txt");
    let sibling = home.join("remove.txt");
    fs::write(&protected, "keep").expect("protected target");
    fs::write(&sibling, "remove").expect("unrelated sibling");
    let (xdg, whitelist) = configured_whitelist(&home, std::slice::from_ref(&protected));
    let context = SinkContext::new(xdg, whitelist);

    let results = delete_batch(
        vec![
            (protected.clone(), identity(&protected)),
            (sibling.clone(), identity(&sibling)),
        ],
        DeleteMode::Permanent,
        &context,
        || false,
    );

    assert!(matches!(results[0].1, Err(DeleteError::Whitelisted)));
    assert!(results[1].1.is_ok());
    assert!(protected.exists());
    assert!(!sibling.exists());
}
