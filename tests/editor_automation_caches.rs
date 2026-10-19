// M5.4 contract: editor CachedData and obsolete extensions. Discovery only: the runner
// owns deletion through the sink.
// Ported from tw93/Mole lib/clean/app_caches.sh:clean_editor_obsolete_extensions, Linux roots only.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Candidate, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode, RuleContext,
    catalog,
};

struct Root {
    _temp: tempfile::TempDir,
    home: PathBuf,
    xdg: XdgPaths,
    whitelist: Whitelist,
}

fn root() -> Root {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("create HOME");
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    Root {
        _temp: temp,
        home,
        xdg,
        whitelist,
    }
}

fn seed(path: &Path) {
    fs::create_dir_all(path).expect("create dir");
    fs::write(path.join("payload"), vec![b'x'; 4096]).expect("seed payload");
}

fn discover_with(
    root: &Root,
    rule: &dyn CleanRule,
    probe: &dyn Fn(&str) -> ProcessState,
) -> Discovery {
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &root.home,
        xdg: &root.xdg,
        whitelist: &root.whitelist,
        proc_probe: probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    rule.discover(&ctx)
}

fn catalog_rule(id: &str) -> Box<dyn CleanRule> {
    catalog()
        .into_iter()
        .find(|rule| rule.id() == id)
        .unwrap_or_else(|| panic!("rule {id} is registered"))
}

fn relative(root: &Root, candidate: &Candidate) -> PathBuf {
    candidate
        .path
        .strip_prefix(&root.home)
        .expect("inside HOME")
        .to_path_buf()
}

fn not_running(_: &str) -> ProcessState {
    ProcessState::NotRunning
}

#[test]
fn cached_data_children_of_each_editor_are_offered_as_regenerable() {
    let root = root();
    seed(&root.home.join(".config/Code/CachedData/abc123"));
    seed(&root.home.join(".config/Code - Insiders/CachedData/def456"));
    seed(&root.home.join(".config/Cursor/CachedData/ghi789"));
    let rule = catalog_rule("editor-cached-data");

    let discovery = discover_with(&root, rule.as_ref(), &not_running);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    let mut paths: Vec<PathBuf> = discovery
        .candidates
        .iter()
        .map(|candidate| relative(&root, candidate))
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            PathBuf::from(".config/Code/CachedData/abc123"),
            PathBuf::from(".config/Code - Insiders/CachedData/def456"),
            PathBuf::from(".config/Cursor/CachedData/ghi789"),
        ]
    );
    for candidate in &discovery.candidates {
        assert_eq!(candidate.recovery, RecoveryMode::Regenerate);
        assert!(candidate.selected_by_default);
        assert!(matches!(candidate.owner_check, OwnerCheck::ProcessName(_)));
    }
}

#[test]
fn cached_data_is_absent_when_no_editor_has_one() {
    let root = root();
    let rule = catalog_rule("editor-cached-data");

    let discovery = discover_with(&root, rule.as_ref(), &not_running);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert!(discovery.candidates.is_empty());
}

#[test]
fn running_or_unknown_editor_keeps_its_cached_data_unselected() {
    let root = root();
    seed(&root.home.join(".config/Code/CachedData/abc123"));
    seed(&root.home.join(".config/Cursor/CachedData/ghi789"));
    let rule = catalog_rule("editor-cached-data");
    let probe = |name: &str| match name {
        "cursor" => ProcessState::Running,
        "code" => ProcessState::Unknown,
        _ => ProcessState::NotRunning,
    };

    let discovery = discover_with(&root, rule.as_ref(), &probe);

    assert_eq!(discovery.candidates.len(), 2);
    for candidate in &discovery.candidates {
        assert!(
            !candidate.selected_by_default,
            "{} must not be selected while its editor is running or unknown",
            candidate.path.display()
        );
    }
}

fn write_obsolete(extensions: &Path, body: &str) {
    fs::create_dir_all(extensions).expect("extensions root");
    fs::write(extensions.join(".obsolete"), body).expect("write .obsolete");
}

