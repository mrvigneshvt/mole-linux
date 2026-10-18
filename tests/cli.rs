use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(args)
        .output()
        .expect("run mole-linux")
}

#[test]
fn unimplemented_subcommands_parse_flags_and_exit_two() {
    let cases: &[(&[&str], u32)] = &[
        (&["status", "--json", "--watch", "--interval", "2"], 37),
        (
            &[
                "clean",
                "--dry-run",
                "--categories",
                "cache",
                "--select",
                "one",
                "--exclude",
                "two",
                "--external",
                "--whitelist",
            ],
            16,
        ),
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
        (&["history", "--json", "--limit", "5"], 12),
        (&["completion", "bash", "--dry-run"], 41),
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
fn analyze_json_is_explicitly_unimplemented() {
    let output = run(&["analyze", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("JSON output is not implemented yet (#7)")
    );
}

#[test]
fn global_debug_is_accepted_before_and_after_subcommand() {
    for args in [&["--debug", "clean"][..], &["clean", "--debug"][..]] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented yet (#16)"));
    }
}

#[test]
fn help_matches_cli_snapshot() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert_eq!(stdout, include_str!("fixtures/cli-help.txt"));
}
