// Ported contracts: tw93/Mole tests/clean_dev_caches.bats (JS cases) and
// tests/clean_corepack.bats (GPL-3.0). Owner tools are shims that record their
// argv; no test touches the real HOME or a real package manager.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mole_linux::adapters::{
    AdapterEnv, Bun, Corepack, Npm, OwnerAdapter, Pnpm, Refusal, Resolved, resolve,
    run_owner_action,
};
use mole_linux::core::outcome::Outcome;
use mole_linux::core::proc::{ProcessState, is_command_running};
use mole_linux::rules::clean::OwnerAction;

const QUERY: Duration = Duration::from_secs(5);
const CLEAN: Duration = Duration::from_secs(20);

/// Writes an executable `sh` shim that appends `label` and its argv to `log`, then runs `body`.
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
}

fn sandbox() -> Sandbox {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    let proc_root = root.join("proc");
    fs::create_dir_all(&proc_root).expect("proc root");
    Sandbox {
        bin: root.join("bin"),
        log: root.join("calls.log"),
        _temp: temp,
        home,
        proc_root,
    }
}

impl Sandbox {
    fn env(&self, path: &Path) -> AdapterEnv {
        AdapterEnv {
            home: self.home.clone(),
            path: OsString::from(path),
            proc_root: self.proc_root.clone(),
            query_timeout: QUERY,
            clean_timeout: CLEAN,
            corepack_home: None,
            bun_install_cache_dir: None,
            bun_install: None,
        }
    }

    /// Resolves the only binary the adapter sees; panics with the full result otherwise.
    fn first(&self, adapter: &dyn OwnerAdapter) -> Result<Resolved, Refusal> {
        let results = resolve(adapter, &self.env(&self.bin)).expect("resolve");
        assert_eq!(results.len(), 1, "expected one binary: {results:?}");
        results.into_iter().next().expect("one result")
    }
}

fn action(program: &Path, adapter: &dyn OwnerAdapter) -> OwnerAction {
    OwnerAction {
        program: program.to_path_buf(),
        args: adapter.clean_args(),
        timeout: CLEAN,
    }
}

// --- npm -------------------------------------------------------------------

/// Shim tests in one binary run one at a time. A sibling test's fork can hold a freshly written
/// shim open for write, and exec then fails with ETXTBSY ("Text file busy").
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn npm_resolves_the_cache_the_owner_reports_without_running_clean() {
    let _serial = serial();
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
    let resolved = s.first(&Npm).expect("npm resolves");
    assert_eq!(resolved.target, cache);
    assert_eq!(resolved.binary, s.bin.join("npm"));
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("clean")),
        "{calls:?}"
    );
}

#[test]
fn npm_missing_is_unavailable_not_zero() {
    let _serial = serial();
    let s = sandbox();
    fs::create_dir_all(&s.bin).expect("empty PATH dir");
    let refusal = resolve(&Npm, &s.env(&s.bin)).expect_err("missing npm");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn relative_path_entries_are_never_resolved() {
    let _serial = serial();
    let s = sandbox();
    // A relative PATH entry would let a repository's ./npm run as the user.
    write_shim(&s.bin, "npm", "npm", &s.log, "echo 10.9.0");
    let mut env = s.env(&s.bin);
    env.path = OsString::from("relative-bin");
    let refusal = resolve(&Npm, &env).expect_err("relative PATH");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(
        read_log(&s.log).is_empty(),
        "relative PATH must never execute"
    );
}

#[test]
fn running_npm_is_refused_before_any_owner_query() {
    let _serial = serial();
    let s = sandbox();
    write_shim(&s.bin, "npm", "npm", &s.log, "echo 10.9.0");
    fake_process(&s.proc_root, 4242, &["npm", "install", "left-pad"]);
    let refusal = resolve(&Npm, &s.env(&s.bin)).expect_err("busy npm");
    assert_eq!(refusal.outcome, Outcome::Skipped, "{refusal:?}");
    assert!(
        read_log(&s.log).is_empty(),
        "busy owner must not be queried"
    );
}

#[test]
fn unknown_process_state_is_unavailable_not_safe() {
    let _serial = serial();
    let s = sandbox();
    write_shim(&s.bin, "npm", "npm", &s.log, "echo 10.9.0");
    let mut env = s.env(&s.bin);
    env.proc_root = s.bin.join("no-such-proc");
    let refusal = resolve(&Npm, &env).expect_err("unknown state");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(read_log(&s.log).is_empty());
}

#[test]
fn non_absolute_npm_cache_path_is_refused() {
    let _serial = serial();
    let s = sandbox();
    write_shim(
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in '--version') echo 10.9.0;; 'config get cache') echo 'relative/cache';; esac",
    );
    let refusal = s.first(&Npm).expect_err("relative cache path");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn npm_clean_action_runs_owner_cache_clean_force() {
    let _serial = serial();
    let s = sandbox();
    let program = write_shim(&s.bin, "npm", "npm", &s.log, "exit 0");
    let (outcome, reason) = run_owner_action(&action(&program, &Npm));
    assert_eq!(outcome, Outcome::Applied, "{reason}");
    assert_eq!(
        read_log(&s.log),
        vec!["npm\tcache\tclean\t--force".to_owned()]
    );
}

