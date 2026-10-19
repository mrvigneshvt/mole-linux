// Ported contracts: tw93/Mole tests/clean_dev_caches.bats (JS cases) and
// tests/clean_corepack.bats (GPL-3.0). Owner tools are PATH-free shims that
// record their argv; no test touches the real HOME or a real package manager.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mole_linux::adapters::{AdapterEnv, Bun, Corepack, Npm, Pnpm, Report, run_adapter};
use mole_linux::core::outcome::Outcome;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::RunMode;
use mole_linux::core::proc::{ProcessState, is_command_running};
use mole_linux::core::whitelist::Whitelist;

const QUERY: Duration = Duration::from_secs(5);
const CLEAN: Duration = Duration::from_secs(20);

/// Writes an executable `sh` shim named `file_name` that appends `label` and its argv to `log`, then runs `body`.
fn write_shim(dir: &Path, file_name: &str, label: &str, log: &Path, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{label}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = log.display(),
    );
    let path = dir.join(file_name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    path
}

fn read_log(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Writes a fake `/proc/<pid>/cmdline` with NUL-separated argv, as the kernel does.
fn fake_process(proc_root: &Path, pid: u32, argv: &[&str]) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("fake process dir");
    let mut bytes = Vec::new();
    for arg in argv {
        bytes.extend_from_slice(arg.as_bytes());
        bytes.push(0);
    }
    fs::write(dir.join("cmdline"), bytes).expect("fake cmdline");
}

struct Sandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    proc_root: PathBuf,
    log: PathBuf,
    whitelist: Whitelist,
}

fn sandbox() -> Sandbox {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("empty whitelist");
    let proc_root = root.join("proc");
    fs::create_dir_all(&proc_root).expect("proc root");
    Sandbox {
        bin: root.join("bin"),
        log: root.join("calls.log"),
        _temp: temp,
        home,
        proc_root,
        whitelist,
    }
}

impl Sandbox {
    fn env(&self, path: &Path) -> AdapterEnv {
        AdapterEnv {
            home: self.home.clone(),
            path: OsString::from(path),
            proc_root: self.proc_root.clone(),
            whitelist: self.whitelist.clone(),
            query_timeout: QUERY,
            clean_timeout: CLEAN,
            corepack_home: None,
        }
    }
}

fn only(reports: &[Report]) -> &Report {
    assert_eq!(reports.len(), 1, "expected one report: {reports:?}");
    &reports[0]
}

// --- npm -------------------------------------------------------------------

#[test]
fn npm_dry_run_previews_owner_cache_without_cleaning() {
    let s = sandbox();
    let cache = s.home.join(".npm");
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo '{}';; esac",
            cache.display()
        ),
    );
    let reports = run_adapter(&Npm, &s.env(&s.bin), RunMode::DryRun);
    let report = only(&reports);
    assert_eq!(report.outcome, Outcome::Unchanged, "{report:?}");
    assert_eq!(report.target.as_deref(), Some(cache.as_path()));
    assert!(
        report.reason.contains("npm cache clean --force"),
        "{report:?}"
    );
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("cache\tclean")),
        "dry run must not clean: {calls:?}"
    );
}

#[test]
fn npm_apply_runs_owner_cache_clean_force() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo '{}/.npm';; esac",
            s.home.display()
        ),
    );
    let reports = run_adapter(&Npm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Applied, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.contains(&"npm\tcache\tclean\t--force".to_owned()),
        "argv: {calls:?}"
    );
}

#[test]
fn npm_missing_is_unavailable_not_zero() {
    let s = sandbox();
    let empty_bin = s.bin.clone();
    fs::create_dir_all(&empty_bin).expect("empty PATH dir");
    let reports = run_adapter(&Npm, &s.env(&empty_bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
}

#[test]
fn relative_path_entries_are_never_resolved() {
    let s = sandbox();
    // A relative PATH entry would let a repository's ./npm run as the user.
    // The shim exists only in an absolute directory that is not on PATH.
    write_shim(&s.bin, "npm", "npm", &s.log, "echo 10.9.0");
    let mut env = s.env(&s.bin);
    env.path = OsString::from("relative-bin");
    let reports = run_adapter(&Npm, &env, RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    assert!(
        read_log(&s.log).is_empty(),
        "relative PATH must never execute"
    );
}

#[test]
fn running_npm_is_skipped_and_never_queried() {
    let s = sandbox();
    write_shim(&s.bin, "npm", "npm", &s.log, "echo 10.9.0");
    fake_process(&s.proc_root, 4242, &["npm", "install", "left-pad"]);
    let reports = run_adapter(&Npm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Skipped, "{reports:?}");
    assert!(read_log(&s.log).is_empty(), "busy owner must not run");
}

#[test]
fn unknown_process_state_is_unavailable_not_safe() {
    let s = sandbox();
    write_shim(&s.bin, "npm", "npm", &s.log, "echo 10.9.0");
    let mut env = s.env(&s.bin);
    env.proc_root = s.bin.join("no-such-proc");
    let reports = run_adapter(&Npm, &env, RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    assert!(read_log(&s.log).is_empty());
}

#[test]
fn non_absolute_npm_cache_path_is_unavailable_and_not_cleaned() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo 'relative/cache';; esac",
    );
    let reports = run_adapter(&Npm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("cache\tclean")),
        "{calls:?}"
    );
}

#[test]
fn whitelisted_npm_cache_is_skipped_without_cleaning() {
    let s = sandbox();
    let cache = s.home.join(".npm");
    let config = s.home.join(".config/mole-linux");
    fs::create_dir_all(&config).expect("config dir");
    fs::write(config.join("whitelist"), format!("{}\n", cache.display())).expect("whitelist");
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let mut env = s.env(&s.bin);
    env.whitelist = Whitelist::load(&xdg).expect("whitelist load");
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo '{}';; esac",
            cache.display()
        ),
    );
    let reports = run_adapter(&Npm, &env, RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Skipped, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("cache\tclean")),
        "{calls:?}"
    );
}

