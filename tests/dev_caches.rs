// M5.5 contracts: the misc-language and tooling caches from upstream lib/clean/dev.sh that have a
// Linux cache path. Each rule removes only the children of its named cache roots, through the
// sink, and leaves config, installs and credentials beside them alone. Everything runs in a
// temporary HOME; nothing touches the real one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::commands::clean::{self, CleanArgs};
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{
    Candidate, Category, CleanRule, DiscoveryStatus, RecoveryMode, RuleContext,
};
use mole_linux::rules::dev_caches;

struct Sandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    state: PathBuf,
}

fn sandbox() -> Sandbox {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    Sandbox {
        state: root.join("state"),
        home,
        _temp: temp,
    }
}

fn file(path: &Path) {
    fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
    fs::write(path, b"cache entry").expect("write entry");
}

fn rule(id: &str) -> Box<dyn CleanRule> {
    dev_caches::registered()
        .into_iter()
        .find(|rule| rule.id() == id)
        .unwrap_or_else(|| panic!("no rule {id}"))
}

fn discover(s: &Sandbox, id: &str) -> (Vec<PathBuf>, DiscoveryStatus) {
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let discovery = rule(id).discover(&ctx);
    let mut paths: Vec<PathBuf> = discovery
        .candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    paths.sort();
    (paths, discovery.status)
}

fn run_select(s: &Sandbox, select: &str) -> clean::CleanSummary {
    let rules = dev_caches::registered();
    let rule_refs: Vec<&dyn CleanRule> = rules.iter().map(Box::as_ref).collect();
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
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
        .start_session("clean", false)
        .expect("session");
    let args = CleanArgs {
        yes: true,
        select: Some(select.to_owned()),
        ..CleanArgs::default()
    };
    clean::execute_rules(&args, &rule_refs, &ctx, &sink, &mut session).expect("runner")
}

fn candidates_of(s: &Sandbox, id: &str) -> Vec<Candidate> {
    let xdg = XdgPaths::from_values(s.home.clone(), None, None, None, None).expect("xdg");
    let whitelist = Whitelist::load(&xdg).expect("whitelist");
    let probe = |_: &str| ProcessState::NotRunning;
    let cancel = AtomicBool::new(false);
    let ctx = RuleContext {
        home: &s.home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    rule(id).discover(&ctx).candidates
}

#[test]
fn registered_rules_have_the_upstream_ids_in_development() {
    let rules = dev_caches::registered();
    let mut ids: Vec<&str> = rules.iter().map(|rule| rule.id()).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "cicd-caches",
            "elixir",
            "network-caches",
            "ocaml",
            "perl",
            "ruby",
            "shell-caches",
        ]
    );
    assert!(
        rules
            .iter()
            .all(|rule| rule.category() == Category::Development)
    );
}

#[test]
fn ruby_offers_children_of_each_upstream_root_and_only_gem_files_in_versioned_caches() {
    let s = sandbox();
    let home = &s.home;
    file(&home.join(".rbenv/cache/ruby-3.3.0.tar.gz"));
    file(&home.join(".gem/specs/index.gz"));
    file(&home.join(".gem/ruby/3.3.0/cache/rake-13.gem"));
    file(&home.join(".gem/ruby/3.3.0/cache/notes.txt"));
    file(&home.join(".bundle/cache/rails-7.gem"));

    let (paths, status) = discover(&s, "ruby");

    assert_eq!(status, DiscoveryStatus::Complete);
    assert_eq!(
        paths,
        [
            home.join(".bundle/cache/rails-7.gem"),
            home.join(".gem/ruby/3.3.0/cache/rake-13.gem"),
            home.join(".gem/specs/index.gz"),
            home.join(".rbenv/cache/ruby-3.3.0.tar.gz"),
        ]
    );
}

#[test]
fn ruby_candidates_are_regenerable_and_selected_by_default() {
    let s = sandbox();
    file(&s.home.join(".rbenv/cache/ruby-3.3.0.tar.gz"));

    let candidates = candidates_of(&s, "ruby");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].recovery, RecoveryMode::Regenerate);
    assert!(candidates[0].selected_by_default);
    assert!(
        candidates[0].reason.contains("rbenv"),
        "{}",
        candidates[0].reason
    );
}

#[test]
fn perl_offers_only_the_cpan_build_tree_and_keeps_the_distribution_store() {
    let s = sandbox();
    file(&s.home.join(".cpan/build/Some-Dist-1.0-abc/Makefile"));
    file(
        &s.home
            .join(".cpan/sources/authors/id/A/AB/ABC/Some-Dist-1.0.tar.gz"),
    );

    let (paths, status) = discover(&s, "perl");

    assert_eq!(status, DiscoveryStatus::Complete);
    assert_eq!(paths, [s.home.join(".cpan/build/Some-Dist-1.0-abc")]);
}

