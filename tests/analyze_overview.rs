use std::fs;
use std::process::Command;
use tempfile::tempdir;

use mole_linux::scan::{self, OverviewRoots};

#[test]
fn pathless_analyze_json_measures_injected_overview_roots_and_keeps_missing_unavailable() {
    let home = tempdir().expect("temporary home");
    let cache = home.path().join("xdg-cache");
    let data = home.path().join("xdg-data");
    let state = home.path().join("xdg-state");
    fs::create_dir_all(&cache).expect("create cache");
    fs::create_dir_all(data.join("Trash/files")).expect("create trash");
    fs::create_dir_all(&state).expect("create state");
    fs::write(cache.join("payload"), b"cache-data").expect("seed cache");
    fs::write(home.path().join("home-payload"), b"home-data").expect("seed home");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["analyze", "--json"])
        .env("HOME", home.path())
        .env("XDG_CACHE_HOME", &cache)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_STATE_HOME", &state)
        .output()
        .expect("run pathless analyze --json");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(document["overview"], true);
    assert_eq!(document["path"], "/");
    let entries = document["entries"].as_array().expect("overview entries");
    for expected in [
        "Home",
        "XDG cache",
        "XDG data",
        "XDG state",
        "Trash",
        "Pacman package cache",
        "Journal",
        "Docker",
    ] {
        assert!(
            entries.iter().any(|entry| entry["name"] == expected),
            "missing {expected}: {entries:?}"
        );
    }
    let cache_row = entries
        .iter()
        .find(|entry| entry["name"] == "XDG cache")
        .expect("cache row");
    assert_eq!(cache_row["path"], cache.to_string_lossy().as_ref());
    assert_eq!(cache_row["scan_status"], "complete");
    assert!(cache_row["apparent_bytes"].as_u64().unwrap_or_default() > 0);
    let docker = entries
        .iter()
        .find(|entry| entry["name"] == "Docker")
        .expect("Docker row");
    assert!(matches!(
        docker["scan_status"].as_str(),
        Some("complete" | "partial" | "unavailable")
    ));
}

#[test]
fn injected_missing_overview_root_is_retained_as_unavailable() {
    let fixture = tempdir().expect("fixture root");
    let home = fixture.path().join("home");
    let cache = fixture.path().join("cache");
    let data = fixture.path().join("data");
    let state = fixture.path().join("state");
    for path in [&home, &cache, &data, &state] {
        fs::create_dir_all(path).expect("create overview fixture");
    }
    let missing_docker = fixture.path().join("missing-docker");
    let roots = OverviewRoots {
        home,
        cache,
        data: data.clone(),
        state,
        trash: data.join("Trash/files"),
        pacman_cache: fixture.path().join("missing-pacman"),
        journal: fixture.path().join("missing-journal"),
        docker: missing_docker,
    };

    let report = scan::scan_overview(&roots, &std::sync::atomic::AtomicBool::new(false));
    let json = scan::overview_json(&report).expect("serialize overview");
    let document: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let entries = document["entries"].as_array().expect("overview entries");
    for name in ["Pacman package cache", "Journal", "Docker"] {
        let row = entries
            .iter()
            .find(|entry| entry["name"] == name)
            .expect("missing row retained");
        assert_eq!(row["scan_status"], "unavailable");
    }
}

#[test]
fn overview_rows_preserve_cleanable_hints_from_scanned_roots() {
    let fixture = tempdir().expect("fixture root");
    let home = fixture.path().join("node_modules");
    let cache = fixture.path().join("cache");
    let data = fixture.path().join("data");
    let state = fixture.path().join("state");
    for path in [&home, &cache, &data, &state] {
        fs::create_dir_all(path).expect("create overview fixture");
    }
    let roots = OverviewRoots {
        home,
        cache,
        data: data.clone(),
        state,
        trash: data.join("Trash/files"),
        pacman_cache: fixture.path().join("missing-pacman"),
        journal: fixture.path().join("missing-journal"),
        docker: fixture.path().join("missing-docker"),
    };

    let report = scan::scan_overview(&roots, &std::sync::atomic::AtomicBool::new(false));
    let home_row = report
        .root
        .children
        .iter()
        .find(|row| row.name == "Home")
        .expect("Home overview row");
    assert!(home_row.cleanable);
    let json = scan::overview_json(&report).expect("serialize overview");
    let document: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let home_json = document["entries"]
        .as_array()
        .expect("overview entries")
        .iter()
        .find(|row| row["name"] == "Home")
        .expect("Home JSON row");
    assert_eq!(home_json["cleanable"], true);
}

