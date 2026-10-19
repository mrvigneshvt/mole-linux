// Ported contracts: tw93/Mole lib/clean/dev.sh:clean_dev_python, clean_pyinstaller_bincache,
// clean_conda_metadata_caches and tests/clean_dev_caches.bats (PyInstaller, conda whitelist,
// uv and pip3 owner cases) at 8c55e8c (GPL-3.0). HOME and every owner tool are temporary.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::commands::clean::{self, CleanArgs, CleanSummary};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::{ProcessState, is_process_name_running};
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Category, CleanRule, Discovery, DiscoveryStatus, OwnerCheck, RecoveryMode, RuleContext, catalog,
};
use mole_linux::rules::owner_cache::{OwnerRuleEnv, registered_with};
use mole_linux::rules::python;

/// One registered Python rule by id.
fn rule(id: &str) -> Box<dyn CleanRule> {
    python::registered()
        .into_iter()
        .find(|rule| rule.id() == id)
        .unwrap_or_else(|| panic!("rule {id} not registered"))
}

struct Sandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    proc_root: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

fn sandbox() -> Sandbox {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    let proc_root = root.join("proc");
    fs::create_dir_all(&proc_root).expect("proc");
    Sandbox {
        bin: root.join("bin"),
        state: root.join("state"),
        log: root.join("calls.log"),
        proc_root,
        home,
        _temp: temp,
    }
}

