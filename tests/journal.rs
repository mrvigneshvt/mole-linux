//! M6.3 journald primitives: usage probe through `OwnerRunner`, bounded machine-id path, archived/active inventory.
//! Behavior source: journalctl(1) and systemd-journald(8) at systemd 261 (no upstream
//! Mole contract exists for journald; see PARITY-LEDGER.md).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use mole_linux::commands::journal::{
    self, MAX_JOURNAL_ENTRIES, Retention, Usage, inventory, journal_dir_from, parse_disk_usage,
};
use mole_linux::core::privilege::{Consent, OwnerRunner};

const DISK_USAGE_LINE: &str = "Archived and active journals take up 286.8M in the file system.";

fn write_shim(dir: &Path, name: &str, log: &Path, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("shim dir");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{name}' >> '{log}'\nfor a in \"$@\"; do printf '\\t%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n",
        log = log.display(),
    );
    let path = dir.join(name);
    fs::write(&path, script).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    path
}

fn sudo_shim(dir: &Path, log: &Path) -> PathBuf {
    write_shim(
        dir,
        "sudo",
        log,
        "if [ \"$1\" = -- ]; then shift; fi\nexec \"$@\"",
    )
}

fn read_log(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Shim tests in one binary run one at a time. A sibling test's fork can hold a freshly written
/// shim open for write, and exec then fails with ETXTBSY ("Text file busy"). Other binaries are
/// separate processes and never share that descriptor, so only same-binary tests need this.
static SHIM_SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _serial: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    bin: PathBuf,
    sudo_dir: PathBuf,
    log: PathBuf,
}

fn fixture() -> Fixture {
    let serial = SHIM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    Fixture {
        _serial: serial,
        bin: root.join("usr-bin"),
        sudo_dir: root.join("sudo-bin"),
        log: root.join("calls.log"),
        _temp: temp,
    }
}

#[test]
fn parses_the_value_journalctl_reports() {
    assert_eq!(
        parse_disk_usage(&format!("{DISK_USAGE_LINE}\n")),
        Some("286.8M".to_owned())
    );
    assert_eq!(
        parse_disk_usage("Archived and active journals take up 0B in the file system.\n"),
        Some("0B".to_owned())
    );
}

#[test]
fn rejects_output_that_is_not_the_expected_shape() {
    assert_eq!(parse_disk_usage(""), None);
    assert_eq!(parse_disk_usage("Failed to open journal\n"), None);
    assert_eq!(
        parse_disk_usage("Archived and active journals take up 1G; rm -rf / in the file system.\n"),
        None
    );
    assert_eq!(
        parse_disk_usage("Archived and active journals take up -5M in the file system.\n"),
        None
    );
}

#[test]
fn disk_usage_is_read_through_owner_runner_without_sudo() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(
        &fx.bin,
        "journalctl",
        &fx.log,
        &format!("printf '%s\\n' '{DISK_USAGE_LINE}'"),
    );
    let runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Unasked);

    let usage = journal::disk_usage(&runner);

    assert_eq!(usage, Usage::Reported("286.8M".to_owned()));
    assert_eq!(
        read_log(&fx.log),
        vec!["journalctl\t--disk-usage".to_owned()]
    );
}

#[test]
fn disk_usage_is_unknown_when_journalctl_is_missing() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    let runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Unasked);

    let usage = journal::disk_usage(&runner);

    match usage {
        Usage::Unknown(reason) => assert!(reason.contains("journalctl"), "reason: {reason}"),
        other => panic!("missing tool must be Unknown, got {other:?}"),
    }
}

#[test]
fn disk_usage_is_unknown_when_output_is_unrecognized() {
    let fx = fixture();
    let sudo = sudo_shim(&fx.sudo_dir, &fx.log);
    write_shim(&fx.bin, "journalctl", &fx.log, "printf 'nonsense\\n'");
    let runner = OwnerRunner::new(fx.bin.clone(), sudo, Consent::Unasked);

    let usage = journal::disk_usage(&runner);

    assert!(matches!(usage, Usage::Unknown(_)), "got {usage:?}");
}

#[test]
fn retention_accepts_only_bounded_values_and_builds_journalctl_flags() {
    assert_eq!(
        Retention::size_mib(512).expect("512 MiB").arg(),
        "--vacuum-size=512M"
    );
    assert_eq!(
        Retention::age_days(30).expect("30 days").arg(),
        "--vacuum-time=30days"
    );
    assert!(Retention::size_mib(0).is_err());
    assert!(Retention::size_mib(u32::MAX).is_err());
    assert!(Retention::age_days(0).is_err());
    assert!(Retention::age_days(3651).is_err());
}

#[test]
fn inventory_splits_archived_from_active_and_ignores_unclassified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path();
    for name in [
        "system.journal",
        "user-1000.journal",
        "system@ed80-0000000000000001-00065d07.journal",
        "system@ed80-000000000000e0a5-00065d1a.journal~",
        "user-1000@ed80-00000000000059f-00065d07.journal",
        "weird.journal~",
        "notes.txt",
    ] {
        fs::write(dir.join(name), b"x").expect("write journal fixture");
    }

    let found = inventory(dir).expect("inventory");

    assert_eq!(found.archived, 3);
    assert_eq!(
        found.active,
        vec!["system.journal".to_owned(), "user-1000.journal".to_owned()]
    );
    assert_eq!(found.other, 2);
}

#[test]
fn inventory_refuses_directories_beyond_the_entry_bound() {
    let temp = tempfile::tempdir().expect("tempdir");
    for index in 0..=MAX_JOURNAL_ENTRIES {
        fs::write(temp.path().join(format!("f{index}")), b"").expect("write entry");
    }

    let error = inventory(temp.path()).expect_err("too many entries must fail");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn journal_dir_requires_a_32_hex_machine_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_id = temp.path().join("machine-id");
    let root = temp.path().join("journal");

    fs::write(&machine_id, "fc919a235e10479284adc8a1a11a0bfd\n").expect("write id");
    assert_eq!(
        journal_dir_from(&machine_id, &root).expect("valid id"),
        root.join("fc919a235e10479284adc8a1a11a0bfd")
    );

    for hostile in ["../../etc", "FC919A235E10479284ADC8A1A11A0BFD", "short"] {
        fs::write(&machine_id, hostile).expect("write hostile id");
        let error = journal_dir_from(&machine_id, &root).expect_err("hostile id must fail");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidData,
            "id: {hostile}"
        );
    }
}