#[test]
fn failing_npm_clean_is_failed_not_applied() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo '{}/.npm';; 'cache clean --force') exit 3;; esac",
            s.home.display()
        ),
    );
    let reports = run_adapter(&Npm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Failed, "{reports:?}");
}

#[test]
fn hung_owner_command_is_killed_and_reported_unavailable() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo '{}/.npm';; 'cache clean --force') sleep 30;; esac",
            s.home.display()
        ),
    );
    let mut env = s.env(&s.bin);
    env.clean_timeout = Duration::from_millis(300);
    let started = Instant::now();
    let reports = run_adapter(&Npm, &env, RunMode::Apply);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "timeout not enforced"
    );
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    assert!(only(&reports).reason.contains("timed out"), "{reports:?}");
}

// --- pnpm ------------------------------------------------------------------

#[test]
fn pnpm_prunes_each_distinct_store_from_path_and_mise_installs() {
    let s = sandbox();
    let v11 = s.home.join("Library/pnpm/store/v11");
    let v10 = s.home.join(".local/share/pnpm/store/v10");
    write_shim(
        &s.bin,
        "pnpm",
        "pnpm11",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 11.17.0;; 'store path') echo '{}';; esac",
            v11.display()
        ),
    );
    let mise = s.home.join(".local/share/mise/installs/pnpm/10.34.5");
    write_shim(
        &mise,
        "pnpm",
        "pnpm10",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 10.34.5;; 'store path') echo '{}';; esac",
            v10.display()
        ),
    );
    let reports = run_adapter(&Pnpm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(
        reports.iter().all(|r| r.outcome == Outcome::Applied),
        "{reports:?}"
    );
    let calls = read_log(&s.log);
    assert!(
        calls.contains(&"pnpm11\tstore\tprune".to_owned()),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"pnpm10\tstore\tprune".to_owned()),
        "{calls:?}"
    );
}

#[test]
fn pnpm_same_store_from_two_binaries_is_pruned_once() {
    let s = sandbox();
    let store = s.home.join("Library/pnpm/store/v11");
    let body = format!(
        "case \"$*\" in '--version') echo 11.17.0;; 'store path') echo '{}';; esac",
        store.display()
    );
    write_shim(&s.bin, "pnpm", "pnpm-path", &s.log, &body);
    let mise = s.home.join(".local/share/mise/installs/pnpm/11.17.0");
    write_shim(&mise, "pnpm", "pnpm-mise", &s.log, &body);
    let reports = run_adapter(&Pnpm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(reports.len(), 1, "{reports:?}");
    let prunes = read_log(&s.log)
        .into_iter()
        .filter(|call| call.ends_with("store\tprune"))
        .count();
    assert_eq!(prunes, 1, "one store, one prune");
}

#[test]
fn pnpm_store_path_with_parent_traversal_is_refused() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "pnpm",
        "pnpm",
        &s.log,
        "case \"$*\" in '--version') echo 11.0.0;; 'store path') echo '/home/x/../etc';; esac",
    );
    let reports = run_adapter(&Pnpm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("prune")),
        "{calls:?}"
    );
}

#[test]
fn running_pnpm_started_by_node_is_skipped() {
    let s = sandbox();
    write_shim(&s.bin, "pnpm", "pnpm", &s.log, "echo 11.0.0");
    fake_process(
        &s.proc_root,
        7001,
        &["node", "/usr/lib/corepack/dist/pnpm.cjs", "install"],
    );
    let reports = run_adapter(&Pnpm, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Skipped, "{reports:?}");
    assert!(read_log(&s.log).is_empty());
}

// --- bun -------------------------------------------------------------------

#[test]
fn bun_apply_runs_owner_pm_cache_rm() {
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    write_shim(
        &s.bin,
        "bun",
        "bun",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 1.4.2;; 'pm cache') echo '{}';; esac",
            cache.display()
        ),
    );
    let reports = run_adapter(&Bun, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Applied, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.contains(&"bun\tpm\tcache\trm".to_owned()),
        "{calls:?}"
    );
}

