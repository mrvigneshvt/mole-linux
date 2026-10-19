// Ported contracts: tw93/Mole tests/clean_dev_caches.bats (uv, pip3, conda cases) at
// 8c55e8c (GPL-3.0). Owner tools are shims that record their argv; the HOME is a
// temporary directory and no test touches the real package managers.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mole_linux::adapters::{
    AdapterEnv, Conda, OwnerAdapter, Pip, Refusal, Uv, resolve, run_owner_action,
};
use mole_linux::core::outcome::Outcome;
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

/// Writes a fake `/proc/<pid>/comm` and `cmdline`, as the kernel does.
fn fake_process(proc_root: &Path, pid: u32, comm: &str, argv: &[&str]) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("fake process dir");
    fs::write(dir.join("comm"), format!("{comm}\n")).expect("fake comm");
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
            mise_cache_dir: None,
        }
    }

    /// Resolves the only binary the adapter sees; panics with the full result otherwise.
    fn first(&self, adapter: &dyn OwnerAdapter) -> Result<mole_linux::adapters::Resolved, Refusal> {
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

// --- uv --------------------------------------------------------------------

#[test]
fn uv_resolves_the_cache_uv_reports_without_running_prune() {
    let s = sandbox();
    let cache = s.home.join("custom/uv-cache");
    write_shim(
        &s.bin,
        "uv",
        "uv",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 0.9.0;; 'cache dir') echo '{}';; esac",
            cache.display()
        ),
    );
    let resolved = s.first(&Uv).expect("uv resolves");
    assert_eq!(resolved.target, cache);
    assert_eq!(resolved.guarded, vec![cache]);
    let calls = read_log(&s.log);
    assert!(
        calls.iter().all(|call| !call.contains("prune")),
        "{calls:?}"
    );
}

#[test]
fn uv_cache_dir_failure_is_unavailable_and_runs_no_prune() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "uv",
        "uv",
        &s.log,
        "case \"$*\" in '--version') echo 0.9.0;; 'cache dir') exit 2;; esac",
    );
    let refusal = s.first(&Uv).expect_err("cache dir failed");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(refusal.reason.contains("cache dir"), "{refusal:?}");
    assert!(read_log(&s.log).iter().all(|call| !call.contains("prune")));
}

#[test]
fn uv_relative_cache_dir_is_refused() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "uv",
        "uv",
        &s.log,
        "case \"$*\" in '--version') echo 0.9.0;; 'cache dir') echo 'relative/cache';; esac",
    );
    let refusal = s.first(&Uv).expect_err("relative");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn missing_uv_is_unavailable_and_its_default_cache_survives() {
    let s = sandbox();
    let cache = s.home.join(".cache/uv");
    fs::create_dir_all(&cache).expect("uv cache");
    fs::write(cache.join("entry"), b"cached").expect("entry");
    fs::create_dir_all(&s.bin).expect("empty PATH dir");
    let refusal = resolve(&Uv, &s.env(&s.bin)).expect_err("missing uv");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(
        cache.join("entry").exists(),
        "missing owner must not delete"
    );
}

#[test]
fn uv_cache_prune_action_runs_owner_prune() {
    let s = sandbox();
    let program = write_shim(&s.bin, "uv", "uv", &s.log, "exit 0");
    let (outcome, reason) = run_owner_action(&action(&program, &Uv));
    assert_eq!(outcome, Outcome::Applied, "{reason}");
    assert_eq!(read_log(&s.log), vec!["uv\tcache\tprune".to_owned()]);
}

// --- pip3 ------------------------------------------------------------------

#[test]
fn pip3_resolves_the_cache_pip_reports() {
    let s = sandbox();
    let cache = s.home.join(".cache/pip");
    write_shim(
        &s.bin,
        "pip3",
        "pip3",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 'pip 25.0';; 'cache dir') echo '{}';; esac",
            cache.display()
        ),
    );
    let resolved = s.first(&Pip).expect("pip3 resolves");
    assert_eq!(resolved.target, cache);
    assert_eq!(resolved.binary, s.bin.join("pip3"));
}

#[test]
fn pip3_cache_purge_action_runs_owner_purge() {
    let s = sandbox();
    let program = write_shim(&s.bin, "pip3", "pip3", &s.log, "exit 0");
    let (outcome, reason) = run_owner_action(&action(&program, &Pip));
    assert_eq!(outcome, Outcome::Applied, "{reason}");
    assert_eq!(read_log(&s.log), vec!["pip3\tcache\tpurge".to_owned()]);
}

