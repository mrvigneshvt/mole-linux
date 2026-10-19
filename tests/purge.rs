use std::fs;
use std::path::Path;
use tempfile::tempdir;

use mole_linux::purge::{
    DEFAULT_SEARCH_DIRS, TARGETS, is_dotnet_bin_dir, is_project_root, is_protected_artifact,
    load_search_paths, scan_artifacts,
};

#[test]
fn purge_targets_include_upstream_build_artifacts() {
    for target in [
        "node_modules",
        "target",
        "vendor",
        "bin",
        ".terragrunt-cache",
    ] {
        assert!(TARGETS.contains(&target));
    }
}

#[test]
fn config_paths_expand_home_ignore_comments_and_fall_back_when_empty() {
    let home = tempdir().expect("temporary home");
    let config = home.path().join("purge_paths");
    fs::write(&config, " # note\n\n~/Projects\n  ~/Code  \n").expect("write config");
    let paths = load_search_paths(home.path(), &config).expect("load configured paths");
    assert_eq!(
        paths,
        vec![home.path().join("Projects"), home.path().join("Code")]
    );

    fs::write(&config, "# only a comment\n\n").expect("empty config");
    let paths = load_search_paths(home.path(), &config).expect("load defaults");
    assert_eq!(
        paths,
        DEFAULT_SEARCH_DIRS
            .iter()
            .map(|dir| home.path().join(dir))
            .collect::<Vec<_>>()
    );
}

#[test]
fn project_detection_and_vendor_protection_follow_owner_evidence() {
    let home = tempdir().expect("temporary home");
    let rails = home.path().join("rails");
    fs::create_dir_all(rails.join("config")).expect("rails dirs");
    fs::create_dir_all(rails.join("bin")).expect("rails bin");
    fs::write(rails.join("Gemfile"), "").expect("Gemfile");
    fs::write(rails.join("config/application.rb"), "").expect("application");
    fs::write(rails.join("bin/rails"), "").expect("rails executable");
    assert!(is_project_root(&rails));
    let rails_vendor = rails.join("vendor");
    fs::create_dir(&rails_vendor).expect("vendor");
    assert!(is_protected_artifact(&rails_vendor));

    let php = home.path().join("php");
    fs::create_dir_all(php.join("vendor")).expect("php dirs");
    fs::write(php.join("composer.json"), "{}").expect("composer manifest");
    assert!(!is_protected_artifact(&php.join("vendor")));

    let go = home.path().join("go");
    fs::create_dir_all(go.join("vendor")).expect("go dirs");
    fs::write(go.join("go.mod"), "module example\n").expect("go module");
    assert!(is_protected_artifact(&go.join("vendor")));

    let dotnet = home.path().join("dotnet");
    fs::create_dir_all(dotnet.join("bin/Debug")).expect("dotnet output");
    fs::write(dotnet.join("App.csproj"), "").expect("dotnet project");
    assert!(is_dotnet_bin_dir(&dotnet.join("bin")).expect("dotnet probe"));
    assert!(!is_protected_artifact(&dotnet.join("bin")));
}

#[test]
fn scan_filters_nested_artifacts_and_protects_authored_content() {
    let home = tempdir().expect("temporary home");
    let project = home.path().join("project");
    fs::create_dir_all(project.join("node_modules/pkg/node_modules")).expect("nested modules");
    fs::write(project.join("package.json"), "{}").expect("manifest");
    fs::create_dir_all(project.join("target/deploy")).expect("deploy output");
    fs::write(project.join("target/deploy/id-keypair.json"), "{}")
        .expect("authored keypair marker");
    let found = scan_artifacts(&project, 6).expect("scan project");
    assert!(
        found
            .iter()
            .any(|item| item.path == project.join("node_modules"))
    );
    assert!(
        !found
            .iter()
            .any(|item| item.path.ends_with("pkg/node_modules"))
    );
    assert!(
        found
            .iter()
            .any(|item| item.path == project.join("target") && item.protected)
    );
}

#[test]
fn scan_rejects_non_project_direct_child_artifacts() {
    let temp = tempdir().expect("temporary root");
    fs::create_dir_all(temp.path().join("node_modules")).expect("stray artifact");
    assert!(!is_project_root(Path::new(temp.path())));
    assert!(scan_artifacts(temp.path(), 2).expect("scan").is_empty());
}

#[test]
fn tracked_files_inside_an_artifact_protect_it() {
    use std::process::Command;
    let home = tempdir().expect("temporary home");
    let project = home.path().join("project");
    let artifact = project.join("build");
    fs::create_dir_all(&artifact).expect("artifact");
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("manifest");
    fs::write(artifact.join("authored.rs"), "fn authored() {}\n").expect("source");
    let init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&project)
        .status()
        .expect("git init");
    assert!(init.success());
    let add = Command::new("git")
        .args(["add", "build/authored.rs"])
        .current_dir(&project)
        .status()
        .expect("git add");
    assert!(add.success());
    assert!(mole_linux::purge::has_authored_content(&artifact).expect("authored probe"));
}

#[test]
fn a_purge_target_is_never_mistaken_for_a_project_container() {
    let home = tempdir().expect("temporary home");
    let stray = home.path().join("node_modules");
    fs::create_dir_all(stray.join("pkg/dist")).expect("stray package tree");
    fs::write(stray.join("pkg/package.json"), "{}").expect("package manifest");
    assert!(scan_artifacts(&stray, 4).expect("scan").is_empty());
}