#[test]
fn bun_relative_cache_path_is_unavailable() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in '--version') echo 1.4.2;; 'pm cache') echo 'cache';; esac",
    );
    let reports = run_adapter(&Bun, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(calls.iter().all(|call| !call.contains("rm")), "{calls:?}");
}

#[test]
fn failing_bun_rm_is_failed_with_no_filesystem_fallback() {
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    fs::create_dir_all(&cache).expect("cache dir");
    fs::write(cache.join("pkg.tgz"), b"cached").expect("cache entry");
    write_shim(
        &s.bin,
        "bun",
        "bun",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 1.4.2;; 'pm cache') echo '{}';; 'pm cache rm') exit 1;; esac",
            cache.display()
        ),
    );
    let reports = run_adapter(&Bun, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Failed, "{reports:?}");
    assert!(
        cache.join("pkg.tgz").exists(),
        "adapter must never delete files itself"
    );
}

// --- corepack --------------------------------------------------------------

#[test]
fn corepack_suppresses_download_prompt_for_version_and_clean() {
    let s = sandbox();
    // The shim records the env it actually sees, so a missing setting shows as UNSET.
    write_shim(
        &s.bin,
        "corepack",
        "corepack",
        &s.log,
        &format!(
            "echo \"env:${{COREPACK_ENABLE_DOWNLOAD_PROMPT:-UNSET}}\" >> '{}'",
            s.log.display()
        ),
    );
    let reports = run_adapter(&Corepack, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Applied, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.contains(&"corepack\t--version".to_owned()),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"corepack\tcache\tclean".to_owned()),
        "{calls:?}"
    );
    let envs: Vec<&String> = calls.iter().filter(|c| c.starts_with("env:")).collect();
    assert!(!envs.is_empty(), "{calls:?}");
    assert!(
        envs.iter().all(|c| c.as_str() == "env:0"),
        "prompt not suppressed: {calls:?}"
    );
}

#[test]
fn corepack_missing_is_unavailable_and_never_deletes_its_cache() {
    let s = sandbox();
    let cache = s.home.join(".cache/node/corepack");
    fs::create_dir_all(&cache).expect("corepack cache");
    fs::write(cache.join("pnpm.tgz"), b"cached").expect("cache entry");
    fs::create_dir_all(&s.bin).expect("empty PATH dir");
    let reports = run_adapter(&Corepack, &s.env(&s.bin), RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Unavailable, "{reports:?}");
    assert!(
        cache.join("pnpm.tgz").exists(),
        "missing owner must not delete"
    );
}

#[test]
fn corepack_home_defaults_under_home_cache_in_dry_run() {
    let s = sandbox();
    write_shim(&s.bin, "corepack", "corepack", &s.log, "echo 0.34.0");
    let reports = run_adapter(&Corepack, &s.env(&s.bin), RunMode::DryRun);
    let report = only(&reports);
    assert_eq!(report.outcome, Outcome::Unchanged, "{reports:?}");
    assert_eq!(
        report.target.as_deref(),
        Some(s.home.join(".cache/node/corepack").as_path())
    );
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("clean")),
        "{calls:?}"
    );
}

#[test]
fn corepack_home_at_filesystem_root_is_refused() {
    let s = sandbox();
    write_shim(&s.bin, "corepack", "corepack", &s.log, "echo 0.34.0");
    let mut env = s.env(&s.bin);
    env.corepack_home = Some(OsString::from("/"));
    let reports = run_adapter(&Corepack, &env, RunMode::Apply);
    assert_eq!(only(&reports).outcome, Outcome::Skipped, "{reports:?}");
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("clean")),
        "{calls:?}"
    );
}

// --- process probe ---------------------------------------------------------

#[test]
fn command_probe_matches_node_wrapped_script_in_argv1() {
    let s = sandbox();
    fake_process(&s.proc_root, 10, &["node", "/x/npm-cli.js", "ci"]);
    assert_eq!(
        is_command_running(&s.proc_root, &["npm", "npm-cli.js"]),
        ProcessState::Running
    );
}

#[test]
fn command_probe_ignores_lockfile_and_argument_mentions() {
    let s = sandbox();
    fake_process(&s.proc_root, 11, &["vim", "pnpm-lock.yaml"]);
    fake_process(
        &s.proc_root,
        12,
        &["mole-linux", "clean", "--select", "npm"],
    );
    assert_eq!(
        is_command_running(&s.proc_root, &["pnpm", "pnpm.cjs"]),
        ProcessState::NotRunning
    );
    assert_eq!(
        is_command_running(&s.proc_root, &["npm", "npm-cli.js"]),
        ProcessState::NotRunning
    );
}

#[test]
fn command_probe_unreadable_proc_is_unknown() {
    let s = sandbox();
    assert_eq!(
        is_command_running(&s.bin.join("no-such-proc"), &["npm"]),
        ProcessState::Unknown
    );
}