#[test]
fn progressive_overview_publishes_scanning_rows_before_root_measurements() {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::Duration;

    let fixture = tempdir().expect("fixture root");
    let home = fixture.path().join("home");
    let cache = fixture.path().join("cache");
    let data = fixture.path().join("data");
    let state = fixture.path().join("state");
    for path in [&home, &cache, &data, &state] {
        fs::create_dir_all(path).expect("create overview fixture");
    }
    let roots = OverviewRoots {
        home,
        cache,
        data: data.clone(),
        state,
        trash: data.join("Trash/files"),
        pacman_cache: fixture.path().join("missing-pacman"),
        journal: fixture.path().join("missing-journal"),
        docker: fixture.path().join("missing-docker"),
    };
    let cancel = AtomicBool::new(false);
    let (events_tx, events_rx) = mpsc::channel();

    let worker = std::thread::spawn(move || {
        scan::scan_overview_progressive(&roots, &cancel, events_tx);
    });

    let initial = events_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("initial overview arrives before measurement");
    let scan::ScanEvent::Initial(root) = initial else {
        panic!("first overview event is Initial");
    };
    assert_eq!(root.children.len(), 8);
    assert!(
        root.children
            .iter()
            .all(|row| row.status == scan::NodeStatus::Scanning)
    );
    drop(events_rx);
    worker
        .join()
        .expect("overview worker exits after receiver closes");
}

#[test]
fn progressive_overview_publishes_each_root_result() {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::Duration;

    let fixture = tempdir().expect("fixture root");
    let home = fixture.path().join("node_modules");
    let cache = fixture.path().join("cache");
    let data = fixture.path().join("data");
    let state = fixture.path().join("state");
    for path in [&home, &cache, &data, &state] {
        fs::create_dir_all(path).expect("create overview fixture");
    }
    let roots = OverviewRoots {
        home,
        cache,
        data: data.clone(),
        state,
        trash: data.join("Trash/files"),
        pacman_cache: fixture.path().join("missing-pacman"),
        journal: fixture.path().join("missing-journal"),
        docker: fixture.path().join("missing-docker"),
    };
    let (events_tx, events_rx) = mpsc::channel();
    let cancel = AtomicBool::new(false);
    let worker = std::thread::spawn(move || {
        scan::scan_overview_progressive(&roots, &cancel, events_tx);
    });

    let initial = events_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("initial overview arrives immediately");
    assert!(matches!(initial, scan::ScanEvent::Initial(_)));
    let mut updated = 0_usize;
    loop {
        match events_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("overview root completes")
        {
            scan::ScanEvent::ChildUpdated(row) => {
                assert_ne!(row.status, scan::NodeStatus::Scanning);
                updated = updated.saturating_add(1);
            }
            scan::ScanEvent::Complete(report) => {
                assert_eq!(updated, 8);
                assert!(
                    report
                        .root
                        .children
                        .iter()
                        .all(|row| row.status != scan::NodeStatus::Scanning)
                );
                assert!(
                    report
                        .root
                        .children
                        .iter()
                        .find(|row| row.name == "Home")
                        .is_some_and(|row| row.cleanable)
                );
                break;
            }
            scan::ScanEvent::Initial(_) => panic!("overview publishes one Initial event"),
        }
    }
    worker.join().expect("overview scan finishes");
}
