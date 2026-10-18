use std::fs;
use std::path::Path;

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::validate::{ValidationError, validate_path_for_deletion};
use mole_linux::core::whitelist::Whitelist;

fn xdg(home: &Path, config: &Path) -> XdgPaths {
    XdgPaths::from_values(
        home.to_path_buf(),
        Some(config.to_path_buf()),
        None,
        None,
        None,
    )
    .expect("valid temporary XDG paths")
}

#[test]
fn loads_patterns_from_xdg_mole_linux_whitelist_and_matches_globs() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("home");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::write(
        &whitelist_path,
        "# user protected paths\n\n/home/test/.cache/tool-*/**\n/home/test/keep\n",
    )
    .expect("write whitelist");

    let whitelist = Whitelist::load(&xdg(&home, &config)).expect("load whitelist");

    assert!(whitelist.matches(Path::new("/home/test/.cache/tool-a/item")));
    assert!(whitelist.matches(Path::new("/home/test/keep")));
    assert!(!whitelist.matches(Path::new("/home/test/other")));
}

#[test]
fn invalid_whitelist_lines_are_skipped_and_reported_with_line_numbers() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("home");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::write(
        &whitelist_path,
        "# ignored comment\nrelative/cache\n/home/test/../secret\n/home/test/bad\tname\n/home/test/[broken\n/home/test/keep\n",
    )
    .expect("write whitelist");

    let whitelist = Whitelist::load(&xdg(&home, &config)).expect("load whitelist");

    assert_eq!(whitelist.warnings.len(), 4);
    assert_eq!(whitelist.warnings[0].line, 2);
    assert_eq!(whitelist.warnings[0].reason, "relative path");
    assert_eq!(whitelist.warnings[1].line, 3);
    assert_eq!(whitelist.warnings[1].reason, "path traversal");
    assert_eq!(whitelist.warnings[2].line, 4);
    assert_eq!(whitelist.warnings[2].reason, "control character");
    assert_eq!(whitelist.warnings[3].line, 5);
    assert_eq!(whitelist.warnings[3].reason, "invalid glob pattern");
    assert!(whitelist.is_whitelisted(Path::new("/home/test/keep")));
    assert!(!whitelist.is_whitelisted(Path::new("/home/test/nope")));
}

#[test]
fn normalizes_repeated_slashes_and_preserves_literal_pattern_descendants() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("home");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::write(&whitelist_path, "/home/test/cache/keep\n").expect("write whitelist");

    let whitelist = Whitelist::load(&xdg(&home, &config)).expect("load whitelist");
    assert!(whitelist.matches(Path::new("/home/test//cache/keep")));
    assert!(whitelist.matches(Path::new("/home/test/cache/keep/child")));
    assert!(whitelist.matches(Path::new("/home/test/cache")));
}

#[test]
fn saves_unique_patterns_at_the_xdg_path() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("home");
    let xdg = xdg(&home, &config);

    Whitelist::save_patterns(
        &xdg,
        &[
            "/home/test/cache/*".to_owned(),
            "/home/test/cache/*".to_owned(),
        ],
    )
    .expect("save whitelist");

    let contents =
        fs::read_to_string(config.join("mole-linux/whitelist")).expect("read saved file");
    assert_eq!(contents.matches("/home/test/cache/*").count(), 1);
    assert!(contents.starts_with("# Mole Linux Whitelist"));
}

#[cfg(unix)]
#[test]
fn saving_replaces_file_atomically_and_preserves_config_symlink() {
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    let external = temp.path().join("linked-whitelist");
    fs::create_dir_all(&home).expect("home");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::write(&external, "# old\n").expect("initial target");
    std::os::unix::fs::symlink(&external, &whitelist_path).expect("whitelist symlink");
    let original_inode = fs::metadata(&external).expect("target metadata").ino();

    Whitelist::save_patterns(&xdg(&home, &config), &["/home/test/keep".to_owned()])
        .expect("save through symlink");

    assert!(
        fs::symlink_metadata(&whitelist_path)
            .expect("symlink metadata")
            .file_type()
            .is_symlink()
    );
    assert_ne!(
        fs::metadata(&external)
            .expect("updated target metadata")
            .ino(),
        original_inode
    );
    assert!(
        fs::read_to_string(&external)
            .expect("read target")
            .contains("/home/test/keep")
    );
}

#[cfg(unix)]
#[test]
fn glob_patterns_match_non_utf8_path_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("home");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::write(&whitelist_path, "/home/test/*\n").expect("write whitelist");

    let whitelist = Whitelist::load(&xdg(&home, &config)).expect("load whitelist");
    let non_utf8 = Path::new("/home/test").join(OsString::from_vec(vec![0xff]));
    assert!(whitelist.matches(&non_utf8));
}

#[test]
fn whitelist_patterns_protect_target_and_its_whitelisted_children_in_validator() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    let target = home.join("cache/tool");
    fs::create_dir_all(&target).expect("target");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::write(
        &whitelist_path,
        format!("{}\n", home.join("cache/tool/keep").display()),
    )
    .expect("write whitelist");
    let xdg = xdg(&home, &config);

    assert_eq!(
        validate_path_for_deletion(&target, &home, &xdg),
        Err(ValidationError::Protected(target))
    );
}

#[test]
fn unreadable_whitelist_fails_closed_during_validation() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("home");
    let whitelist_path = config.join("mole-linux/whitelist");
    fs::create_dir_all(whitelist_path.parent().expect("parent")).expect("config dir");
    fs::create_dir(&whitelist_path).expect("invalid whitelist file type");
    let target = home.join("cache/item");
    fs::create_dir_all(target.parent().expect("target parent")).expect("target dir");
    fs::write(&target, "data").expect("target file");

    assert!(matches!(
        validate_path_for_deletion(&target, &home, &xdg(&home, &config)),
        Err(ValidationError::Unverifiable(path)) if path == whitelist_path
    ));
}
