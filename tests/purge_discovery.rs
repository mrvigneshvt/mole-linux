use std::fs;
use tempfile::tempdir;

use mole_linux::purge::{
    discover_project_dirs, filter_nested_artifacts, scan_artifacts, scan_artifacts_with_context,
};

#[test]
fn project_container_discovery_includes_agent_worktrees_but_skips_unlisted_hidden_dirs() {
    let home = tempdir().expect("home");
    fs::create_dir_all(home.path().join("CustomProjects/app")).expect("projects");
    fs::write(home.path().join("CustomProjects/app/go.mod"), "").expect("go marker");
    for container in [
        ".codex/worktrees",
        ".claude/worktrees",
        ".superset/worktrees",
    ] {
        fs::create_dir_all(home.path().join(container).join("checkout/node_modules"))
            .expect("worktree container");
    }
    fs::create_dir_all(home.path().join(".local/share/app")).expect("hidden directory");
    fs::write(home.path().join(".local/share/app/package.json"), "{}").expect("hidden marker");
    let roots = discover_project_dirs(home.path()).expect("discover");
    assert!(roots.contains(&home.path().join("CustomProjects")));
    for container in [
        ".codex/worktrees",
        ".claude/worktrees",
        ".superset/worktrees",
    ] {
        assert!(roots.contains(&home.path().join(container)));
    }
    assert!(
        !roots
            .iter()
            .any(|path| path.starts_with(home.path().join(".local")))
    );
}

#[test]
fn agent_worktree_containers_do_not_authorize_direct_child_artifacts() {
    let home = tempdir().expect("home");
    let container = home.path().join(".codex/worktrees");
    fs::create_dir_all(container.join("node_modules")).expect("direct artifact");
    fs::create_dir_all(container.join("checkout/node_modules")).expect("nested artifact");
    fs::write(container.join("checkout/package.json"), "{}").expect("project marker");
    let results = scan_artifacts(&container, 5).expect("scan");
    assert!(
        !results
            .iter()
            .any(|candidate| candidate.path == container.join("node_modules"))
    );
    assert!(
        results
            .iter()
            .any(|candidate| candidate.path == container.join("checkout/node_modules"))
    );
}

#[test]
fn nested_artifact_filter_drops_descendants_and_keeps_independent_candidates() {
    let home = tempdir().expect("home");
    let parent = home.path().join("project/node_modules");
    let nested = parent.join("pkg/node_modules");
    let independent = home.path().join("other/target");
    assert_eq!(
        filter_nested_artifacts(&[nested, independent.clone(), parent.clone()]),
        vec![independent, parent]
    );
}

#[test]
fn valid_regular_cachedir_tags_add_candidates_but_invalid_or_symlink_tags_do_not() {
    let home = tempdir().expect("home");
    let project = home.path().join("project");
    fs::create_dir_all(project.join("cache/valid")).expect("cache");
    fs::create_dir_all(project.join("cache/invalid")).expect("cache");
    fs::create_dir_all(project.join("cache/symlinked")).expect("cache");
    fs::write(project.join("Cargo.toml"), "").expect("project marker");
    fs::write(
        project.join("cache/valid/CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    )
    .expect("valid tag");
    fs::write(
        project.join("cache/invalid/CACHEDIR.TAG"),
        "Signature: incorrect\n",
    )
    .expect("invalid tag");
    fs::write(
        project.join("cache/target.tag"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    )
    .expect("tag target");
    std::os::unix::fs::symlink(
        project.join("cache/target.tag"),
        project.join("cache/symlinked/CACHEDIR.TAG"),
    )
    .expect("tag symlink");
    let candidates = scan_artifacts(&project, 6).expect("scan");
    assert!(
        candidates
            .iter()
            .any(|item| item.path == project.join("cache/valid"))
    );
    assert!(
        !candidates
            .iter()
            .any(|item| item.path == project.join("cache/invalid"))
    );
    assert!(
        !candidates
            .iter()
            .any(|item| item.path == project.join("cache/symlinked"))
    );
}

#[test]
fn scans_skip_linux_cloud_sync_roots_and_fuse_mounts() {
    let home = tempdir().expect("home");
    let cloud_project = home.path().join("Dropbox/project");
    fs::create_dir_all(cloud_project.join("node_modules")).expect("cloud artifact");
    fs::write(cloud_project.join("package.json"), "{}").expect("project marker");
    assert!(
        scan_artifacts_with_context(
            &home.path().join("Dropbox"),
            6,
            std::time::Duration::from_secs(2),
            home.path(),
            ""
        )
        .expect("scan cloud root")
        .is_empty()
    );

    let mounted = home.path().join("mounted/project");
    fs::create_dir_all(mounted.join("node_modules")).expect("mount artifact");
    fs::write(mounted.join("package.json"), "{}").expect("project marker");
    let mountinfo = format!(
        "42 1 0:1 / {} rw - fuse.rclone remote rw\n",
        home.path().join("mounted").display()
    );
    assert!(
        scan_artifacts_with_context(
            &home.path().join("mounted"),
            6,
            std::time::Duration::from_secs(2),
            home.path(),
            &mountinfo
        )
        .expect("scan mount")
        .is_empty()
    );
}

#[test]
fn a_failed_scan_stage_does_not_publish_a_partial_root() {
    let home = tempdir().expect("home");
    let project = home.path().join("project");
    fs::create_dir_all(project.join("node_modules")).expect("artifact");
    fs::write(project.join("package.json"), "{}").expect("marker");
    assert!(scan_artifacts(&project, 6).is_ok());
    assert!(
        mole_linux::purge::scan_artifacts_with_budget(&project, 6, std::time::Duration::ZERO)
            .is_err()
    );
}

#[test]
fn an_unavailable_scan_root_is_an_error_not_an_empty_inventory() {
    let home = tempdir().expect("home");
    assert!(scan_artifacts(&home.path().join("missing"), 6).is_err());
}
