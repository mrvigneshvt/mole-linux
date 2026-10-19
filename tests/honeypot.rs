#[path = "honeypot/fixtures.rs"]
mod fixtures;

use std::fs;
use std::process::Command;
#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

#[test]
fn builds_realistic_home_and_canaries_survive() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");

    for relative in [
        ".config",
        ".cache",
        ".local/share",
        ".config/chromium/Default/Cookies",
        ".config/chromium/Default/History",
        ".config/chromium/Default/Login Data",
        ".config/google-chrome/Default/Cookies",
        ".mozilla/firefox/x.default-release/places.sqlite",
        ".mozilla/firefox/x.default-release/places.sqlite-wal",
        ".mozilla/firefox/x.default-release/cookies.sqlite",
        ".ssh/id_ed25519",
        ".gnupg/private-keys-v1.d/key.key",
        ".local/share/keyrings/login.keyring",
        "project/.env",
        "Documents/thesis.odt",
        "Pictures/photo.jpg",
        ".local/share/docker-volume-like/volumes/db/data",
        "models/llama.gguf",
        ".local/share/app/db.sqlite",
        ".local/share/app/db.sqlite-wal",
        ".local/share/app/db.sqlite-shm",
        "project/src/main.rs",
        "project/.git/HEAD",
        "project/node_modules/authored-package/index.js",
        ".config/mole-linux/whitelist-protected/file",
        ".npm/_cacache",
        ".cache/pnpm",
        ".cache/uv",
        ".cargo/registry",
        ".gradle/caches",
        ".cache/chromium/Default/Cache",
        ".cache/mozilla/firefox/x.default-release/cache2",
        ".cache/thumbnails",
        ".cache/yay/pkg-build",
        "project/src/__pycache__",
        "cargo-project/Cargo.toml",
        "cargo-project/target",
        "project/.git",
        "project-linked/.git",
        "Downloads/installer.AppImage",
        ".local/share/Trash/files/old",
    ] {
        assert!(home.join(relative).exists(), "missing {relative}");
    }
    assert!(!home.join("project/node_modules/.keep-authored").exists());

    fixture.assert_canaries_intact().expect("canaries intact");
    fs::write(home.join(".ssh/id_ed25519"), "modified").expect("modify canary");
    assert!(fixture.assert_canaries_intact().is_err());
}

#[test]
fn generated_tool_shims_log_argv_and_print_canned_output() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let bin = temp.path().join("bin");
    let log = temp.path().join("calls.log");
    fixtures::generate_tool_shims(&bin, &log).expect("generate shims");

    let output = Command::new(bin.join("pacman"))
        .args(["-Q", "example"])
        .output()
        .expect("run shim");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "mole-linux-shim pacman\n"
    );
    assert_eq!(
        fs::read_to_string(log).expect("read log"),
        "pacman\t-Q\texample\n"
    );
}

#[test]
fn fixture_builder_refuses_to_overlay_an_existing_home() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("create existing home");
    fs::write(home.join("keep"), "unchanged").expect("seed existing file");

    assert!(fixtures::HomeFixture::build(&home).is_err());
    assert_eq!(
        fs::read_to_string(home.join("keep")).expect("read existing file"),
        "unchanged"
    );
}

#[test]
fn fixture_contains_a_real_tracked_repo_and_untracked_worktree_content() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let _fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");

    let tracked = Command::new("git")
        .args([
            "-C",
            home.join("project").to_str().expect("utf8 path"),
            "ls-files",
            "--error-unmatch",
            "src/main.rs",
        ])
        .output()
        .expect("query tracked file");
    assert!(tracked.status.success());

    let untracked = Command::new("git")
        .args([
            "-C",
            home.join("project").to_str().expect("utf8 path"),
            "ls-files",
            "--error-unmatch",
            "node_modules/authored-package/index.js",
        ])
        .output()
        .expect("query authored dependency file");
    assert!(!untracked.status.success());

    let worktrees = Command::new("git")
        .args([
            "-C",
            home.join("project").to_str().expect("utf8 path"),
            "worktree",
            "list",
            "--porcelain",
        ])
        .output()
        .expect("list worktrees");
    assert!(worktrees.status.success());
    assert!(String::from_utf8_lossy(&worktrees.stdout).contains("project-linked"));
}

#[test]
fn clean_dryrun_scenario_keeps_every_honeypot_canary() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");
    let scenario = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/honeypot/scenarios/clean-dryrun.sh");
    let output = Command::new("bash")
        .arg(scenario)
        .arg(env!("CARGO_BIN_EXE_mole-linux"))
        .isolated_home(&home)
        .output()
        .expect("run clean dry-run scenario");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fixture
        .assert_canaries_intact()
        .expect("canaries intact after clean preview");
}

#[test]
fn clean_browser_dryrun_keeps_browser_caches_and_every_canary() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");
    let scenario = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/honeypot/scenarios/clean-browser-dryrun.sh");
    let output = Command::new("bash")
        .arg(scenario)
        .arg(env!("CARGO_BIN_EXE_mole-linux"))
        .isolated_home(&home)
        .output()
        .expect("run browser cache preview");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fixture
        .assert_canaries_intact()
        .expect("canaries intact after browser cache preview");
}

#[test]
fn clean_browser_apply_removes_only_browser_caches_and_keeps_every_canary() {
    let temp = tempfile::tempdir().expect("temporary workspace");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot");
    let scenario = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/honeypot/scenarios/clean-browser-apply.sh");
    let output = Command::new("bash")
        .arg(scenario)
        .arg(env!("CARGO_BIN_EXE_mole-linux"))
        .isolated_home(&home)
        .output()
        .expect("run browser cache cleanup");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fixture
        .assert_canaries_intact()
        .expect("canaries intact after browser cache cleanup");
}
