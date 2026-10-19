use std::fs;
use std::os::unix::fs::symlink;
use std::process::Command;
#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

const SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

#[test]
fn artifact_directory_names_are_cleanable_hints() {
    for name in [
        "node_modules",
        "bower_components",
        ".yarn",
        ".pnpm-store",
        "venv",
        ".venv",
        "virtualenv",
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        ".tox",
        ".eggs",
        "htmlcov",
        ".ipynb_checkpoints",
        "vendor",
        ".bundle",
        ".gradle",
        "out",
        "build",
        "dist",
        "target",
        ".next",
        ".nuxt",
        ".output",
        ".parcel-cache",
        ".turbo",
        ".vite",
        ".nx",
        "coverage",
        ".coverage",
        ".nyc_output",
        ".angular",
        ".svelte-kit",
        ".astro",
        ".docusaurus",
        "DerivedData",
        "Pods",
        ".build",
        "Carthage",
        ".dart_tool",
        ".terraform",
    ] {
        assert!(
            mole_linux::cleanable::is_cleanable_dir(std::path::Path::new(name)),
            "{name}"
        );
    }
    assert!(!mole_linux::cleanable::is_cleanable_dir(
        std::path::Path::new("src")
    ));
    assert!(!mole_linux::cleanable::is_cleanable_dir(
        std::path::Path::new(".git")
    ));
    assert!(!mole_linux::cleanable::is_cleanable_dir(
        std::path::Path::new("")
    ));
}

#[test]
fn cachedir_tag_requires_valid_signature_in_a_regular_file() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path();
    let tag = root.join("CACHEDIR.TAG");

    fs::write(&tag, format!("{SIGNATURE}\n# cache marker\n")).expect("write valid tag");
    assert!(mole_linux::cleanable::is_cleanable_dir(root));

    fs::write(&tag, "Signature: invalid").expect("write invalid tag");
    assert!(!mole_linux::cleanable::is_cleanable_dir(root));

    fs::write(&tag, &SIGNATURE[..SIGNATURE.len() - 1]).expect("write short tag");
    assert!(!mole_linux::cleanable::is_cleanable_dir(root));

    fs::remove_file(&tag).expect("remove tag");
    let real_tag = root.join("real-tag");
    fs::write(&real_tag, SIGNATURE).expect("write target tag");
    symlink(&real_tag, &tag).expect("create symlink tag");
    assert!(!mole_linux::cleanable::is_cleanable_dir(root));
}

#[test]
fn analyze_json_marks_directory_hints_without_marking_files() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("scan-root");
    fs::create_dir_all(root.join("node_modules")).expect("create artifact directory");
    fs::create_dir(root.join("ordinary")).expect("create ordinary directory");
    fs::create_dir(root.join("tagged")).expect("create tagged directory");
    fs::write(
        root.join("tagged/CACHEDIR.TAG"),
        format!("{SIGNATURE}\n# cache marker\n"),
    )
    .expect("write CACHEDIR.TAG");
    fs::write(root.join("file"), b"payload").expect("create file");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["analyze", "--json"])
        .arg(&root)
        .isolated_home(temp.path())
        .output()
        .expect("run analyze --json");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let entries = document["entries"].as_array().expect("entries array");
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry["name"] == "node_modules")
            .unwrap()["cleanable"],
        true
    );
    assert!(
        entries
            .iter()
            .find(|entry| entry["name"] == "ordinary")
            .unwrap()
            .get("cleanable")
            .is_none()
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry["name"] == "tagged")
            .unwrap()["cleanable"],
        true
    );
    assert!(
        entries
            .iter()
            .find(|entry| entry["name"] == "file")
            .unwrap()
            .get("cleanable")
            .is_none()
    );
}
