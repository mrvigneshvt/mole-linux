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

#[test]
fn large_git_probe_inventory_handles_many_linked_worktrees_and_tracked_artifacts() {
    use std::time::Duration;

    let home = tempdir().expect("home");
    let workspace = home.path().join("Workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let repo_count = 40;
    for index in 0..repo_count {
        let main = workspace.join(format!("repo-{index}"));
        fs::create_dir_all(main.join("target")).expect("target");
        run_git(&main, &["init", "-q"]);
        run_git(&main, &["config", "user.email", "test@example.com"]);
        run_git(&main, &["config", "user.name", "Test"]);
        let tracked_count = if index == 0 { 6_000 } else { 250 };
        for file in 0..tracked_count {
            fs::write(
                main.join("target").join(format!("tracked-{file:05}.rs")),
                "x",
            )
            .expect("tracked file");
        }
        run_git(&main, &["add", "target"]);
        run_git(&main, &["commit", "-qm", "tracked target tree"]);
        let worktree = workspace.join(format!("worktree-{index}"));
        run_git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "linked",
                worktree.to_str().expect("utf8 path"),
            ],
        );
    }

    let candidates =
        scan_artifacts_with_context(&workspace, 4, Duration::from_secs(30), home.path(), "")
            .expect("large linked-worktree inventory");
    assert_eq!(candidates.len(), repo_count * 2);
    assert!(candidates.iter().all(|candidate| candidate.protected));
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    use std::process::Command;
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
