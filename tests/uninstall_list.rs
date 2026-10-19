use std::process::Output;

#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

fn run(args: &[&str]) -> Output {
    home_support::output(args)
}

#[test]
fn uninstall_list_prints_every_source_and_exits_zero() {
    let output = run(&["uninstall", "--list"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for title in [
        "pacman explicit",
        "AUR/foreign",
        "flatpak",
        "AppImage",
        "desktop entries",
    ] {
        assert!(stdout.contains(title), "missing {title}: {stdout}");
    }
    assert!(stdout.contains("nothing is removed"), "stdout: {stdout}");
}

fn tree(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            found.push(path);
        }
    }
    found.sort();
    found
}

#[test]
fn uninstall_list_never_writes_into_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_mole-linux"));
    command.args(["uninstall", "--list"]).isolated_home(&home);
    // isolated_home creates the XDG dirs; the snapshot is taken after that setup.
    let before = tree(&home);

    let output = command.output().expect("run uninstall --list");

    assert!(output.status.success());
    assert_eq!(
        tree(&home),
        before,
        "read-only inventory must not change HOME"
    );
}
