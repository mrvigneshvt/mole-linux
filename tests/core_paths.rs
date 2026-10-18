use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::validate::{ValidationError, validate_path_for_deletion};

fn dirs(home: &Path) -> XdgPaths {
    XdgPaths::from_values(home.to_path_buf(), None, None, None, None)
        .expect("valid home produces XDG paths")
}

#[test]
fn xdg_paths_follow_absolute_environment_values_and_fallbacks() {
    let home = PathBuf::from("/home/tester");
    let paths = XdgPaths::from_values(
        home.clone(),
        Some(PathBuf::from("/mnt/config")),
        Some(PathBuf::from("relative/data")),
        None,
        Some(PathBuf::from("/mnt/cache")),
    )
    .expect("valid home produces XDG paths");

    assert_eq!(paths.config, PathBuf::from("/mnt/config"));
    assert_eq!(paths.data, home.join(".local/share"));
    assert_eq!(paths.state, home.join(".local/state"));
    assert_eq!(paths.cache, PathBuf::from("/mnt/cache"));
}

#[test]
fn validation_table_rejects_unsafe_paths_and_accepts_ordinary_children() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".cache/app")).expect("create cache fixture");
    std::fs::create_dir_all(home.join(".cache/name..files")).expect("create dotted fixture");
    let xdg = dirs(&home);
    let cases = [
        ("empty", PathBuf::new(), false),
        ("relative", PathBuf::from("cache/item"), false),
        (
            "parent traversal",
            PathBuf::from("/tmp/../etc/passwd"),
            false,
        ),
        ("newline", PathBuf::from("/tmp/bad\nname"), false),
        ("root", PathBuf::from("/"), false),
        ("usr", PathBuf::from("/usr/bin/tool"), false),
        ("etc", PathBuf::from("/etc/hosts"), false),
        ("boot", PathBuf::from("/boot/vmlinuz"), false),
        ("home root", home.clone(), false),
        ("config root", xdg.config.clone(), false),
        ("data root", xdg.data.clone(), false),
        ("state root", xdg.state.clone(), false),
        ("cache root", xdg.cache.clone(), false),
        ("ssh credentials", home.join(".ssh/id_ed25519"), false),
        (
            "environment secret",
            home.join("project/.env.production"),
            false,
        ),
        (
            "git repository",
            home.join("src/project/.git/config"),
            false,
        ),
        ("ordinary cache child", home.join(".cache/app/item"), true),
        (
            "name containing two dots",
            home.join(".cache/name..files"),
            true,
        ),
    ];

    for (name, path, expected) in cases {
        let result = validate_path_for_deletion(&path, &home, &xdg);
        assert_eq!(result.is_ok(), expected, "case {name}: {result:?}");
    }
}

#[test]
fn every_ascii_control_byte_is_rejected() {
    let home = PathBuf::from("/home/tester");
    let xdg = dirs(&home);
    for byte in 0_u8..=0x1f {
        let mut raw = b"/tmp/control".to_vec();
        raw.push(byte);
        raw.extend_from_slice(b"name");
        let path = { PathBuf::from(std::ffi::OsString::from_vec(raw)) };
        assert!(
            matches!(
                validate_path_for_deletion(&path, &home, &xdg),
                Err(ValidationError::ControlByte)
            ),
            "accepted ASCII control byte {byte}"
        );
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(
        [b"/tmp/control".as_slice(), &[0x7f], b"name"].concat(),
    ));
    assert!(matches!(
        validate_path_for_deletion(&path, &home, &xdg),
        Err(ValidationError::ControlByte)
    ));
}

