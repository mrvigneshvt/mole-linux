// Ported contracts: tw93/Mole tests/clean_dev_caches.bats and tests/dev_extended.bats
// (Go, mise, Rust registry, Gradle cases), GPL-3.0. Owner tools are argv-recording
// shims and the HOME is a temporary directory; nothing touches the real host.
// Maven has no deletion path here: upstream removed it (lib/clean/dev.sh:clean_dev_jvm),
// so no rule id "maven" is registered.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::commands::clean::{self, CleanArgs, CleanSummary};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Category, CleanRule, Discovery, DiscoveryStatus, RuleContext, catalog,
};
use mole_linux::rules::owner_cache::{OwnerRuleEnv, registered_with as owner_rules};
use mole_linux::rules::toolchain_cache::registered_with as toolchain_rules;

struct Sandbox {
    _temp: tempfile::TempDir,
    root: PathBuf,
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
        root,
        _temp: temp,
    }
}

impl Sandbox {
    /// Owner rules see only the shim directory on PATH, the sandbox proc root, and no tool env.
    fn env(&self) -> OwnerRuleEnv {
        OwnerRuleEnv {
            path: self.bin.clone().into_os_string(),
            proc_root: self.proc_root.clone(),
            ..OwnerRuleEnv::default()
        }
    }

    /// Every rule this milestone adds, built from one environment.
    fn rules(&self, env: OwnerRuleEnv) -> Vec<Box<dyn CleanRule>> {
        let mut rules = owner_rules(env.clone());
        rules.extend(toolchain_rules(env));
        rules
    }
}