/// Writes an argv-recording shim. `$HOME` in `body` becomes the sandbox HOME.
fn write_shim(s: &Sandbox, file_name: &str, label: &str, body: &str) {
    let body = body.replace("$HOME", &s.home.display().to_string());
    fs::create_dir_all(&s.bin).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{label}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = s.log.display(),
    );
    let path = s.bin.join(file_name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

fn calls(s: &Sandbox) -> Vec<String> {
    fs::read_to_string(&s.log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A fake `/proc` entry whose `comm` is `name`, as the kernel reports an executable's basename.
fn fake_comm(proc_root: &Path, pid: u32, name: &str) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("fake process");
    fs::write(dir.join("comm"), format!("{name}\n")).expect("comm");
}

fn xdg(s: &Sandbox) -> XdgPaths {
    XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg")
}

fn whitelist_patterns(s: &Sandbox, patterns: &[&str]) -> Whitelist {
    let config = s.home.join(".config/mole-linux");
    fs::create_dir_all(&config).expect("config dir");
    fs::write(config.join("whitelist"), patterns.join("\n")).expect("whitelist");
    Whitelist::load(&xdg(s)).expect("whitelist loads")
}

/// Discovery of one rule, with process state read from `proc_root`.
fn discover(s: &Sandbox, rule: &dyn CleanRule, whitelist: &Whitelist) -> Discovery {
    let xdg = xdg(s);
    let proc_root = s.proc_root.clone();
    let probe = move |name: &str| is_process_name_running(&proc_root, name);
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    rule.discover(&context)
}

/// Runs the given rules through the shared runner, with process state read from `proc_root`.
fn run(s: &Sandbox, rules: Vec<Box<dyn CleanRule>>, args: CleanArgs) -> CleanSummary {
    let rule_refs: Vec<&dyn CleanRule> = rules.iter().map(Box::as_ref).collect();
    let xdg = xdg(s);
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let proc_root = s.proc_root.clone();
    let probe = move |name: &str| is_process_name_running(&proc_root, name);
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let sink = SinkContext::new(xdg.clone(), whitelist.clone());
    let mut session = OpLog::with_state_home(&s.state)
        .start_session("clean", args.dry_run)
        .expect("session");
    clean::execute_rules(&args, &rule_refs, &context, &sink, &mut session).expect("runner")
}

fn select(names: &str) -> CleanArgs {
    CleanArgs {
        yes: true,
        select: Some(names.to_owned()),
        ..CleanArgs::default()
    }
}

fn names_of(discovery: &Discovery) -> Vec<String> {
    discovery
        .candidates
        .iter()
        .map(|candidate| {
            candidate
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

// --- registration ----------------------------------------------------------

#[test]
fn catalog_registers_the_python_rules_in_development() {
    let rules = catalog();
    for id in [
        "uv",
        "pip",
        "conda",
        "pyinstaller",
        "pyenv-cache",
        "poetry-cache",
        "ruff-cache",
        "mypy-cache",
        "pytest-cache",
    ] {
        let rule = rules
            .iter()
            .find(|rule| rule.id() == id)
            .unwrap_or_else(|| panic!("rule {id} not registered"));
        assert_eq!(rule.category(), Category::Development, "{id}");
    }
}

// --- uv and pip3 through the runner ------------------------------------------

#[test]
fn uv_apply_runs_cache_prune_once_and_nothing_else() {
    let s = sandbox();
    let cache = s.home.join(".cache/uv");
    fs::create_dir_all(&cache).expect("uv cache");
    fs::write(cache.join("entry"), vec![b'x'; 4096]).expect("entry");
    write_shim(
        &s,
        "uv",
        "uv",
        &format!(
            "case \"$*\" in '--version') echo 0.9.0;; 'cache dir') echo '{}';; esac",
            cache.display()
        ),
    );
    let env = OwnerRuleEnv {
        path: s.bin.clone().into_os_string(),
        proc_root: s.proc_root.clone(),
        ..OwnerRuleEnv::default()
    };
    let rules = registered_with(env);
    let summary = run(&s, rules, select("uv"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    let prunes: Vec<String> = calls(&s)
        .into_iter()
        .filter(|c| c.contains("prune"))
        .collect();
    assert_eq!(prunes, vec!["uv\tcache\tprune".to_owned()]);
    assert!(
        cache.join("entry").exists(),
        "the owner, not the runner, removes entries"
    );
}

// --- conda whitelist protection ---------------------------------------------

#[test]
fn whitelisting_a_second_conda_package_root_refuses_the_owner_clean() {
    let s = sandbox();
    let first = s.home.join(".conda/pkgs");
    let second = s.home.join("custom-cache");
    fs::create_dir_all(&first).expect("first root");
    fs::create_dir_all(&second).expect("second root");
    write_shim(
        &s,
        "conda",
        "conda",
        &format!(
            "case \"$*\" in '--version') echo 25.1.0;; 'info --json') printf '{{\"pkgs_dirs\":[\"%s\",\"%s\"]}}\\n' '{}' '{}';; esac",
            first.display(),
            second.display()
        ),
    );
    let env = OwnerRuleEnv {
        path: s.bin.clone().into_os_string(),
        proc_root: s.proc_root.clone(),
        ..OwnerRuleEnv::default()
    };
    let rule = registered_with(env)
        .into_iter()
        .find(|rule| rule.id() == "conda")
        .expect("conda rule");
    let whitelist = whitelist_patterns(&s, &[second.to_str().unwrap()]);
    let discovery = discover(&s, rule.as_ref(), &whitelist);
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(
        matches!(discovery.status, DiscoveryStatus::Partial(ref reason) if reason.contains("whitelist")),
        "{discovery:?}"
    );
    assert!(calls(&s).iter().all(|call| !call.contains("clean")));
}

// --- PyInstaller bincache --------------------------------------------------

fn pyinstaller_root(s: &Sandbox) -> PathBuf {
    s.home.join(".local/share/pyinstaller")
}

#[test]
fn pyinstaller_offers_only_bincache_children_and_keeps_state() {
    let s = sandbox();
    let root = pyinstaller_root(&s);
    fs::create_dir_all(root.join("bincache00_py311")).expect("bincache");
    fs::create_dir_all(root.join("hooks")).expect("hooks");
    fs::write(root.join("config.json"), b"state").expect("config");
    let discovery = discover(
        &s,
        rule("pyinstaller").as_ref(),
        &whitelist_patterns(&s, &[]),
    );
    assert_eq!(
        names_of(&discovery),
        vec!["bincache00_py311".to_owned()],
        "{discovery:?}"
    );
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.recovery, RecoveryMode::Regenerate);
    assert_eq!(
        candidate.owner_check,
        OwnerCheck::ProcessName("pyinstaller".to_owned())
    );
}

#[test]
fn pyinstaller_skips_a_bincache_that_is_a_symlink() {
    let s = sandbox();
    let root = pyinstaller_root(&s);
    let outside = s.home.join("outside");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, root.join("bincache00_py312")).expect("symlink");
    let discovery = discover(
        &s,
        rule("pyinstaller").as_ref(),
        &whitelist_patterns(&s, &[]),
    );
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(outside.exists());
}

#[test]
fn pyinstaller_root_that_is_a_symlink_offers_nothing() {
    let s = sandbox();
    let outside = s.home.join("outside");
    fs::create_dir_all(outside.join("bincache00_py313")).expect("outside bincache");
    fs::create_dir_all(s.home.join(".local/share")).expect("data dir");
    std::os::unix::fs::symlink(&outside, pyinstaller_root(&s)).expect("root symlink");
    let discovery = discover(
        &s,
        rule("pyinstaller").as_ref(),
        &whitelist_patterns(&s, &[]),
    );
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(
        matches!(discovery.status, DiscoveryStatus::Partial(_)),
        "{discovery:?}"
    );
}

#[test]
fn running_pyinstaller_is_gated_and_its_bincache_survives_apply() {
    let s = sandbox();
    let root = pyinstaller_root(&s);
    fs::create_dir_all(root.join("bincache00_py311")).expect("bincache");
    fake_comm(&s.proc_root, 8080, "pyinstaller");
    let summary = run(&s, vec![rule("pyinstaller")], select("pyinstaller"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(
        root.join("bincache00_py311").exists(),
        "a running PyInstaller must keep its cache"
    );
}

#[test]
fn unknown_process_state_for_pyinstaller_is_not_treated_as_idle() {
    let s = sandbox();
    let root = pyinstaller_root(&s);
    fs::create_dir_all(root.join("bincache00_py314")).expect("bincache");
    let missing_proc = s.home.join("no-such-proc");
    let xdg = xdg(&s);
    let whitelist = whitelist_patterns(&s, &[]);
    let probe = move |name: &str| is_process_name_running(&missing_proc, name);
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let discovery = rule("pyinstaller").discover(&context);
    let candidate = discovery.candidates.first().expect("bincache offered");
    assert_eq!(
        candidate.owner_state,
        ProcessState::Unknown,
        "{candidate:?}"
    );
    assert!(!candidate.selected_by_default, "{candidate:?}");
}

#[test]
fn pyinstaller_apply_moves_the_bincache_to_trash_and_keeps_state() {
    let s = sandbox();
    let root = pyinstaller_root(&s);
    fs::create_dir_all(root.join("bincache00_py310")).expect("bincache");
    fs::write(root.join("bincache00_py310/module.bin"), b"cache").expect("payload");
    fs::write(root.join("config.json"), b"state").expect("config");
    let summary = run(&s, vec![rule("pyinstaller")], select("pyinstaller"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert!(!root.join("bincache00_py310").exists());
    assert!(root.join("config.json").exists());
    assert!(
        s.home
            .join(".local/share/Trash/files/bincache00_py310")
            .exists()
    );
}

// --- sink-backed tool caches (ruff, mypy, pytest, pyenv, poetry) -----------

#[test]
fn ruff_cache_children_are_offered_and_the_directory_stays() {
    let s = sandbox();
    let root = s.home.join(".cache/ruff");
    fs::create_dir_all(root.join("0.9.1")).expect("ruff entry");
    fs::create_dir_all(root.join("0.9.2")).expect("ruff entry");
    let discovery = discover(
        &s,
        rule("ruff-cache").as_ref(),
        &whitelist_patterns(&s, &[]),
    );
    let mut names = names_of(&discovery);
    names.sort();
    assert_eq!(
        names,
        vec!["0.9.1".to_owned(), "0.9.2".to_owned()],
        "{discovery:?}"
    );
    assert!(
        discovery
            .candidates
            .iter()
            .all(|c| c.recovery == RecoveryMode::Regenerate)
    );
}

#[test]
fn pytest_cache_apply_removes_children_through_the_sink() {
    let s = sandbox();
    let root = s.home.join(".pytest_cache");
    fs::create_dir_all(root.join("v/cache")).expect("pytest state");
    fs::write(root.join("README.md"), b"child file").expect("file");
    let rule = rule("pytest-cache");
    let summary = run(&s, vec![rule], select("pytest-cache"));
    assert_eq!(summary.applied, 2, "{summary:?}");
    assert!(!root.join("v").exists());
    assert!(!root.join("README.md").exists());
    assert!(root.exists(), "the tool recreates its cache directory");
}

#[test]
fn missing_tool_cache_offers_nothing_and_is_not_an_error() {
    let s = sandbox();
    let discovery = discover(
        &s,
        rule("mypy-cache").as_ref(),
        &whitelist_patterns(&s, &[]),
    );
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert_eq!(discovery.status, DiscoveryStatus::Complete, "{discovery:?}");
}