#[test]
fn validator_rejects_symlink_ancestors_and_symlink_leaf_into_critical_roots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join("cache")).expect("create home cache");
    std::fs::create_dir_all(home.join(".ssh")).expect("create credential fixture");
    let xdg = dirs(&home);
    let repository = home.join("src/repo");
    std::fs::create_dir_all(repository.join(".git")).expect("create git marker");
    std::fs::create_dir_all(repository.join("src")).expect("create source directory");
    let repo_file = repository.join("src/main.rs");
    std::fs::write(&repo_file, "test fixture").expect("create repository file");
    let repo_result = validate_path_for_deletion(&repo_file, &home, &xdg);
    assert!(
        repo_result.is_ok(),
        "ordinary repository child: {repo_result:?}"
    );
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).expect("create outside target");
    let redirect = home.join("redirect");
    std::os::unix::fs::symlink(&outside, &redirect).expect("create ancestor symlink");
    let protected_redirect = home.join("redirect-credentials");
    std::os::unix::fs::symlink(home.join(".ssh"), &protected_redirect)
        .expect("create protected ancestor symlink");
    let leaf = home.join("cache/system-link");
    std::os::unix::fs::symlink("/usr", &leaf).expect("create leaf symlink");
    let dangling = home.join("cache/dangling");
    std::os::unix::fs::symlink(temp.path().join("missing-target"), &dangling)
        .expect("create dangling symlink");
    let safe_target = home.join("cache/target");
    std::fs::write(&safe_target, "keep target").expect("create symlink target");
    let safe_link = home.join("cache/link");
    std::os::unix::fs::symlink(&safe_target, &safe_link).expect("create safe leaf symlink");

    assert!(matches!(
        validate_path_for_deletion(&redirect.join("passwd"), &home, &xdg),
        Err(ValidationError::OutsideHome)
    ));
    assert!(matches!(
        validate_path_for_deletion(&protected_redirect.join("key"), &home, &xdg),
        Err(ValidationError::SymlinkAncestor { .. })
    ));
    assert!(matches!(
        validate_path_for_deletion(&leaf, &home, &xdg),
        Err(ValidationError::SymlinkTargetProtected { .. })
    ));
    assert!(matches!(
        validate_path_for_deletion(&dangling, &home, &xdg),
        Err(ValidationError::Unverifiable(_))
    ));
    assert!(validate_path_for_deletion(&safe_link, &home, &xdg).is_ok());
    assert!(safe_target.exists());
    assert!(validate_path_for_deletion(&home.join("cache/item"), &home, &xdg).is_ok());
}

#[test]
fn git_policy_protects_metadata_and_repository_roots_but_allows_generated_children() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let xdg = dirs(&home);
    let repo = home.join("project");
    std::fs::create_dir_all(repo.join(".git")).expect("create git metadata");
    std::fs::create_dir_all(repo.join("node_modules/pkg")).expect("create node modules");
    std::fs::create_dir_all(repo.join("target/debug")).expect("create build output");
    std::fs::create_dir_all(repo.join("__pycache__")).expect("create python cache");
    let worktree = home.join("linked-worktree");
    std::fs::create_dir_all(&worktree).expect("create worktree");
    std::fs::write(worktree.join(".git"), "gitdir: /tmp/gitdir").expect("create worktree marker");

    let cases = [
        ("git metadata", repo.join(".git"), false),
        ("git metadata child", repo.join(".git/objects/pack"), false),
        ("repository root", repo.clone(), false),
        ("linked worktree root", worktree, false),
        ("node modules", repo.join("node_modules/pkg/file.js"), true),
        ("build output", repo.join("target/debug/app"), true),
        ("python cache", repo.join("__pycache__/module.pyc"), true),
    ];
    for (name, path, expected) in cases {
        let result = validate_path_for_deletion(&path, &home, &xdg);
        assert_eq!(result.is_ok(), expected, "case {name}: {result:?}");
    }
}

