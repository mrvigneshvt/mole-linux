//! #81 help contracts: every flag of `clean`, `purge` and `installer` has a description,
//! `clean --help` lists its categories and the rule ids a `--select` can name, and the
//! three help screens are pinned to snapshots.

use std::process::{Command, Output};

#[path = "support/home.rs"]
mod home_support;
use home_support::CommandHomeExt;

fn help(args: &[&str]) -> String {
    let home = tempfile::tempdir().expect("temporary HOME");
    let mut command = Command::new(env!("CARGO_BIN_EXE_mole-linux"));
    command
        .args(args)
        .env_remove("COLUMNS")
        .isolated_home(home.path());
    let output: Output = command.output().expect("run help");
    assert!(
        output.status.success(),
        "{args:?} exited {:?}",
        output.status
    );
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

/// The help line for `flag` must carry text beyond the flag name and its `<VALUE>` placeholder.
fn assert_described(help: &str, flag: &str) {
    let line = help
        .lines()
        .find(|line| {
            line.split(|c: char| c == ',' || c.is_whitespace())
                .any(|token| token == flag)
        })
        .unwrap_or_else(|| panic!("no help line for {flag}:\n{help}"));
    let description: String = line
        .split_once(flag)
        .map_or("", |(_, rest)| rest)
        .split_whitespace()
        .filter(|word| !word.starts_with('<') && !word.ends_with('>'))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!description.is_empty(), "{flag} has no description: {line}");
}

#[test]
fn clean_help_describes_every_flag() {
    let text = help(&["clean", "--help"]);
    for flag in [
        "--dry-run",
        "--yes",
        "--categories",
        "--select",
        "--exclude",
        "--external",
        "--whitelist",
    ] {
        assert_described(&text, flag);
    }
}

#[test]
fn clean_help_lists_categories_and_rule_ids() {
    let text = help(&["clean", "--help"]);
    for name in [
        "user",
        "browser",
        "development",
        "system",
        "user-essentials",
        "trash",
        "coredumps",
        "browser-cache",
        "project-pycache",
        "pacman-cache-keep3",
        "pacman-cache-uninstalled",
        "aur-build-dirs",
        "orphan-packages",
        "journal",
    ] {
        assert!(
            text.contains(name),
            "clean --help does not list {name}:\n{text}"
        );
    }
}

#[test]
fn purge_help_describes_every_flag() {
    let text = help(&["purge", "--help"]);
    for flag in ["--dry-run", "--paths", "--include-empty", "--yes"] {
        assert_described(&text, flag);
    }
}

#[test]
fn installer_help_describes_every_flag() {
    let text = help(&["installer", "--help"]);
    for flag in ["--dry-run", "--max-depth", "--hidden", "--type"] {
        assert_described(&text, flag);
    }
}

#[test]
fn clean_help_matches_snapshot() {
    assert_eq!(
        help(&["clean", "--help"]),
        include_str!("fixtures/clean-help.txt")
    );
}

#[test]
fn purge_help_matches_snapshot() {
    assert_eq!(
        help(&["purge", "--help"]),
        include_str!("fixtures/purge-help.txt")
    );
}

#[test]
fn installer_help_matches_snapshot() {
    assert_eq!(
        help(&["installer", "--help"]),
        include_str!("fixtures/installer-help.txt")
    );
}
