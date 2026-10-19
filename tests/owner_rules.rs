// Runner-level contract for the owner-tool rules: discovery, selection, owner
// gating, and apply all go through the shared clean runner (M5.1 round 1).
// Owner tools are argv-recording shims; the HOME is a temporary directory.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::commands::clean::{self, CleanArgs, CleanSummary};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{Category, CleanRule, DiscoveryStatus, RuleContext, catalog};
use mole_linux::rules::owner_cache::{OwnerRuleEnv, registered_with};

struct Sandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    proc_root: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

impl Sandbox {
    fn owner_env(&self) -> OwnerRuleEnv {
        OwnerRuleEnv {
            path: self.bin.clone().into_os_string(),
            proc_root: self.proc_root.clone(),
            ..OwnerRuleEnv::default()
        }
    }
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

/// Writes a shim. `$HOME` in `body` is replaced by the sandbox home, because the child inherits
/// the host HOME and the shim must report sandbox paths only.
fn write_shim(home: &Path, dir: &Path, file_name: &str, label: &str, log: &Path, body: &str) {
    let body = body.replace("$HOME", &home.display().to_string());
    fs::create_dir_all(dir).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{label}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = log.display(),
    );
    let path = dir.join(file_name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

/// Seeds a cache directory with one file, so the runner has something to size.
fn seed_cache(path: &Path) {
    fs::create_dir_all(path).expect("cache dir");
    fs::write(path.join("entry"), vec![b'x'; 4096]).expect("cache entry");
}

fn calls(s: &Sandbox) -> Vec<String> {
    fs::read_to_string(&s.log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Calls made by one program: each recorded line starts with the program's name.
fn calls_of(s: &Sandbox, program: &str) -> Vec<String> {
    calls(s)
        .into_iter()
        .filter(|c| c.split('\t').next() == Some(program))
        .collect()
}

fn owner_calls(s: &Sandbox, word: &str) -> Vec<String> {
    calls(s).into_iter().filter(|c| c.contains(word)).collect()
}

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

/// Runs the owner rules through the shared runner with the given arguments.
fn run(s: &Sandbox, args: CleanArgs) -> CleanSummary {
    let rules = registered_with(s.owner_env());
    let rule_refs: Vec<&dyn CleanRule> = rules.iter().map(Box::as_ref).collect();
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
    clean::execute_rules(&args, &rule_refs, &context, &sink, &mut session).expect("runner")
}

fn select(names: &str) -> CleanArgs {
    CleanArgs {
        yes: true,
        select: Some(names.to_owned()),
        ..CleanArgs::default()
    }
}

#[test]
fn catalog_registers_the_four_js_owner_rules_in_development() {
    let rules = catalog();
    for id in ["npm", "pnpm", "bun", "corepack"] {
        let rule = rules
            .iter()
            .find(|rule| rule.id() == id)
            .unwrap_or_else(|| panic!("rule {id} not registered"));
        assert_eq!(rule.category(), Category::Development, "{id}");
    }
}

#[test]
fn dry_run_reports_the_owner_cache_and_runs_no_owner_command() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; esac",
    );
    let summary = run(
        &s,
        CleanArgs {
            dry_run: true,
            select: Some("npm".to_owned()),
            ..CleanArgs::default()
        },
    );
    assert_eq!(summary.discovered, 1, "{summary:?}");
    assert_eq!(summary.unchanged, 1, "{summary:?}");
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(owner_calls(&s, "clean").is_empty(), "{:?}", calls(&s));
}

#[test]
fn apply_runs_the_owner_clean_once_and_keeps_the_runner_accounting() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; esac",
    );
    let summary = run(&s, select("npm"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert_eq!(
        owner_calls(&s, "clean"),
        vec!["npm\tcache\tclean\t--force".to_owned()]
    );
    // The owner reports no freed bytes, so none are summed into the applied total.
    assert_eq!(summary.allocated_applied, Some(0), "{summary:?}");
    let oplog = fs::read_to_string(s.state.join("mole-linux/operations.log")).expect("oplog");
    assert!(oplog.contains("CLEAN applied"), "{oplog}");
}

#[test]
fn select_runs_only_the_named_owner_rule() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    seed_cache(&s.home.join(".local/share/pnpm/store/v11"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; esac",
    );
    write_shim(
        &s.home,
        &s.bin,
        "pnpm",
        "pnpm",
        &s.log,
        "case \"$*\" in '--version') echo 11;; 'store path') echo \"$HOME/.local/share/pnpm/store/v11\";; esac",
    );
    let summary = run(&s, select("pnpm"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert!(calls_of(&s, "npm").is_empty(), "{:?}", calls(&s));
    assert_eq!(owner_calls(&s, "store\tprune").len(), 1, "{:?}", calls(&s));
}

#[test]
fn running_owner_is_gated_by_the_runner_and_never_cleaned() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; esac",
    );
    fake_process(&s.proc_root, 4242, &["npm", "install"]);
    let summary = run(&s, select("npm"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(summary.unavailable, 1, "{summary:?}");
    assert!(owner_calls(&s, "clean").is_empty());
}

#[test]
fn unknown_process_state_is_unavailable_and_never_cleaned() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; esac",
    );
    fs::remove_dir_all(&s.proc_root).expect("drop proc root");
    let summary = run(&s, select("npm"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(summary.unavailable, 1, "{summary:?}");
    assert!(owner_calls(&s, "clean").is_empty());
}

#[test]
fn missing_bun_is_unavailable_and_nothing_else_runs() {
    let s = sandbox();
    fs::create_dir_all(&s.bin).expect("empty PATH");
    let summary = run(&s, select("bun"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(summary.unavailable, 1, "{summary:?}");
}

#[test]
fn bun_query_failure_never_runs_the_owner_rm() {
    let s = sandbox();
    seed_cache(&s.home.join(".bun/install/cache"));
    write_shim(
        &s.home,
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in 'pm cache') exit 1;; esac",
    );
    let _ = run(&s, select("bun"));
    assert!(owner_calls(&s, "rm").is_empty(), "{:?}", calls(&s));
}

#[test]
fn bun_reported_cache_is_sized_and_cleared_with_its_own_rm() {
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    seed_cache(&cache);
    write_shim(
        &s.home,
        &s.bin,
        "bun",
        "bun",
        &s.log,
        &format!(
            "case \"$*\" in 'pm cache') echo '{}';; esac",
            cache.display()
        ),
    );
    let summary = run(
        &s,
        CleanArgs {
            dry_run: true,
            select: Some("bun".to_owned()),
            ..CleanArgs::default()
        },
    );
    assert_eq!(summary.discovered, 1, "{summary:?}");
    assert!(owner_calls(&s, "rm").is_empty());
}

#[test]
fn failing_owner_clean_counts_as_failed_not_applied() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; 'cache clean --force') exit 3;; esac",
    );
    let summary = run(&s, select("npm"));
    assert_eq!(summary.failed, 1, "{summary:?}");
    assert_eq!(summary.applied, 0, "{summary:?}");
}

#[test]
fn two_pnpm_binaries_on_one_store_prune_it_once() {
    let s = sandbox();
    let store = s.home.join(".local/share/pnpm/store/v11");
    seed_cache(&store);
    let body = format!(
        "case \"$*\" in '--version') echo 11;; 'store path') echo '{}';; esac",
        store.display()
    );
    write_shim(&s.home, &s.bin, "pnpm", "pnpm", &s.log, &body);
    let mise = s.home.join(".local/share/mise/installs/pnpm/11.17.0");
    write_shim(&s.home, &mise, "pnpm", "pnpm", &s.log, &body);
    let summary = run(&s, select("pnpm"));
    assert_eq!(summary.discovered, 1, "{summary:?}");
    assert_eq!(owner_calls(&s, "store\tprune").len(), 1, "{:?}", calls(&s));
}

#[test]
fn missing_corepack_is_unavailable_and_its_cache_survives() {
    let s = sandbox();
    let cache = s.home.join(".cache/node/corepack");
    seed_cache(&cache);
    fs::create_dir_all(&s.bin).expect("empty PATH");
    let summary = run(&s, select("corepack"));
    assert_eq!(summary.unavailable, 1, "{summary:?}");
    assert!(
        cache.join("entry").exists(),
        "missing owner must not delete"
    );
}

#[test]
fn owner_gating_is_rechecked_before_the_owner_command() {
    let s = sandbox();
    seed_cache(&s.home.join(".npm"));
    write_shim(
        &s.home,
        &s.bin,
        "npm",
        "npm",
        &s.log,
        "case \"$*\" in 'config get cache') echo \"$HOME/.npm\";; esac",
    );
    let rules = registered_with(s.owner_env());
    let rule = rules
        .iter()
        .find(|rule| rule.id() == "npm")
        .expect("npm rule");
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
    let discovery = rule.discover(&context);
    assert!(
        matches!(discovery.status, DiscoveryStatus::Complete),
        "{discovery:?}"
    );
    let candidate = discovery.candidates.first().expect("npm candidate").clone();

    // npm starts between discovery and the owner command: the final guard must refuse it.
    fake_process(&s.proc_root, 5150, &["npm", "ci"]);
    assert!(rule.final_guard(&candidate, &context).is_err());
}

/// Discovers one owner rule directly, for checks on what it offers before any apply.
fn discover_with(s: &Sandbox, env: OwnerRuleEnv, id: &str) -> mole_linux::rules::clean::Discovery {
    let rules = registered_with(env);
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

#[test]
fn bun_that_cannot_run_falls_back_to_its_cache_dir_sent_to_trash() {
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    seed_cache(&cache);
    write_shim(
        &s.home,
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in 'pm cache') exit 1;; esac",
    );
    let summary = run(&s, select("bun"));
    assert_eq!(summary.applied, 1, "{summary:?}");
    assert!(
        !cache.exists(),
        "the fallback must move the cache out of place"
    );
    let trashed = fs::read_dir(s.home.join(".local/share/Trash/files"))
        .expect("trash files dir")
        .count();
    assert_eq!(trashed, 1, "the cache must be in Trash, not deleted");
    assert!(
        calls_of(&s, "bun").iter().all(|c| !c.contains("rm")),
        "{:?}",
        calls(&s)
    );
}

#[test]
fn bun_fallback_dry_run_selects_without_moving_anything() {
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    seed_cache(&cache);
    write_shim(
        &s.home,
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in 'pm cache') exit 1;; esac",
    );
    let summary = run(
        &s,
        CleanArgs {
            dry_run: true,
            select: Some("bun".to_owned()),
            ..CleanArgs::default()
        },
    );
    assert_eq!(summary.unchanged, 1, "{summary:?}");
    assert!(cache.join("entry").exists());
}

#[test]
fn bun_fallback_is_not_offered_while_bun_runs() {
    let s = sandbox();
    let cache = s.home.join(".bun/install/cache");
    seed_cache(&cache);
    write_shim(
        &s.home,
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in 'pm cache') exit 1;; esac",
    );
    fake_process(&s.proc_root, 9001, &["bun", "install"]);
    let summary = run(&s, select("bun"));
    assert_eq!(summary.applied, 0, "{summary:?}");
    assert!(
        cache.join("entry").exists(),
        "a running bun's cache must not move"
    );
}

