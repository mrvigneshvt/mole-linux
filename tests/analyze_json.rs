use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

use mole_linux::scan::{EntryKind, Node, NodeStatus, ScanReport};

#[test]
fn analyze_json_emits_scan_model_and_preserves_non_utf8_names() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("scan-root");
    fs::create_dir(&root).expect("create root");
    let raw_name = b"bad-\xff-name";
    fs::write(root.join(OsStr::from_bytes(raw_name)), b"data").expect("create non-UTF-8 file");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["analyze", "--json"])
        .arg(&root)
        .output()
        .expect("run analyze --json");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(document["scan_status"], "complete");
    assert_eq!(document["path"], root.to_string_lossy().as_ref());
    assert_eq!(document["overview"], false);
    assert_eq!(document["total_size"], 4);
    let entry = document["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|entry| entry["name_bytes"] == serde_json::json!(raw_name))
        .expect("entry with raw filename bytes");
    assert_eq!(entry["name"], "bad-�-name");
    assert_eq!(entry["scan_status"], "complete");
    assert_eq!(entry["size"], 4);
}

#[test]
fn analyze_json_keeps_partial_and_unavailable_states() {
    let root = Node {
        name: "root".into(),
        kind: EntryKind::Directory,
        apparent: 9,
        allocated: 9,
        children: vec![Node {
            name: "locked".into(),
            kind: EntryKind::Directory,
            apparent: 0,
            allocated: 0,
            children: Vec::new(),
            status: NodeStatus::Unavailable("permission denied".to_owned()),
        }],
        status: NodeStatus::Partial,
    };
    let json =
        mole_linux::scan::report_json(&ScanReport { root }, std::path::Path::new("/tmp/root"))
            .expect("serialize report");
    let document: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    assert_eq!(document["scan_status"], "partial");
    assert_eq!(document["entries"][0]["scan_status"], "unavailable");
}
