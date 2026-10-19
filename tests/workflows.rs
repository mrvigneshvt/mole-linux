use std::fs;

#[test]
fn ci_checks_rust_on_ubuntu_and_arch_and_runs_the_honeypot_scenarios() {
    let workflow = fs::read_to_string(".github/workflows/ci.yml").expect("CI workflow exists");

    assert!(workflow.contains("ubuntu-latest"));
    assert!(workflow.contains("docker run --rm") && workflow.contains("archlinux:latest"));
    assert!(workflow.contains("cargo fmt --all -- --check"));
    assert!(workflow.contains("cargo clippy --all-targets -- -D warnings"));
    assert!(workflow.contains("cargo test"));
    assert!(workflow.contains("rust cargo git"));
    assert!(workflow.contains("tests/honeypot/scenarios/*.sh"));
    assert!(workflow.contains("tests/honeypot/docker/run.sh"));
}

#[test]
fn release_builds_both_linux_libcs_and_publishes_checksums_and_provenance() {
    let workflow =
        fs::read_to_string(".github/workflows/release.yml").expect("release workflow exists");

    assert!(workflow.contains("tags:"));
    assert!(workflow.contains("v*"));
    assert!(workflow.contains("x86_64-unknown-linux-gnu"));
    assert!(workflow.contains("x86_64-unknown-linux-musl"));
    assert!(workflow.contains("SHA256SUMS"));
    assert!(workflow.contains("actions/attest-build-provenance"));
    for action in workflow
        .lines()
        .filter(|line| line.trim_start().starts_with("uses:"))
    {
        let reference = action
            .split('@')
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .expect("action must be pinned");
        assert_eq!(
            reference.len(),
            40,
            "action must use a full commit SHA: {action}"
        );
        assert!(
            reference
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }
}