#[test]
fn obsolete_extension_is_offered_only_when_a_newer_version_is_installed() {
    let root = root();
    let extensions = root.home.join(".vscode/extensions");
    write_obsolete(
        &extensions,
        r#"{"ms-python.python-2024.1.0-linux-x64": true, "esbenp.prettier-vscode-9.0.0": true}"#,
    );
    seed(&extensions.join("ms-python.python-2024.1.0-linux-x64"));
    seed(&extensions.join("ms-python.python-2025.2.0-linux-x64"));
    seed(&extensions.join("esbenp.prettier-vscode-9.0.0"));
    let rule = catalog_rule("editor-obsolete-extensions");

    let discovery = discover_with(&root, rule.as_ref(), &not_running);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    let paths: Vec<PathBuf> = discovery
        .candidates
        .iter()
        .map(|candidate| relative(&root, candidate))
        .collect();
    assert_eq!(
        paths,
        vec![PathBuf::from(
            ".vscode/extensions/ms-python.python-2024.1.0-linux-x64"
        )]
    );
    assert_eq!(discovery.candidates[0].recovery, RecoveryMode::Trash);
}

#[test]
fn obsolete_keys_that_escape_the_extensions_root_or_are_symlinks_are_ignored() {
    let root = root();
    let extensions = root.home.join(".cursor/extensions");
    let outside = root.home.join("outside-1.0");
    seed(&outside);
    write_obsolete(
        &extensions,
        r#"{"../outside-1.0": true, "/abs-1.0": true, "": true, ".": true, "link-1.0": true}"#,
    );
    fs::create_dir_all(&extensions).expect("extensions root");
    symlink(&outside, extensions.join("link-1.0")).expect("symlink obsolete key");
    seed(&extensions.join("link-2.0"));
    let rule = catalog_rule("editor-obsolete-extensions");

    let discovery = discover_with(&root, rule.as_ref(), &not_running);

    assert!(
        discovery.candidates.is_empty(),
        "no escaping key or symlink may be offered: {:?}",
        discovery.candidates
    );
    assert!(outside.join("payload").exists());
}

#[test]
fn obsolete_key_without_a_directory_is_skipped_quietly() {
    let root = root();
    write_obsolete(
        &root.home.join(".vscode/extensions"),
        r#"{"gone.ext-1.0.0-linux-x64": true}"#,
    );
    let rule = catalog_rule("editor-obsolete-extensions");

    let discovery = discover_with(&root, rule.as_ref(), &not_running);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    assert!(discovery.candidates.is_empty());
}

#[test]
fn malformed_or_oversized_obsolete_file_reports_partial_and_offers_nothing() {
    let root = root();
    let extensions = root.home.join(".vscode/extensions");
    write_obsolete(&extensions, "{not json");
    seed(&extensions.join("a-1.0"));
    seed(&extensions.join("a-2.0"));
    let rule = catalog_rule("editor-obsolete-extensions");

    let discovery = discover_with(&root, rule.as_ref(), &not_running);
    assert!(discovery.candidates.is_empty());
    assert!(matches!(discovery.status, DiscoveryStatus::Partial(_)));

    write_obsolete(&extensions, &" ".repeat(2 * 1024 * 1024));
    let discovery = discover_with(&root, rule.as_ref(), &not_running);
    assert!(discovery.candidates.is_empty());
    assert!(matches!(discovery.status, DiscoveryStatus::Partial(_)));
}

#[test]
fn running_editor_keeps_obsolete_extension_unselected() {
    let root = root();
    let extensions = root.home.join(".vscode/extensions");
    write_obsolete(&extensions, r#"{"a-1.0.0": true}"#);
    seed(&extensions.join("a-1.0.0"));
    seed(&extensions.join("a-2.0.0"));
    let rule = catalog_rule("editor-obsolete-extensions");
    let probe = |name: &str| match name {
        "code" => ProcessState::Running,
        _ => ProcessState::NotRunning,
    };

    let discovery = discover_with(&root, rule.as_ref(), &probe);

    assert_eq!(discovery.candidates.len(), 1);
    assert!(!discovery.candidates[0].selected_by_default);
}
