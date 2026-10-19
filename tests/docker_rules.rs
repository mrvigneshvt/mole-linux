// M5.5 contracts for Docker (upstream lib/clean/dev.sh:clean_dev_docker, Linux adaptation):
// `docker` is an advisory row that reads `docker system df` and removes nothing. The build
// cache under ~/.docker/buildx/cache is offered only when its own id is selected explicitly;
// a category selection or the default run never removes it. `docker` is a shim that records argv.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use mole_linux::commands::clean::{self, CleanArgs, CleanSummary};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{CleanRule, Discovery, DiscoveryStatus, RuleContext};
use mole_linux::rules::docker;

const DF_JSON: &str = concat!(
    r#"{"Active":"1","Reclaimable":"4.074GB (77%)","Size":"5.28GB","TotalCount":"10","Type":"Images"}"#,
    "\n",
    r#"{"Active":"0","Reclaimable":"1.784GB (100%)","Size":"1.784GB","TotalCount":"3","Type":"Local Volumes"}"#,
    "\n",
    r#"{"Active":"0","Reclaimable":"1.41GB","Size":"2.008GB","TotalCount":"75","Type":"Build Cache"}"#,
);

/// Serialises shim tests: a freshly written shim can fail to exec while a sibling fork holds it.
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Sandbox {
    _serial: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    sudo: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

fn sandbox() -> Sandbox {
    let serial = serial();
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    let sudo_dir = root.join("sudo-bin");
    fs::create_dir_all(&sudo_dir).expect("sudo dir");
    Sandbox {
        _serial: serial,
        bin: root.join("usr-bin"),
        sudo: sudo_dir.join("sudo"),
        state: root.join("state"),
        log: root.join("calls.log"),
        home,
        _temp: temp,
    }
}

/// Writes an executable `docker` shim that logs argv, then runs `body`.
fn docker_shim(s: &Sandbox, body: &str) {
    fs::create_dir_all(&s.bin).expect("bin dir");
    let script = format!(
        "#!/bin/sh\nprintf 'docker' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = s.log.display(),
    );
    let path = s.bin.join("docker");
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

/// A `docker` that answers `system df` with the given JSON lines and fails any other command.
fn df_shim(s: &Sandbox, json: &str) {
    let lines: String = json
        .lines()
        .map(|line| format!("printf '%s\\n' '{line}'\n"))
        .collect();
    let body = format!(
        "case \"$*\" in\n  'system df --format {{{{json .}}}}')\n{lines};;\n  *) exit 9 ;;\nesac"
    );
    docker_shim(s, &body);
}

fn read_log(s: &Sandbox) -> Vec<String> {
    fs::read_to_string(&s.log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn owner(s: &Sandbox) -> OwnerRunner {
    OwnerRunner::new(s.bin.clone(), s.sudo.clone(), Consent::Unasked)
}

fn never_running(_: &str) -> ProcessState {
    ProcessState::NotRunning
}

fn discover_advisory(s: &Sandbox) -> Discovery {
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rule = docker::registered()
        .into_iter()
        .find(|rule| rule.id() == "docker")
        .expect("docker advisory rule");
    rule.discover_with_owner(&ctx, &owner(s))
}

fn buildx_cache(s: &Sandbox) -> PathBuf {
    let dir = s.home.join(".docker/buildx/cache/abc123");
    fs::create_dir_all(&dir).expect("buildx cache");
    fs::write(dir.join("blob"), vec![b'x'; 8192]).expect("blob");
    dir
}

/// Runs the given rule ids (all of the docker rules when empty) through the shared runner.
fn run(s: &Sandbox, args: CleanArgs) -> CleanSummary {
    let rules = docker::registered();
    let rule_refs: Vec<&dyn CleanRule> = rules.iter().map(Box::as_ref).collect();
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = never_running;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
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
    let mut runner = owner(s);
    let prompts = RefCell::new(0_usize);
    let prompt = |_: &[Vec<OsString>]| {
        *prompts.borrow_mut() += 1;
        true
    };
    clean::execute_rules_with_owner(
        &args,
        &rule_refs,
        &ctx,
        &sink,
        &mut session,
        &mut runner,
        &prompt,
    )
    .expect("clean run")
}

// --- advisory report ---------------------------------------------------------

#[test]
fn advisory_reports_each_type_from_docker_system_df_and_offers_no_candidate() {
    let s = sandbox();
    df_shim(&s, DF_JSON);

    let discovery = discover_advisory(&s);

    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    let DiscoveryStatus::Advisory(reason) = &discovery.status else {
        panic!("expected advisory, got {:?}", discovery.status);
    };
    assert!(
        reason.contains("Build Cache: 2.008GB total, 1.41GB reclaimable"),
        "{reason}"
    );
    assert!(
        reason.contains("Images: 5.28GB total, 4.074GB (77%) reclaimable"),
        "{reason}"
    );
    assert!(reason.contains("advisory"), "{reason}");
}

#[test]
fn advisory_only_ever_runs_the_read_only_df_query() {
    let s = sandbox();
    df_shim(&s, DF_JSON);

    let _ = discover_advisory(&s);

    assert_eq!(
        read_log(&s),
        ["docker\tsystem\tdf\t--format\t{{json .}}".to_owned()]
    );
}

#[test]
fn unreachable_daemon_is_unavailable_never_an_empty_report() {
    let s = sandbox();
    docker_shim(
        &s,
        "echo 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock' >&2; exit 1",
    );

    let discovery = discover_advisory(&s);

    let DiscoveryStatus::Unavailable(reason) = &discovery.status else {
        panic!("expected unavailable, got {:?}", discovery.status);
    };
    assert!(reason.contains("Cannot connect"), "{reason}");
}

#[test]
fn missing_docker_is_unavailable_with_a_reason() {
    let s = sandbox();
    fs::create_dir_all(&s.bin).expect("empty bin");

    let discovery = discover_advisory(&s);

    assert!(matches!(discovery.status, DiscoveryStatus::Unavailable(_)));
    assert!(discovery.candidates.is_empty());
}

#[test]
fn unparseable_df_output_is_unavailable_not_silently_empty() {
    let s = sandbox();
    docker_shim(
        &s,
        "echo 'TYPE TOTAL ACTIVE SIZE RECLAIMABLE'; echo 'Images 10 1 5GB'",
    );

    let discovery = discover_advisory(&s);

    let DiscoveryStatus::Unavailable(reason) = &discovery.status else {
        panic!("expected unavailable, got {:?}", discovery.status);
    };
    assert!(reason.contains("docker system df"), "{reason}");
}

// --- build cache ---------------------------------------------------------------

#[test]
fn build_cache_is_kept_by_category_selection_and_by_the_default_run() {
    let s = sandbox();
    let cache = buildx_cache(&s);
    docker_shim(&s, "exit 9");
    let rules = docker::registered();
    assert!(
        rules.iter().any(|rule| rule.id() == "docker-build-cache"),
        "docker-build-cache must be registered"
    );

    let by_category = run(
        &s,
        CleanArgs {
            yes: true,
            categories: Some("development".to_owned()),
            ..CleanArgs::default()
        },
    );

    assert_eq!(by_category.applied, 0, "{by_category:?}");
    assert!(cache.exists(), "category selection removed the build cache");
}

#[test]
fn build_cache_is_removed_only_when_its_own_id_is_selected() {
    let s = sandbox();
    let cache = buildx_cache(&s);
    docker_shim(&s, "exit 9");

    let by_development = run(
        &s,
        CleanArgs {
            yes: true,
            select: Some("development".to_owned()),
            ..CleanArgs::default()
        },
    );
    assert_eq!(by_development.applied, 0, "{by_development:?}");
    assert!(
        cache.exists(),
        "--select development removed the build cache"
    );

    let by_id = run(
        &s,
        CleanArgs {
            yes: true,
            select: Some("docker-build-cache".to_owned()),
            ..CleanArgs::default()
        },
    );
    assert_eq!(by_id.applied, 1, "{by_id:?}");
    assert!(!cache.exists());
}

#[test]
fn selecting_the_build_cache_never_runs_a_docker_command() {
    let s = sandbox();
    buildx_cache(&s);
    docker_shim(&s, "exit 9");

    let _ = run(
        &s,
        CleanArgs {
            yes: true,
            select: Some("docker-build-cache".to_owned()),
            ..CleanArgs::default()
        },
    );

    // Only the advisory row queries docker, and --select docker-build-cache excludes it.
    assert!(read_log(&s).is_empty(), "{:?}", read_log(&s));
}

#[test]
fn the_advisory_row_is_counted_as_attention_in_a_development_run() {
    let s = sandbox();
    df_shim(&s, DF_JSON);

    let summary = run(
        &s,
        CleanArgs {
            dry_run: true,
            categories: Some("development".to_owned()),
            ..CleanArgs::default()
        },
    );

    assert_eq!(summary.attention, 1, "{summary:?}");
    assert_eq!(summary.applied, 0, "{summary:?}");
}