#[test]
fn elixir_and_ocaml_offer_their_download_caches_children() {
    let s = sandbox();
    file(&s.home.join(".hex/cache/hex-package.tar"));
    file(&s.home.join(".opam/download-cache/archive.tar.gz"));
    file(&s.home.join(".opam/config"));

    assert_eq!(
        discover(&s, "elixir").0,
        [s.home.join(".hex/cache/hex-package.tar")]
    );
    assert_eq!(
        discover(&s, "ocaml").0,
        [s.home.join(".opam/download-cache/archive.tar.gz")]
    );
}

#[test]
fn shell_caches_leave_oh_my_zsh_config_and_history_alone() {
    let s = sandbox();
    file(&s.home.join(".oh-my-zsh/cache/completions-zsh"));
    file(&s.home.join(".oh-my-zsh/custom/aliases.zsh"));
    file(&s.home.join(".cache/pre-commit/repo-1234"));
    file(&s.home.join(".zsh_history"));

    let (paths, status) = discover(&s, "shell-caches");

    assert_eq!(status, DiscoveryStatus::Complete);
    assert_eq!(
        paths,
        [
            s.home.join(".cache/pre-commit/repo-1234"),
            s.home.join(".oh-my-zsh/cache/completions-zsh"),
        ]
    );
}

#[test]
fn network_caches_offer_curl_and_wget_children_only() {
    let s = sandbox();
    file(&s.home.join(".cache/curl/hsts-cache"));
    file(&s.home.join(".cache/wget/index.html"));
    file(&s.home.join(".curlrc"));
    file(&s.home.join(".wgetrc"));

    let (paths, status) = discover(&s, "network-caches");

    assert_eq!(status, DiscoveryStatus::Complete);
    assert_eq!(
        paths,
        [
            s.home.join(".cache/curl/hsts-cache"),
            s.home.join(".cache/wget/index.html"),
        ]
    );
}

#[test]
fn cicd_caches_offer_named_cache_children_and_never_the_scanner_install() {
    let s = sandbox();
    file(&s.home.join(".cache/terraform/providers/plugin.zip"));
    file(&s.home.join(".cache/gitlab-runner/cache-1"));
    file(&s.home.join(".grafana/cache/plugin-data"));
    file(&s.home.join(".circleci/cache/orb.json"));
    file(&s.home.join(".sonar/cache/analysis.bin"));
    file(&s.home.join(".sonar/native-sonar-scanner/bin/sonar-scanner"));
    file(&s.home.join(".grafana/data/grafana.db"));

    let (paths, status) = discover(&s, "cicd-caches");

    assert_eq!(status, DiscoveryStatus::Complete);
    assert_eq!(
        paths,
        [
            s.home.join(".cache/gitlab-runner/cache-1"),
            s.home.join(".cache/terraform/providers"),
            s.home.join(".circleci/cache/orb.json"),
            s.home.join(".grafana/cache/plugin-data"),
            s.home.join(".sonar/cache/analysis.bin"),
        ]
    );
}

#[test]
fn missing_cache_roots_are_empty_and_complete() {
    let s = sandbox();

    for id in [
        "ruby",
        "perl",
        "elixir",
        "ocaml",
        "shell-caches",
        "network-caches",
        "cicd-caches",
    ] {
        let (paths, status) = discover(&s, id);
        assert!(paths.is_empty(), "{id}: {paths:?}");
        assert_eq!(status, DiscoveryStatus::Complete, "{id}");
    }
}

#[test]
fn applying_one_rule_removes_its_children_and_nothing_else() {
    let s = sandbox();
    let kept = s.home.join(".gem/specs-keep.txt");
    file(&s.home.join(".rbenv/cache/ruby-3.3.0.tar.gz"));
    file(&s.home.join(".bundle/cache/rails-7.gem"));
    file(&kept);

    let summary = run_select(&s, "ruby");

    assert_eq!(summary.discovered, 2);
    assert_eq!(summary.applied, 2, "{summary:?}");
    assert!(!s.home.join(".rbenv/cache/ruby-3.3.0.tar.gz").exists());
    assert!(!s.home.join(".bundle/cache/rails-7.gem").exists());
    assert!(kept.exists());
}

#[test]
fn hidden_entries_are_left_alone_as_a_shell_glob_would_leave_them() {
    // Written after the glob-matching fix, to pin the upstream `dir/*` behavior.
    let s = sandbox();
    file(&s.home.join(".rbenv/cache/ruby-3.3.0.tar.gz"));
    file(&s.home.join(".rbenv/cache/.lock"));

    let (paths, status) = discover(&s, "ruby");

    assert_eq!(status, DiscoveryStatus::Complete);
    assert_eq!(paths, [s.home.join(".rbenv/cache/ruby-3.3.0.tar.gz")]);
}
