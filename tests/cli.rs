use std::os::unix::fs::symlink;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(args)
        .output()
        .expect("run mole-linux")
}

#[test]
fn unsupported_subcommands_parse_flags_and_exit_two() {
    let cases: &[(&[&str], u32)] = &[
        (
            &[
                "purge",
                "--dry-run",
                "--paths",
                "target",
                "--include-empty",
                "--yes",
            ],
            31,
        ),
        (
            &[
                "installer",
                "--dry-run",
                "--max-depth",
                "2",
                "--hidden",
                "--type",
                "zip",
            ],
            33,
        ),
        (
            &[
                "uninstall",
                "--dry-run",
                "--list",
                "--permanent",
                "--whitelist",
            ],
            34,
        ),
        (&["optimize", "--dry-run", "--whitelist"], 39),
    ];

    for (args, issue) in cases {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("not implemented yet (#{issue})")),
            "args: {args:?}, expected issue #{issue}, stderr: {stderr}"
        );
    }
}

#[test]
fn completion_generates_bash_zsh_and_fish_scripts_to_stdout() {
    for (shell, expected) in [
        ("bash", "_mole__linux"),
        ("zsh", "#compdef mole-linux"),
        ("fish", "complete -c mole-linux"),
    ] {
        let output = run(&["completion", shell]);
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(expected),
            "{shell}"
        );
    }
}

#[test]
fn completion_install_dry_run_previews_and_remove_clears_only_marked_block() {
    let home = tempdir().expect("temp home");
    let rc = home.path().join(".bashrc");
    std::fs::write(&rc, "export KEEP=yes\n").expect("seed rc");

    let preview = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion", "--dry-run"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("run preview");
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview_text = String::from_utf8_lossy(&preview.stdout);
    assert_eq!(
        preview_text,
        format!(
            "Would update {:?}\n+# >>> mole-linux completion >>>\n+eval \"$(mole-linux completion bash 2>/dev/null)\"\n+# <<< mole-linux completion <<<\n",
            rc
        )
    );
    assert_eq!(
        std::fs::read_to_string(&rc).expect("rc unchanged"),
        "export KEEP=yes\n"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("run install");
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    let installed = std::fs::read_to_string(&rc).expect("installed rc");
    assert!(installed.contains("# >>> mole-linux completion >>>"));
    assert!(installed.contains("export KEEP=yes"));

    let remove_preview = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion", "--remove", "--dry-run"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("preview removal");
    assert!(remove_preview.status.success());
    assert_eq!(
        String::from_utf8_lossy(&remove_preview.stdout),
        format!(
            "Would update {:?}\n-# >>> mole-linux completion >>>\n-eval \"$(mole-linux completion bash 2>/dev/null)\"\n-# <<< mole-linux completion <<<\n",
            rc
        )
    );
    assert_eq!(
        std::fs::read_to_string(&rc).expect("rc after preview"),
        installed
    );

    let remove = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion", "--remove"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("run remove");
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&rc).expect("rc after remove"),
        "export KEEP=yes\n"
    );
}

#[test]
fn completion_fish_closed_pipe_exits_cleanly() {
    let home = tempdir().expect("temp home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion", "fish"])
        .env("HOME", home.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn completion");
    drop(child.stdout.take());
    let status = child.wait().expect("wait for completion");
    assert!(status.success(), "closed stdout should be clean: {status}");
}

#[test]
fn completion_stdout_write_errors_are_reported() {
    let home = tempdir().expect("temp home");
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open full device");
    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion", "fish"])
        .env("HOME", home.path())
        .stdout(Stdio::from(full))
        .output()
        .expect("run completion");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("failed to write completion to stdout")
    );
}

#[test]
fn completion_follows_rc_symlinks_and_refuses_broken_or_cyclic_chains() {
    let home = tempdir().expect("temp home");
    let dotfiles = home.path().join("dotfiles");
    std::fs::create_dir(&dotfiles).expect("create dotfiles");
    let target = dotfiles.join("bashrc");
    std::fs::write(&target, "export KEEP=yes\n").expect("seed target");
    symlink("bashrc", dotfiles.join("current-bashrc")).expect("link target");
    let rc = home.path().join(".bashrc");
    symlink("dotfiles/current-bashrc", &rc).expect("link rc");

    let installed = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("install completion");
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(rc.is_symlink(), "rc symlink must remain intact");
    assert!(
        std::fs::read_to_string(&target)
            .expect("target contents")
            .contains("# >>> mole-linux completion >>>")
    );

    let broken_home = tempdir().expect("broken link home");
    symlink(
        broken_home.path().join("missing"),
        broken_home.path().join(".bashrc"),
    )
    .expect("create broken link");
    let broken = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion"])
        .env("HOME", broken_home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("run broken link case");
    assert!(!broken.status.success());
    assert!(String::from_utf8_lossy(&broken.stderr).contains("broken symbolic link"));

    let cyclic_home = tempdir().expect("cyclic link home");
    symlink(
        cyclic_home.path().join("second"),
        cyclic_home.path().join(".bashrc"),
    )
    .expect("create first cycle link");
    symlink(
        cyclic_home.path().join(".bashrc"),
        cyclic_home.path().join("second"),
    )
    .expect("create second cycle link");
    let cyclic = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["completion"])
        .env("HOME", cyclic_home.path())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("run cyclic link case");
    assert!(!cyclic.status.success());
    assert!(String::from_utf8_lossy(&cyclic.stderr).contains("cyclic symbolic link"));
}

#[test]
fn status_json_emits_a_snapshot() {
    let output = run(&["status", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid status JSON");
    assert!(value.get("cpu").is_some());
    assert!(value.get("memory").is_some());
    assert!(value.get("disk_io").is_some());
    assert!(value.get("processes").is_some());
}

#[test]
fn analyze_json_emits_a_document_instead_of_the_unimplemented_error() {
    let dir = tempdir().expect("temporary scan root");
    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["analyze", "--json"])
        .arg(dir.path())
        .output()
        .expect("run analyze JSON");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(document["scan_status"], "complete");
}

#[test]
fn global_debug_is_accepted_before_and_after_subcommand() {
    for args in [&["--debug", "clean"][..], &["clean", "--debug"][..]] {
        let home = tempdir().expect("temporary HOME");
        let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
            .args(args)
            .env("HOME", home.path())
            .env("XDG_STATE_HOME", home.path().join("state"))
            .output()
            .expect("run clean");
        assert_eq!(output.status.code(), Some(0), "args: {args:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("Clean complete"));
    }
}

#[test]
fn help_matches_cli_snapshot() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert_eq!(stdout, include_str!("fixtures/cli-help.txt"));
}