#[test]
fn running_pip_is_refused_before_any_owner_query() {
    let s = sandbox();
    write_shim(&s.bin, "pip3", "pip3", &s.log, "echo 25.0");
    fake_process(&s.proc_root, 5150, "pip3", &["pip3", "install", "requests"]);
    let refusal = resolve(&Pip, &s.env(&s.bin)).expect_err("busy pip");
    assert_eq!(refusal.outcome, Outcome::Skipped, "{refusal:?}");
    assert!(
        read_log(&s.log).is_empty(),
        "busy owner must not be queried"
    );
}

// --- conda -----------------------------------------------------------------

fn conda_info(pkgs_dirs: &[&str]) -> String {
    let items: Vec<String> = pkgs_dirs.iter().map(|dir| format!("\"{dir}\"")).collect();
    format!(
        "{{\"platform\":\"linux-64\",\"pkgs_dirs\":[{}],\"envs\":[]}}",
        items.join(",")
    )
}

#[test]
fn conda_names_every_package_root_and_previews_the_first() {
    let s = sandbox();
    let first = s.home.join(".conda/pkgs");
    let second = s.home.join("miniconda3/pkgs");
    let info = conda_info(&[first.to_str().unwrap(), second.to_str().unwrap()]);
    write_shim(
        &s.bin,
        "conda",
        "conda",
        &s.log,
        &format!(
            "case \"$*\" in '--version') echo 'conda 25.1.0';; 'info --json') echo '{info}';; esac"
        ),
    );
    let resolved = s.first(&Conda).expect("conda resolves");
    assert_eq!(resolved.target, first);
    assert_eq!(resolved.guarded, vec![first, second]);
}

#[test]
fn conda_malformed_info_is_unavailable_and_runs_no_clean() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "conda",
        "conda",
        &s.log,
        "case \"$*\" in '--version') echo 25.1.0;; 'info --json') echo '{';; esac",
    );
    let refusal = s.first(&Conda).expect_err("malformed");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(read_log(&s.log).iter().all(|call| !call.contains("clean")));
}

#[test]
fn conda_without_package_roots_is_unavailable() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "conda",
        "conda",
        &s.log,
        "case \"$*\" in '--version') echo 25.1.0;; 'info --json') echo '{\"pkgs_dirs\":[]}';; esac",
    );
    let refusal = s.first(&Conda).expect_err("no roots");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn conda_relative_or_non_string_root_is_refused() {
    let s = sandbox();
    write_shim(
        &s.bin,
        "conda",
        "conda",
        &s.log,
        "case \"$*\" in '--version') echo 25.1.0;; 'info --json') echo '{\"pkgs_dirs\":[\"/first\",\"relative\"]}';; esac",
    );
    let refusal = s.first(&Conda).expect_err("relative root");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");

    write_shim(
        &s.bin,
        "conda",
        "conda",
        &s.log,
        "case \"$*\" in '--version') echo 25.1.0;; 'info --json') echo '{\"pkgs_dirs\":[\"/first\",1]}';; esac",
    );
    let refusal = s.first(&Conda).expect_err("non-string root");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
}

#[test]
fn conda_clean_action_runs_index_tarball_and_log_clean() {
    let s = sandbox();
    let program = write_shim(&s.bin, "conda", "conda", &s.log, "exit 0");
    let (outcome, reason) = run_owner_action(&action(&program, &Conda));
    assert_eq!(outcome, Outcome::Applied, "{reason}");
    assert_eq!(
        read_log(&s.log),
        vec!["conda\tclean\t--yes\t--index-cache\t--tarballs\t--logfiles".to_owned()]
    );
}

#[test]
fn missing_conda_is_unavailable_and_its_package_root_survives() {
    let s = sandbox();
    let root = s.home.join(".conda/pkgs");
    fs::create_dir_all(&root).expect("pkgs root");
    fs::write(root.join("pkg.tar.bz2"), b"cached").expect("entry");
    fs::create_dir_all(&s.bin).expect("empty PATH dir");
    let refusal = resolve(&Conda, &s.env(&s.bin)).expect_err("missing conda");
    assert_eq!(refusal.outcome, Outcome::Unavailable, "{refusal:?}");
    assert!(
        root.join("pkg.tar.bz2").exists(),
        "missing owner must not delete"
    );
}