/// Writes a `sh` shim that logs its argv then runs `body`. `$HOME` in `body` is the sandbox home,
/// because the child inherits the host HOME.
fn write_shim(s: &Sandbox, file_name: &str, body: &str) {
    fs::create_dir_all(&s.bin).expect("shim dir");
    let body = body.replace("$HOME", &s.home.display().to_string());
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{file_name}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = s.log.display(),
    );
    let path = s.bin.join(file_name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

fn seed_file(path: &Path) {
    fs::create_dir_all(path.parent().expect("parent")).expect("seed parent");
    fs::write(path, vec![b'x'; 4096]).expect("seed file");
}

fn seed_cache(path: &Path) {
    seed_file(&path.join("entry"));
}

fn calls(s: &Sandbox) -> Vec<String> {
    fs::read_to_string(&s.log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn calls_of(s: &Sandbox, program: &str) -> Vec<String> {
    calls(s)
        .into_iter()
        .filter(|c| c.split('\t').next() == Some(program))
        .collect()
}

/// Writes a fake `/proc/<pid>/cmdline` with NUL-separated argv, as the kernel does.
fn fake_process(proc_root: &Path, pid: u32, argv: &[&str]) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("fake process");
    let mut bytes = Vec::new();
    for arg in argv {
        bytes.extend_from_slice(arg.as_bytes());
        bytes.push(0);
    }
    fs::write(dir.join("cmdline"), bytes).expect("cmdline");
}

fn run(s: &Sandbox, env: OwnerRuleEnv, args: CleanArgs) -> CleanSummary {
    let rules = s.rules(env);
    let refs: Vec<&dyn CleanRule> = rules.iter().map(Box::as_ref).collect();
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
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
    clean::execute_rules(&args, &refs, &context, &sink, &mut session).expect("runner")
}

fn discover(s: &Sandbox, env: OwnerRuleEnv, id: &str) -> Discovery {
    let rules = s.rules(env);
    let rule = rules.iter().find(|rule| rule.id() == id).expect("rule");
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    rule.discover(&context)
}

fn select(names: &str) -> CleanArgs {
    CleanArgs {
        yes: true,
        select: Some(names.to_owned()),
        ..CleanArgs::default()
    }
}

fn trash_entries(s: &Sandbox) -> usize {
    fs::read_dir(s.home.join(".local/share/Trash/files")).map_or(0, |entries| entries.count())
}

#[test]
fn catalog_registers_cargo_go_gradle_and_mise_in_development_without_maven() {
    let rules = catalog();
    for id in ["cargo", "go", "gradle", "mise"] {
        let rule = rules
            .iter()
            .find(|rule| rule.id() == id)
            .unwrap_or_else(|| panic!("rule {id} not registered"));
        assert_eq!(rule.category(), Category::Development, "{id}");
    }
    assert!(
        rules.iter().all(|rule| !rule.id().contains("maven")),
        "maven must have no deletion rule"
    );
}

#[test]
fn go_apply_runs_go_clean_cache_once_and_never_clears_the_module_cache() {
    let s = sandbox();
    seed_cache(&s.home.join(".cache/go-build"));
    seed_cache(&s.home.join("go/pkg/mod"));
    write_shim(
        &s,
        "go",
        "case \"$*\" in 'env GOCACHE') echo \"$HOME/.cache/go-build\" ;; 'clean -cache') rm -rf \"$HOME/.cache/go-build\" ;; esac",
    );
    let summary = run(&s, s.env(), select("go"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert_eq!(
        calls_of(&s, "go")
            .iter()
            .filter(|c| c.contains("clean"))
            .cloned()
            .collect::<Vec<_>>(),
        vec!["go\tclean\t-cache".to_owned()],
        "{:?}",
        calls(&s)
    );
    assert!(
        calls(&s).iter().all(|c| !c.contains("modcache")),
        "{:?}",
        calls(&s)
    );
    assert!(!s.home.join(".cache/go-build/entry").exists());
    assert!(
        s.home.join("go/pkg/mod/entry").exists(),
        "module cache must survive"
    );
}

#[test]
fn go_dry_run_selects_the_build_cache_and_runs_no_owner_clean() {
    let s = sandbox();
    seed_cache(&s.home.join(".cache/go-build"));
    write_shim(
        &s,
        "go",
        "case \"$*\" in 'env GOCACHE') echo \"$HOME/.cache/go-build\" ;; esac",
    );
    let summary = run(
        &s,
        s.env(),
        CleanArgs {
            dry_run: true,
            select: Some("go".to_owned()),
            ..CleanArgs::default()
        },
    );
    assert_eq!(summary.discovered, 1, "{summary:?}");
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(calls_of(&s, "go").iter().all(|c| !c.contains("clean")));
    assert!(s.home.join(".cache/go-build/entry").exists());
}

#[test]
fn running_go_does_not_block_the_concurrency_safe_build_cache() {
    let s = sandbox();
    seed_cache(&s.home.join(".cache/go-build"));
    write_shim(
        &s,
        "go",
        "case \"$*\" in 'env GOCACHE') echo \"$HOME/.cache/go-build\" ;; esac",
    );
    fake_process(&s.proc_root, 77, &["/usr/bin/go", "build", "./..."]);
    let summary = run(&s, s.env(), select("go"));
    assert_eq!(summary.applied, 1, "{summary:?}");
}

#[test]
fn missing_go_is_unavailable_and_its_cache_survives() {
    let s = sandbox();
    seed_cache(&s.home.join(".cache/go-build"));
    let summary = run(&s, s.env(), select("go"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(s.home.join(".cache/go-build/entry").exists());
}

#[test]
fn mise_honors_an_absolute_mise_cache_dir_without_querying_mise() {
    let s = sandbox();
    let custom = s.root.join("mise-cache");
    seed_cache(&custom);
    write_shim(&s, "mise", "exit 0");
    let env = OwnerRuleEnv {
        mise_cache_dir: Some(custom.clone().into_os_string()),
        ..s.env()
    };
    let found = discover(&s, env, "mise");
    assert_eq!(found.candidates.len(), 1, "{found:?}");
    assert_eq!(found.candidates[0].path, custom);
    assert!(
        calls_of(&s, "mise")
            .iter()
            .all(|c| !c.contains("cache\tpath")),
        "{:?}",
        calls(&s)
    );
}

#[test]
fn mise_apply_runs_mise_cache_clear_once_and_leaves_the_tree_to_the_owner() {
    let s = sandbox();
    seed_cache(&s.home.join(".cache/mise"));
    write_shim(
        &s,
        "mise",
        "case \"$*\" in 'cache path') echo \"$HOME/.cache/mise\" ;; 'cache clear') rm -rf \"$HOME/.cache/mise\" ;; esac",
    );
    let summary = run(&s, s.env(), select("mise"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert_eq!(
        calls_of(&s, "mise")
            .iter()
            .filter(|c| c.contains("cache\tclear"))
            .count(),
        1,
        "{:?}",
        calls(&s)
    );
    assert!(!s.home.join(".cache/mise/entry").exists());
}

#[test]
fn mise_that_cannot_run_falls_back_to_its_cache_dir_sent_to_trash() {
    let s = sandbox();
    seed_cache(&s.home.join(".cache/mise"));
    write_shim(&s, "mise", "case \"$*\" in --version) exit 1 ;; esac");
    let before = trash_entries(&s);
    let summary = run(&s, s.env(), select("mise"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert_eq!(trash_entries(&s), before + 1);
    assert!(!s.home.join(".cache/mise").exists());
    assert!(
        calls_of(&s, "mise")
            .iter()
            .all(|c| !c.contains("cache\tclear"))
    );
}

#[test]
fn cargo_honors_an_absolute_cargo_home_and_cleans_only_the_registry_cache() {
    let s = sandbox();
    let cargo_home = s.home.join("mise-cargo");
    seed_cache(&cargo_home.join("registry/cache/index.crates.io-1"));
    seed_file(&cargo_home.join("registry/src/index.crates.io-1/serde/src/lib.rs"));
    seed_file(&cargo_home.join("git/db/repo/HEAD"));
    seed_file(&cargo_home.join("bin/cargo-nextest"));
    seed_cache(&s.home.join(".cargo/registry/cache/untouched"));
    let env = OwnerRuleEnv {
        cargo_home: Some(cargo_home.clone().into_os_string()),
        ..s.env()
    };
    let found = discover(&s, env.clone(), "cargo");
    assert_eq!(found.candidates.len(), 1, "{found:?}");
    assert_eq!(
        found.candidates[0].path,
        cargo_home.join("registry/cache/index.crates.io-1")
    );

    let summary = run(&s, env, select("cargo"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert!(
        !cargo_home
            .join("registry/cache/index.crates.io-1/entry")
            .exists()
    );
    assert!(
        cargo_home
            .join("registry/src/index.crates.io-1/serde/src/lib.rs")
            .exists()
    );
    assert!(cargo_home.join("git/db/repo/HEAD").exists());
    assert!(cargo_home.join("bin/cargo-nextest").exists());
    assert!(
        s.home
            .join(".cargo/registry/cache/untouched/entry")
            .exists()
    );
}

#[test]
fn cargo_ignores_a_relative_or_traversing_cargo_home_and_uses_the_default() {
    let s = sandbox();
    seed_cache(&s.home.join(".cargo/registry/cache/index-default"));
    for hostile in ["relative/cargo", "/tmp/../etc/cargo"] {
        let env = OwnerRuleEnv {
            cargo_home: Some(OsString::from(hostile)),
            ..s.env()
        };
        let found = discover(&s, env, "cargo");
        assert_eq!(found.candidates.len(), 1, "{hostile}: {found:?}");
        assert_eq!(
            found.candidates[0].path,
            s.home.join(".cargo/registry/cache/index-default"),
            "{hostile}"
        );
    }
}

#[test]
fn cargo_registry_cache_that_escapes_cargo_home_is_refused_and_untouched() {
    let s = sandbox();
    let cargo_home = s.home.join("custom-cargo");
    let outside = s.root.join("outside-registry-cache");
    fs::create_dir_all(cargo_home.join("registry")).expect("registry");
    seed_cache(&outside.join("crate-data"));
    symlink(&outside, cargo_home.join("registry/cache")).expect("symlink");
    let env = OwnerRuleEnv {
        cargo_home: Some(cargo_home.into_os_string()),
        ..s.env()
    };
    let found = discover(&s, env.clone(), "cargo");
    assert!(found.candidates.is_empty(), "{found:?}");
    assert!(
        matches!(found.status, DiscoveryStatus::Unavailable(_)),
        "{found:?}"
    );
    let summary = run(&s, env, select("cargo"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(outside.join("crate-data/entry").exists());
}

#[test]
fn running_cargo_blocks_the_registry_cache_and_nothing_is_deleted() {
    let s = sandbox();
    seed_cache(&s.home.join(".cargo/registry/cache/index-1"));
    fake_process(
        &s.proc_root,
        91,
        &["/usr/bin/rustc", "--crate-name", "serde"],
    );
    let found = discover(&s, s.env(), "cargo");
    assert!(found.candidates.is_empty(), "{found:?}");
    let summary = run(&s, s.env(), select("cargo"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(s.home.join(".cargo/registry/cache/index-1/entry").exists());
}

#[test]
fn unknown_process_state_for_cargo_deletes_nothing() {
    let s = sandbox();
    seed_cache(&s.home.join(".cargo/registry/cache/index-1"));
    let env = OwnerRuleEnv {
        proc_root: s.root.join("no-such-proc"),
        ..s.env()
    };
    let found = discover(&s, env.clone(), "cargo");
    assert!(found.candidates.is_empty(), "{found:?}");
    let summary = run(&s, env, select("cargo"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(s.home.join(".cargo/registry/cache/index-1/entry").exists());
}

fn gradle_tree(s: &Sandbox) {
    seed_cache(&s.home.join(".gradle/caches/build-cache-1"));
    seed_file(&s.home.join(".gradle/notifications/8.14/log.txt"));
    seed_file(&s.home.join(".gradle/daemon/8.14/daemon.log"));
    seed_file(&s.home.join(".gradle/workers/worker.log"));
    seed_file(&s.home.join(".gradle/caches/modules-2/files-2.1/dep.jar"));
    seed_file(&s.home.join(".gradle/wrapper/dists/gradle-8.14/bin/gradle"));
}

#[test]
fn gradle_apply_cleans_build_cache_notifications_daemon_and_workers_only() {
    let s = sandbox();
    gradle_tree(&s);
    let summary = run(&s, s.env(), select("gradle"));
    assert!(summary.applied >= 1, "{summary:?}");
    assert!(!s.home.join(".gradle/caches/build-cache-1/entry").exists());
    assert!(!s.home.join(".gradle/notifications/8.14").exists());
    assert!(!s.home.join(".gradle/daemon/8.14").exists());
    assert!(!s.home.join(".gradle/workers/worker.log").exists());
    assert!(
        s.home
            .join(".gradle/caches/modules-2/files-2.1/dep.jar")
            .exists()
    );
    assert!(
        s.home
            .join(".gradle/wrapper/dists/gradle-8.14/bin/gradle")
            .exists()
    );
}

#[test]
fn running_gradle_daemon_defers_every_gradle_target() {
    let s = sandbox();
    gradle_tree(&s);
    fake_process(
        &s.proc_root,
        55,
        &[
            "java",
            "-cp",
            "gradle-launcher.jar",
            "org.gradle.launcher.daemon.bootstrap.GradleDaemon",
            "8.14",
        ],
    );
    let found = discover(&s, s.env(), "gradle");
    assert!(found.candidates.is_empty(), "{found:?}");
    let summary = run(&s, s.env(), select("gradle"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(s.home.join(".gradle/caches/build-cache-1/entry").exists());
    assert!(s.home.join(".gradle/workers/worker.log").exists());
}

#[test]
fn unknown_process_state_for_gradle_deletes_nothing() {
    let s = sandbox();
    gradle_tree(&s);
    let env = OwnerRuleEnv {
        proc_root: s.root.join("no-such-proc"),
        ..s.env()
    };
    let summary = run(&s, env, select("gradle"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(s.home.join(".gradle/caches/build-cache-1/entry").exists());
}

#[test]
fn symlinked_gradle_root_is_ignored_and_its_target_survives() {
    let s = sandbox();
    let outside = s.root.join("outside-gradle");
    seed_cache(&outside.join("caches/build-cache-1"));
    symlink(&outside, s.home.join(".gradle")).expect("symlink");
    let found = discover(&s, s.env(), "gradle");
    assert!(found.candidates.is_empty(), "{found:?}");
    let summary = run(&s, s.env(), select("gradle"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(outside.join("caches/build-cache-1/entry").exists());
}