#[test]
fn protected_user_data_is_path_scoped_and_honors_xdg_roots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let xdg = XdgPaths::from_values(
        home.clone(),
        Some(home.join("custom-config")),
        Some(home.join("custom-data")),
        Some(home.join("custom-state")),
        Some(home.join("custom-cache")),
    )
    .expect("valid XDG overrides");
    std::fs::create_dir_all(xdg.cache.join("chromium/cache"))
        .expect("create Chromium cache fixture");
    std::fs::create_dir_all(xdg.cache.join("mozilla/cache")).expect("create Mozilla cache fixture");
    std::fs::create_dir_all(home.join("Downloads")).expect("create downloads fixture");

    let cases = [
        ("mozilla root", home.join(".mozilla"), false),
        (
            "mozilla profile",
            home.join(".mozilla/firefox/profile/cookies.sqlite"),
            false,
        ),
        ("chromium profile root", xdg.config.join("chromium"), false),
        (
            "chromium profile",
            xdg.config.join("chromium/Default/Cookies"),
            false,
        ),
        (
            "chrome profile root",
            xdg.config.join("google-chrome"),
            false,
        ),
        (
            "chrome profile",
            xdg.config.join("google-chrome/Default/Cookies"),
            false,
        ),
        (
            "brave profile root",
            xdg.config.join("BraveSoftware"),
            false,
        ),
        (
            "brave profile",
            xdg.config
                .join("BraveSoftware/Brave-Browser/Default/Cookies"),
            false,
        ),
        (
            "edge profile root",
            xdg.config.join("microsoft-edge"),
            false,
        ),
        (
            "edge profile",
            xdg.config.join("microsoft-edge/Default/Cookies"),
            false,
        ),
        ("vivaldi profile root", xdg.config.join("vivaldi"), false),
        (
            "vivaldi profile",
            xdg.config.join("vivaldi/Default/Cookies"),
            false,
        ),
        (
            "cache named chromium",
            xdg.cache.join("chromium/cache/item"),
            true,
        ),
        (
            "cache named mozilla",
            xdg.cache.join("mozilla/cache/item"),
            true,
        ),
        ("documents root", home.join("Documents"), false),
        ("documents child", home.join("Documents/report.txt"), false),
        ("pictures root", home.join("Pictures"), false),
        ("pictures child", home.join("Pictures/photo.jpg"), false),
        ("videos root", home.join("Videos"), false),
        ("videos child", home.join("Videos/movie.mkv"), false),
        ("music root", home.join("Music"), false),
        ("music child", home.join("Music/song.ogg"), false),
        ("desktop root", home.join("Desktop"), false),
        ("desktop child", home.join("Desktop/shortcut"), false),
        ("downloads root", home.join("Downloads"), false),
        (
            "downloads child",
            home.join("Downloads/installer.AppImage"),
            true,
        ),
        ("ssh root", home.join(".ssh"), false),
        ("ssh child", home.join(".ssh/id_ed25519"), false),
        ("gnupg root", home.join(".gnupg"), false),
        (
            "gnupg child",
            home.join(".gnupg/private-keys-v1.d/key"),
            false,
        ),
        ("keyrings root", xdg.data.join("keyrings"), false),
        (
            "keyring child",
            xdg.data.join("keyrings/login.keyring"),
            false,
        ),
        (
            "models anywhere",
            home.join("scratch/models/model.gguf"),
            false,
        ),
        (
            "volumes anywhere",
            home.join("scratch/volumes/container/data"),
            false,
        ),
    ];
    for (name, path, expected) in cases {
        let result = validate_path_for_deletion(&path, &home, &xdg);
        assert_eq!(result.is_ok(), expected, "case {name}: {result:?}");
    }
}

#[test]
fn every_system_root_and_descendant_is_rejected() {
    let home = PathBuf::from("/home/tester");
    let xdg = dirs(&home);
    let roots = [
        "/", "/bin", "/sbin", "/lib", "/lib64", "/usr", "/etc", "/boot", "/var", "/opt", "/srv",
        "/root", "/home", "/proc", "/sys", "/dev", "/run", "/mnt", "/media",
    ];
    for root in roots {
        let paths = if root == "/" || root == "/home" {
            vec![PathBuf::from(root)]
        } else {
            vec![
                PathBuf::from(root),
                PathBuf::from(root).join("mole-test-child"),
            ]
        };
        for path in paths {
            let result = validate_path_for_deletion(&path, &home, &xdg);
            assert!(
                result.is_err(),
                "accepted critical path {path:?}: {result:?}"
            );
        }
    }
}

#[test]
fn deletion_candidates_must_be_strict_home_descendants() {
    let home = PathBuf::from("/home/currentuser");
    let xdg = dirs(&home);
    let cases = [
        ("other user", PathBuf::from("/home/otheruser/x")),
        ("tmp", PathBuf::from("/tmp/x")),
        ("var tmp", PathBuf::from("/var/tmp/x")),
        ("home itself", home.clone()),
        ("escape traversal", home.join("../other")),
    ];
    for (name, path) in cases {
        let result = validate_path_for_deletion(&path, &home, &xdg);
        assert_eq!(
            result,
            Err(ValidationError::OutsideHome),
            "case {name}: {result:?}"
        );
    }
}
