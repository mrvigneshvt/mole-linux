use std::fs;
use tempfile::tempdir;

use mole_linux::purge::{
    find_project_root_for_artifact, is_cloud_synced_path, is_safe_configured_artifact,
    is_safe_project_artifact,
};

#[test]
fn artifact_safety_enforces_project_depth_and_absolute_paths() {
    let home = tempdir().expect("home");
    let root = home.path().join("www");
    let project = root.join("project");
    fs::create_dir_all(project.join("sub/node_modules")).expect("nested artifact");
    fs::create_dir_all(root.join("node_modules")).expect("shallow artifact");
    fs::write(project.join("package.json"), "{}").expect("project marker");
    assert!(!is_safe_project_artifact(
        &root.join("node_modules"),
        &root,
        home.path()
    ));
    assert!(!is_safe_project_artifact(
        std::path::Path::new("relative/node_modules"),
        &root,
        home.path()
    ));
    assert!(is_safe_project_artifact(
        &project.join("sub/node_modules"),
        &root,
        home.path()
    ));

    let single = home.path().join("single");
    fs::create_dir_all(single.join("node_modules")).expect("single project artifact");
    fs::write(single.join("Cargo.toml"), "").expect("project marker");
    assert!(is_safe_project_artifact(
        &single.join("node_modules"),
        &single,
        home.path()
    ));
}

#[test]
fn artifact_safety_accepts_physical_paths_under_symlinked_roots() {
    let home = tempdir().expect("home");
    let real = home.path().join("www/real/proj");
    fs::create_dir_all(real.join("node_modules")).expect("artifact");
    fs::write(real.join("package.json"), "{}").expect("marker");
    let alias = home.path().join("www/link");
    std::os::unix::fs::symlink(home.path().join("www/real"), &alias).expect("symlink");
    assert!(is_safe_project_artifact(
        &real.join("node_modules"),
        &alias.join("proj"),
        home.path()
    ));
}

#[test]
fn configured_artifact_safety_checks_roots_and_supports_physical_alias_fallback() {
    let home = tempdir().expect("home");
    let real = home.path().join("www/real/proj");
    fs::create_dir_all(real.join("node_modules")).expect("artifact");
    fs::write(real.join("package.json"), "{}").expect("marker");
    let alias = home.path().join("www/link");
    std::os::unix::fs::symlink(home.path().join("www/real"), &alias).expect("symlink");
    let roots = vec![home.path().join("unrelated"), alias.join("proj")];
    assert!(is_safe_configured_artifact(
        &real.join("node_modules"),
        &roots,
        home.path()
    ));
    assert!(!is_safe_configured_artifact(
        &home.path().join("dev/other/node_modules"),
        &[home.path().join("www")],
        home.path()
    ));
}

#[test]
fn configured_roots_outside_home_remain_refused() {
    let home = tempdir().expect("home");
    let outside = tempdir().expect("outside root");
    let project = outside.path().join("project");
    fs::create_dir_all(project.join("target")).expect("artifact");
    fs::write(project.join("Cargo.toml"), "").expect("project marker");
    assert!(!is_safe_project_artifact(
        &project.join("target"),
        &project,
        home.path()
    ));
    assert!(!is_safe_configured_artifact(
        &project.join("target"),
        &[project],
        home.path()
    ));
}

#[test]
fn cloud_roots_and_fuse_mounts_match_only_exact_roots_and_descendants() {
    let home = tempdir().expect("home");
    for name in [
        "Dropbox",
        "OneDrive",
        "Nextcloud",
        "pCloudDrive",
        "MEGA",
        "Insync",
    ] {
        let root = home.path().join(name);
        assert!(is_cloud_synced_path(&root, home.path(), ""));
        assert!(is_cloud_synced_path(
            &root.join("project/target"),
            home.path(),
            ""
        ));
        assert!(!is_cloud_synced_path(
            &home.path().join(format!("{name}Backup")),
            home.path(),
            ""
        ));
    }
    let mountinfo = "42 1 0:1 / /mnt/cloud rw - fuse.rclone rclone rw\n";
    assert!(is_cloud_synced_path(
        std::path::Path::new("/mnt/cloud/project"),
        home.path(),
        mountinfo
    ));
    assert!(!is_cloud_synced_path(
        std::path::Path::new("/mnt/cloud-old/project"),
        home.path(),
        mountinfo
    ));
    assert!(!is_cloud_synced_path(
        &home.path().join("ordinary/project"),
        home.path(),
        ""
    ));
}

#[test]
fn cloud_roots_follow_a_symlinked_home_to_its_physical_location() {
    let physical = tempdir().expect("physical home");
    let logical = physical.path().join("home-link");
    std::os::unix::fs::symlink(physical.path(), &logical).expect("home alias");
    assert!(is_cloud_synced_path(
        &physical.path().join("Dropbox/project/target"),
        &logical,
        ""
    ));
    assert!(is_cloud_synced_path(
        &logical.join("Dropbox/project/target"),
        &logical,
        ""
    ));
}

#[test]
fn artifact_owner_prefers_monorepo_and_worktree_markers_over_nested_packages() {
    let home = tempdir().expect("home");
    let monorepo = home.path().join("www/obelisk");
    let artifact = monorepo.join("apps/web/node_modules");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(monorepo.join("pnpm-workspace.yaml"), "").expect("workspace marker");
    fs::write(monorepo.join("apps/web/package.json"), "{}").expect("nested package");
    assert_eq!(
        find_project_root_for_artifact(&artifact, home.path()),
        Some(monorepo)
    );

    let worktree = home.path().join(".codex/worktrees/obelisk");
    fs::create_dir_all(worktree.join("apps/web/node_modules")).expect("worktree artifact");
    fs::write(worktree.join(".git"), "gitdir: /tmp/gitdir\n").expect("worktree marker");
    fs::write(worktree.join("apps/web/package.json"), "{}").expect("nested package");
    assert_eq!(
        find_project_root_for_artifact(&worktree.join("apps/web/node_modules"), home.path()),
        Some(worktree)
    );
}
