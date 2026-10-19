// Contracts ported from tw93/Mole lib/clean/caches.sh (project cache logic) and
// tests/clean_system_caches.bats ("clean_project_caches ..." pycache cases), GPL-3.0.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime};

use mole_linux::commands::clean::{self, CleanArgs};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, RuleContext,
};
use mole_linux::rules::project_cache::PyCacheRule;

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    xdg: XdgPaths,
    whitelist: Whitelist,
    cancel: AtomicBool,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("create HOME");
    let xdg = XdgPaths::from_values(home.clone(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("load whitelist");
    Fixture {
        _temp: temp,
        home,
        xdg,
        whitelist,
        cancel: AtomicBool::new(false),
    }
}

fn with_context<T>(fx: &Fixture, body: impl FnOnce(&RuleContext<'_>) -> T) -> T {
    let probe = |_: &str| ProcessState::NotRunning;
    let context = RuleContext {
        home: &fx.home,
        xdg: &fx.xdg,
        whitelist: &fx.whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &fx.cancel,
    };
    body(&context)
}

fn discover(fx: &Fixture) -> Vec<Candidate> {
    with_context(fx, |context| {
        let discovery: Discovery = PyCacheRule.discover(context);
        assert_eq!(discovery.status, DiscoveryStatus::Complete);
        discovery.candidates
    })
}

fn discovered_paths(fx: &Fixture) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = discover(fx).into_iter().map(|c| c.path).collect();
    paths.sort();
    paths
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
    fs::write(path, bytes).expect("write fixture file");
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Every test keeps one untracked control cache so that a rule which finds
/// nothing fails on the control rather than passing vacuously.
fn control_cache(fx: &Fixture) -> PathBuf {
    let cache = fx.home.join("Projects/app/pkg/__pycache__");
    write(&cache.join("control.pyc"), b"control");
    cache
}

#[test]
fn discovers_pycache_holding_bytecode_under_a_project_root_with_sizes() {
    let fx = fixture();
    let cache = control_cache(&fx);

    let candidates = discover(&fx);

    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let candidate = &candidates[0];
    assert_eq!(candidate.path, cache);
    assert_eq!(candidate.apparent_size, Some(7));
    assert!(candidate.allocated_size.is_some());
    assert!(candidate.selected_by_default);
    assert_eq!(candidate.owner_state, ProcessState::NotRunning);
}

#[test]
fn ignores_pycache_without_direct_bytecode_files() {
    let fx = fixture();
    let cache = control_cache(&fx);
    write(
        &fx.home.join("Projects/app/empty/__pycache__/readme.txt"),
        b"no bytecode",
    );

    assert_eq!(discovered_paths(&fx), vec![cache]);
}

#[test]
fn ignores_pycache_outside_project_roots() {
    let fx = fixture();
    let cache = control_cache(&fx);
    write(&fx.home.join("scratch/__pycache__/stray.pyc"), b"stray");

    assert_eq!(discovered_paths(&fx), vec![cache]);
}

#[test]
fn prunes_dependency_and_virtualenv_trees() {
    let fx = fixture();
    let cache = control_cache(&fx);
    write(
        &fx.home
            .join("Projects/app/node_modules/dep/__pycache__/a.pyc"),
        b"a",
    );
    write(
        &fx.home.join("Projects/app/.venv/lib/__pycache__/b.pyc"),
        b"b",
    );
    write(
        &fx.home.join("Projects/app/venv/lib/__pycache__/c.pyc"),
        b"c",
    );

    assert_eq!(discovered_paths(&fx), vec![cache]);
}

#[test]
fn does_not_follow_a_symlinked_pycache_out_of_the_project() {
    let fx = fixture();
    let cache = control_cache(&fx);
    let outside = fx
        .home
        .parent()
        .expect("parent")
        .join("outside/__pycache__");
    write(&outside.join("victim.pyc"), b"outside");
    fs::create_dir_all(fx.home.join("Projects/app/linked")).expect("create link parent");
    std::os::unix::fs::symlink(&outside, fx.home.join("Projects/app/linked/__pycache__"))
        .expect("create symlinked cache");

    assert_eq!(discovered_paths(&fx), vec![cache]);
}

#[test]
fn keeps_pycache_that_git_tracks() {
    let fx = fixture();
    let app = fx.home.join("Projects/app");
    let committed = app.join("committed/__pycache__");
    write(&committed.join("kept.pyc"), b"tracked");
    git(&app, &["init", "-q"]);
    git(&app, &["add", "-f", "committed/__pycache__/kept.pyc"]);
    let control = app.join("pkg/__pycache__");
    write(&control.join("local.pyc"), b"untracked");

    assert_eq!(discovered_paths(&fx), vec![control]);
}

#[test]
fn keeps_pycache_holding_a_nested_repository() {
    let fx = fixture();
    let cache = control_cache(&fx);
    let vendored = fx.home.join("Projects/app/pkg/__pycache__/vendored");
    fs::create_dir_all(&vendored).expect("create vendored dir");
    git(&vendored, &["init", "-q"]);
    write(&fx.home.join("Projects/app/other/__pycache__/x.pyc"), b"x");
    let other = fx.home.join("Projects/app/other/__pycache__");

    let found = discovered_paths(&fx);

    assert!(!found.contains(&cache), "{found:?}");
    assert_eq!(found, vec![other]);
}

#[test]
fn final_guard_refuses_pycache_that_became_tracked_after_discovery() {
    let fx = fixture();
    let cache = control_cache(&fx);
    let candidates = discover(&fx);
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let app = fx.home.join("Projects/app");
    git(&app, &["init", "-q"]);
    git(&app, &["add", "-f", "pkg/__pycache__/control.pyc"]);

    let refusal = with_context(&fx, |context| {
        PyCacheRule.final_guard(&candidates[0], context)
    });

    let reason = refusal.expect_err("tracked cache must be refused at the final guard");
    assert!(reason.contains("tracked"), "{reason}");
    assert!(cache.join("control.pyc").exists());
}

#[test]
fn final_guard_allows_an_unchanged_cache() {
    let fx = fixture();
    let cache = control_cache(&fx);
    let candidates = discover(&fx);

    let verdict = with_context(&fx, |context| {
        PyCacheRule.final_guard(&candidates[0], context)
    });

    assert_eq!(verdict, Ok(()));
    assert_eq!(candidates[0].path, cache);
}

struct GuardRefusingRule(PyCacheRule);

impl CleanRule for GuardRefusingRule {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn category(&self) -> Category {
        self.0.category()
    }
    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        self.0.discover(ctx)
    }
    fn final_guard(&self, _candidate: &Candidate, _ctx: &RuleContext<'_>) -> Result<(), String> {
        Err("refused at final guard by test".to_owned())
    }
}

fn apply_rule(fx: &Fixture, rule: &dyn CleanRule) -> clean::CleanSummary {
    let rules: [&dyn CleanRule; 1] = [rule];
    let probe = |_: &str| ProcessState::NotRunning;
    let context = RuleContext {
        home: &fx.home,
        xdg: &fx.xdg,
        whitelist: &fx.whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &fx.cancel,
    };
    let state = fx._temp.path().join("state");
    let oplog = OpLog::with_state_home(&state);
    let mut session = oplog.start_session("clean", false).expect("start session");
    let sink = SinkContext::new(fx.xdg.clone(), fx.whitelist.clone());
    clean::execute_rules(
        &CleanArgs {
            yes: true,
            ..CleanArgs::default()
        },
        &rules,
        &context,
        &sink,
        &mut session,
    )
    .expect("execute rules")
}

#[test]
fn runner_applies_untracked_pycache_through_the_trash_sink() {
    let fx = fixture();
    let cache = control_cache(&fx);

    let summary = apply_rule(&fx, &PyCacheRule);

    assert_eq!(summary.applied, 1, "{summary:?}");
    assert!(!cache.exists());
    assert!(
        fx.home
            .join(".local/share/Trash/files/__pycache__")
            .exists()
    );
}

#[test]
fn runner_skips_a_candidate_whose_final_guard_refuses() {
    let fx = fixture();
    let cache = control_cache(&fx);

    let summary = apply_rule(&fx, &GuardRefusingRule(PyCacheRule));

    assert_eq!(summary.applied, 0, "{summary:?}");
    assert_eq!(summary.skipped, 1, "{summary:?}");
    assert!(cache.join("control.pyc").exists());
}

#[test]
fn untracked_cache_in_a_repository_is_clear_on_every_probe() {
    // Regression: the shared git probe raced its own reader thread and reported
    // "git probe reader disconnected" for about 40% of clear, untracked paths.
    let fx = fixture();
    let app = fx.home.join("Projects/app");
    let cache = app.join("pkg/__pycache__");
    write(&cache.join("local.pyc"), b"untracked");
    git(&app, &["init", "-q"]);

    for attempt in 0..50 {
        let verdict =
            mole_linux::purge::has_tracked_files(&cache, Instant::now() + Duration::from_secs(5));
        assert!(
            matches!(verdict, Ok(false)),
            "attempt {attempt}: expected Ok(false), got {verdict:?}"
        );
    }
}

#[test]
fn unanswerable_git_state_keeps_the_cache_and_reports_partial_discovery() {
    // Written after the Unknown/Kept split, not red-first: a `.git` file pointing
    // at a missing gitdir makes git fail, which must keep the cache and surface it.
    let fx = fixture();
    let broken = fx.home.join("Projects/broken");
    write(&broken.join("pkg/__pycache__/m.pyc"), b"x");
    fs::write(
        broken.join(".git"),
        b"gitdir: /nonexistent/mole-linux-test\n",
    )
    .expect("write .git");

    let discovery = with_context(&fx, |context| PyCacheRule.discover(context));

    assert!(
        discovery.candidates.is_empty(),
        "{:?}",
        discovery.candidates
    );
    match discovery.status {
        DiscoveryStatus::Partial(reason) => {
            assert!(reason.contains("git status unknown"), "{reason}")
        }
        other => panic!("expected Partial discovery, got {other:?}"),
    }
}
