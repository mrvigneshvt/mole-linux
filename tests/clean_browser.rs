use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::proc::ProcessState;
use mole_linux::core::sink::SinkContext;
use mole_linux::core::whitelist::Whitelist;
use mole_linux::rules::clean::{CleanRule, RuleContext, catalog};

fn setup(
    home: &Path,
    process_state: ProcessState,
) -> (XdgPaths, Whitelist, impl Fn(&str) -> ProcessState) {
    fs::create_dir_all(home).expect("create temporary HOME");
    let xdg = XdgPaths::from_values(home.to_path_buf(), None, None, None, None).expect("XDG paths");
    let whitelist = Whitelist::load(&xdg).expect("load whitelist");
    (xdg, whitelist, move |_| process_state)
}

#[test]
fn browser_rules_offer_only_cache_directories_and_firefox_profile_cache2() {
    let temp = tempfile::tempdir().expect("temporary root");
    let home = temp.path().join("home");
    for relative in [
        ".cache/chromium/Default/Cache",
        ".cache/google-chrome/Default/Cache",
        ".cache/BraveSoftware/Brave-Browser/Default/Cache",
        ".cache/microsoft-edge/Default/Cache",
        ".cache/vivaldi/Default/Cache",
        ".cache/mozilla/firefox/abc.default-release/cache2/entries",
        ".mozilla/firefox/abc.default-release/cookies.sqlite",
    ] {
        let path = home.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create parent");
        fs::write(&path, b"browser fixture").expect("write fixture");
    }
    let (xdg, whitelist, process_probe) = setup(&home, ProcessState::NotRunning);
    let cancel = AtomicBool::new(false);
    let context = RuleContext {
        home: &home,
        xdg: &xdg,
        whitelist: &whitelist,
        proc_probe: &process_probe,
        now: SystemTime::now(),
        cancel: &cancel,
    };
    let rules = catalog();
    let browser = rules
        .iter()
        .find(|rule| rule.id() == "browser-cache")
        .expect("browser rule");
    let discovery = browser.discover(&context);
    let paths = discovery
        .candidates
        .iter()
        .map(|candidate| {
            candidate
                .path
                .strip_prefix(&home)
                .expect("inside HOME")
                .to_path_buf()
        })
        .collect::<Vec<_>>();

    assert!(paths.contains(&Path::new(".cache/chromium").to_path_buf()));
    assert!(paths.contains(&Path::new(".cache/google-chrome").to_path_buf()));
    assert!(paths.contains(&Path::new(".cache/BraveSoftware/Brave-Browser").to_path_buf()));
    assert!(paths.contains(&Path::new(".cache/microsoft-edge").to_path_buf()));
    assert!(paths.contains(&Path::new(".cache/vivaldi").to_path_buf()));
    assert!(
        paths.contains(
            &Path::new(".cache/mozilla/firefox/abc.default-release/cache2").to_path_buf()
        )
    );
    assert!(
        !paths
            .iter()
            .any(|path| path.to_string_lossy().contains("cookies.sqlite"))
    );
    assert!(!paths.iter().any(|path| path.starts_with(".mozilla")));
}

#[test]
fn browser_running_or_unknown_keeps_cache_and_unknown_is_reported() {
    for (state, expected_unavailable, expected_skipped) in
        [(ProcessState::Running, 0, 1), (ProcessState::Unknown, 1, 0)]
    {
        let temp = tempfile::tempdir().expect("temporary root");
        let home = temp.path().join("home");
        let target = home.join(".cache/chromium");
        fs::create_dir_all(&target).expect("create cache");
        fs::write(target.join("payload"), b"keep while running or unknown").expect("seed cache");
        let (xdg, whitelist, process_probe) = setup(&home, state);
        let cancel = AtomicBool::new(false);
        let context = RuleContext {
            home: &home,
            xdg: &xdg,
            whitelist: &whitelist,
            proc_probe: &process_probe,
            now: SystemTime::now(),
            cancel: &cancel,
        };
        let rules = catalog();
        let browser = rules
            .iter()
            .find(|rule| rule.id() == "browser-cache")
            .expect("browser rule");
        let rule_refs: [&dyn CleanRule; 1] = [browser.as_ref()];
        let mut session = OpLog::with_state_home(&temp.path().join("state"))
            .start_session("clean", false)
            .expect("start session");
        let sink = SinkContext::new(xdg.clone(), whitelist.clone());
        let report = mole_linux::commands::clean::execute_rules(
            &mole_linux::commands::clean::CleanArgs {
                yes: true,
                ..Default::default()
            },
            &rule_refs,
            &context,
            &sink,
            &mut session,
        )
        .expect("guard cache cleanup");
        assert_eq!(report.unavailable, expected_unavailable);
        assert_eq!(report.skipped, expected_skipped);
        assert!(target.join("payload").exists());
    }
}