#[test]
fn bun_fallback_dir_follows_bun_install_cache_dir_then_bun_install() {
    let s = sandbox();
    write_shim(
        &s.home,
        &s.bin,
        "bun",
        "bun",
        &s.log,
        "case \"$*\" in 'pm cache') exit 1;; esac",
    );
    let custom = s.home.join("custom-bun-cache");
    seed_cache(&custom);

    let mut env = s.owner_env();
    env.bun_install_cache_dir = Some(custom.clone().into_os_string());
    env.bun_install = Some(s.home.join("bunroot").into_os_string());
    let discovery = discover_with(&s, env, "bun");
    assert_eq!(discovery.candidates[0].path, custom, "{discovery:?}");

    let mut env = s.owner_env();
    env.bun_install = Some(s.home.join("bunroot").into_os_string());
    let discovery = discover_with(&s, env, "bun");
    assert!(
        discovery.candidates.is_empty(),
        "no bunroot cache yet: {discovery:?}"
    );

    let mut env = s.owner_env();
    env.bun_install_cache_dir = Some("relative/cache".into());
    env.bun_install = Some(s.home.join("bunroot").into_os_string());
    seed_cache(&s.home.join("bunroot/install/cache"));
    let discovery = discover_with(&s, env, "bun");
    assert_eq!(
        discovery.candidates[0].path,
        s.home.join("bunroot/install/cache"),
        "a relative BUN_INSTALL_CACHE_DIR is ignored: {discovery:?}"
    );
}