#[test]
fn failing_owner_clean_is_failed() {
    let _serial = serial();
    let s = sandbox();
    let program = write_shim(&s.bin, "npm", "npm", &s.log, "exit 3");
    let (outcome, reason) = run_owner_action(&action(&program, &Npm));
    assert_eq!(outcome, Outcome::Failed, "{reason}");
}

#[test]
fn hung_owner_command_is_killed_and_reported_unavailable() {
    let _serial = serial();
    let s = sandbox();
    let program = write_shim(&s.bin, "npm", "npm", &s.log, "sleep 30");
    let short = OwnerAction {
        program: program.clone(),
        args: Npm.clean_args(),
        timeout: Duration::from_millis(300),
    };
    let started = Instant::now();
    let (outcome, reason) = run_owner_action(&short);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "timeout not enforced"
    );
    assert_eq!(outcome, Outcome::Unavailable, "{reason}");
    assert!(reason.contains("timed out"), "{reason}");
}

// --- pnpm ------------------------------------------------------------------

#[test]
fn pnpm_resolves_each_distinct_store_from_path_and_mise_installs() {
    let _serial = serial();
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
    let results = resolve(&Pnpm, &s.env(&s.bin)).expect("resolve");
    let targets: Vec<PathBuf> = results
        .into_iter()
        .map(|result| result.expect("usable binary").target)
        .collect();
    assert_eq!(targets, vec![v11, v10]);
}

#[test]
fn pnpm_store_path_with_parent_traversal_is_refused() {
    let _serial = serial();
    let s = sandbox();
    write_shim(
        &s.bin,
        "pnpm",
        "pnpm",
        &s.log,
        "case \"$*\" in '--version') echo 11.0.0;; 'store path') echo '/home/x/../etc';; esac",
    );
    let refusal = s.first(&Pnpm).expect_err("traversal");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn pnpm_store_prune_action_runs_owner_prune() {
    let _serial = serial();
    let s = sandbox();
    let program = write_shim(&s.bin, "pnpm", "pnpm", &s.log, "exit 0");
    let (outcome, reason) = run_owner_action(&action(&program, &Pnpm));
    assert_eq!(outcome, Outcome::Applied, "{reason}");
    assert_eq!(read_log(&s.log), vec!["pnpm\tstore\tprune".to_owned()]);
}

#[test]
fn running_pnpm_started_by_node_is_refused() {
    let _serial = serial();
    let s = sandbox();
    write_shim(&s.bin, "pnpm", "pnpm", &s.log, "echo 11.0.0");
    fake_process(
        &s.proc_root,
        7001,
        &["node", "/usr/lib/corepack/dist/pnpm.cjs", "install"],
    );
    let refusal = resolve(&Pnpm, &s.env(&s.bin)).expect_err("busy pnpm");
    assert_eq!(refusal.outcome, Outcome::Skipped, "{refusal:?}");
    assert!(read_log(&s.log).is_empty());
}

// --- bun -------------------------------------------------------------------

#[test]
fn bun_resolves_the_cache_bun_reports() {
    let _serial = serial();
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
    assert_eq!(s.first(&Bun).expect("bun resolves").target, cache);
}

#[test]
fn bun_cache_query_failure_is_refused_with_the_owner_status() {
    let _serial = serial();
    let s = sandbox();
    write_shim(
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in '--version') echo 1.4.2;; 'pm cache') exit 1;; esac",
    );
    let refusal = s.first(&Bun).expect_err("query failed");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(refusal.reason.contains("pm cache"), "{refusal:?}");
    assert!(read_log(&s.log).iter().all(|call| !call.contains("rm")));
}

