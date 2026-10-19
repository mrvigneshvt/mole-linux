//! #81 polish contracts: clean rows show SI sizes, the paccache preview supplies the
//! logical size its directory candidate lacks, and a `__pycache__` row names its project.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};
use mole_linux::core::proc::ProcessState;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{CleanRule, DiscoveryStatus, RuleContext};
use mole_linux::rules::project_cache::PyCacheRule;
use mole_linux::rules::system::PaccacheRule;

/// Shim tests in one binary run one at a time; a freshly written shim can hit ETXTBSY otherwise.
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn paccache_shim(bin: &Path, preview: &str) {
    fs::create_dir_all(bin).expect("shim dir");
    let path = bin.join("paccache");
    // The preview is parsed as English with '.' decimals, so the query must run under LC_ALL=C.
    let script = format!(
        "#!/bin/sh\n[ \"$LC_ALL\" = C ] || exit 9\ncase \"$1\" in\n  -d*) printf '%s\\n' '{preview}' ;;\n  *) exit 9 ;;\nesac\n"
    );
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

#[test]
fn paccache_preview_supplies_the_logical_size_of_the_pacman_row() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    paccache_shim(
        &bin,
        "==> finished dry run: 1 candidates (disk space saved: 510.4 KiB)",
    );
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let discovery = PaccacheRule::keep_three(cache.clone()).discover_with_owner(&ctx, &runner);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.apparent_size, Some(522_649), "{candidate:?}");
    assert_eq!(candidate.allocated_size, None);
}

#[test]
fn paccache_preview_without_a_saved_figure_keeps_the_size_unknown() {
    let _serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let cache = temp.path().join("pkg");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cache).expect("cache");
    let bin = temp.path().join("usr-bin");
    paccache_shim(&bin, "==> finished dry run: 2 candidates");
    let runner = OwnerRunner::new(bin, temp.path().join("no-sudo"), Consent::Unasked);
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let discovery = PaccacheRule::keep_three(cache.clone()).discover_with_owner(&ctx, &runner);

    assert_eq!(discovery.candidates.len(), 1);
    assert_eq!(discovery.candidates[0].apparent_size, None);
}

#[test]
fn clean_rows_and_summary_use_si_units_not_raw_bytes() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".test-xdg/cache/thumbnails")).expect("thumbnail directory");
    fs::write(
        home.join(".test-xdg/cache/thumbnails/preview.png"),
        vec![b't'; 1_500_000],
    )
    .expect("thumbnail");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["clean", "--dry-run", "--select", "user-essentials"])
        .isolated_home(&home)
        .output()
        .expect("run clean");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("logical=1.5 MB"), "{stdout}");
    assert!(
        !stdout.contains(" bytes"),
        "raw byte counts remain: {stdout}"
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    home: std::path::PathBuf,
}

fn pycache_fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let cache = home.join("Projects/app/pkg/__pycache__");
    fs::create_dir_all(&cache).expect("pycache");
    fs::write(cache.join("module.cpython-312.pyc"), b"bytecode").expect("pyc");
    Fixture { _temp: temp, home }
}

#[test]
fn pycache_row_names_the_owning_project_not_the_search_root() {
    let fx = pycache_fixture();
    let xdg = XdgPaths::from_values(fx.home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &fx.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let discovery = PyCacheRule.discover(&ctx);

    assert_eq!(discovery.status, DiscoveryStatus::Complete);
    let candidate = discovery.candidates.first().expect("pycache candidate");
    assert_eq!(
        candidate.reason, "Python bytecode cache in app",
        "{candidate:?}"
    );
}

#[test]
fn pycache_directly_under_the_search_root_is_labelled_with_the_root() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    let cache = home.join("Projects/__pycache__");
    fs::create_dir_all(&cache).expect("pycache");
    fs::write(cache.join("module.cpython-312.pyc"), b"bytecode").expect("pyc");
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };

    let discovery = PyCacheRule.discover(&ctx);

    let candidate = discovery.candidates.first().expect("pycache candidate");
    assert_eq!(
        candidate.reason, "Python bytecode cache in Projects",
        "{candidate:?}"
    );
}