#[test]
fn bun_relative_cache_path_is_refused() {
    let _serial = serial();
    let s = sandbox();
    write_shim(
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in '--version') echo 1.4.2;; 'pm cache') echo 'cache';; esac",
    );
    let refusal = s.first(&Bun).expect_err("relative");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn bun_cache_rm_failure_is_failed_and_deletes_nothing_itself() {
    let _serial = serial();
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    fs::create_dir_all(&cache).expect("cache dir");
    fs::write(cache.join("pkg.tgz"), b"cached").expect("cache entry");
    let program = write_shim(&s.bin, "bun", "bun", &s.log, "exit 1");
    let (outcome, _) = run_owner_action(&action(&program, &Bun));
    assert_eq!(outcome, Outcome::Failed);
    assert!(
        cache.join("pkg.tgz").exists(),
        "the adapter must never delete files itself"
    );
}

// --- corepack --------------------------------------------------------------

#[test]
fn corepack_probe_suppresses_the_download_prompt() {
    let _serial = serial();
    let s = sandbox();
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
    let resolved = s.first(&Corepack).expect("corepack resolves");
    assert_eq!(resolved.target, s.home.join(".cache/node/corepack"));
    let calls = read_log(&s.log);
    assert!(
        calls.contains(&"corepack\t--version".to_owned()),
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
    let _serial = serial();
    let s = sandbox();
    let cache = s.home.join(".cache/node/corepack");
    fs::create_dir_all(&cache).expect("corepack cache");
    fs::write(cache.join("pnpm.tgz"), b"cached").expect("cache entry");
    fs::create_dir_all(&s.bin).expect("empty PATH dir");
    let refusal = resolve(&Corepack, &s.env(&s.bin)).expect_err("missing corepack");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(
        cache.join("pnpm.tgz").exists(),
        "missing owner must not delete"
    );
}

#[test]
fn corepack_home_at_filesystem_root_is_refused() {
    let _serial = serial();
    let s = sandbox();
    write_shim(&s.bin, "corepack", "corepack", &s.log, "echo 0.34.0");
    let mut env = s.env(&s.bin);
    env.corepack_home = Some(OsString::from("/"));
    let refusal = resolve(&Corepack, &env)
        .expect("binary resolves")
        .into_iter()
        .next()
        .expect("one binary")
        .expect_err("root");
    assert_eq!(refusal.outcome, Outcome::Skipped, "{refusal:?}");
}

#[test]
fn corepack_clean_action_runs_owner_cache_clean() {
    let _serial = serial();
    let s = sandbox();
    let program = write_shim(&s.bin, "corepack", "corepack", &s.log, "exit 0");
    let (outcome, _) = run_owner_action(&action(&program, &Corepack));
    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(read_log(&s.log), vec!["corepack\tcache\tclean".to_owned()]);
}

// --- process probe ---------------------------------------------------------

#[test]
fn command_probe_matches_node_wrapped_script_in_argv1() {
    let _serial = serial();
    let s = sandbox();
    fake_process(&s.proc_root, 10, &["node", "/x/npm-cli.js", "ci"]);
    assert_eq!(
        is_command_running(&s.proc_root, &["npm", "npm-cli.js"]),
        ProcessState::Running
    );
}

#[test]
fn command_probe_ignores_lockfile_and_argument_mentions() {
    let _serial = serial();
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
    let _serial = serial();
    let s = sandbox();
    assert_eq!(
        is_command_running(&s.bin.join("no-such-proc"), &["npm"]),
        ProcessState::Unknown
    );
}
