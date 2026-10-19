use std::fs;
use std::io::Write;
use std::process::Command;

use tempfile::tempdir;

fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .output()
        .expect("run installer")
}

#[test]
fn dry_run_finds_linux_installers_with_paths_and_depth_limit() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(downloads.join("nested/deep")).expect("create scan tree");
    fs::write(downloads.join("tool.AppImage"), b"appimage").expect("seed AppImage");
    fs::write(downloads.join("package.pkg.tar.zst"), b"package").expect("seed package");
    fs::write(downloads.join("bundle.tar.custom"), b"archive").expect("seed tar archive");
    fs::write(downloads.join("remote.flatpakref"), b"flatpak ref").expect("seed flatpak ref");
    fs::write(downloads.join("nested/deep/too-deep.deb"), b"package").expect("seed deep package");
    fs::write(downloads.join("notes.txt"), b"not an installer").expect("seed ordinary file");

    let output = run(temp.path(), &["installer", "--dry-run", "--max-depth", "2"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tool.AppImage"), "{stdout}");
    assert!(stdout.contains("apparent "), "{stdout}");
    assert!(stdout.contains("allocated "), "{stdout}");
    assert!(stdout.contains("package.pkg.tar.zst"), "{stdout}");
    assert!(stdout.contains("bundle.tar.custom"), "{stdout}");
    assert!(stdout.contains("remote.flatpakref"), "{stdout}");
    assert!(!stdout.contains("too-deep.deb"), "{stdout}");
    assert!(!stdout.contains("notes.txt"), "{stdout}");
}

#[test]
fn zip_is_a_candidate_only_when_its_bounded_listing_has_installer_content() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(&downloads).expect("create Downloads");
    write_zip_with_entries(&downloads.join("installer.zip"), &["bundle/App.AppImage"]);
    write_zip_with_entries(&downloads.join("data.zip"), &["docs/readme.txt"]);
    fs::write(downloads.join("broken.zip"), b"not a zip").expect("seed corrupt archive");
    let ordinary_entries = (0..60)
        .map(|index| format!("ordinary-{index}.txt"))
        .collect::<Vec<_>>();
    let mut entries = vec!["early/package.rpm"];
    entries.extend(ordinary_entries.iter().map(String::as_str));
    write_zip_with_entries(&downloads.join("large.zip"), &entries);

    let output = run(temp.path(), &["installer", "--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("installer.zip"), "{stdout}");
    assert!(stdout.contains("large.zip"), "{stdout}");
    assert!(!stdout.contains("data.zip"), "{stdout}");
    assert!(!stdout.contains("broken.zip"), "{stdout}");
}

#[test]
fn dry_run_does_not_mutate_and_escapes_control_characters_in_names() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(&downloads).expect("create Downloads");
    let candidate = downloads.join("line\nbreak.deb");
    fs::write(&candidate, b"preserve during preview").expect("seed candidate");

    let output = run(temp.path(), &["installer", "--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(candidate.exists(), "dry-run changed the candidate");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("line\nbreak.deb"),
        "raw control character rendered: {stdout:?}"
    );
}

#[test]
fn type_filter_hidden_option_and_configured_roots_are_respected() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    let custom = temp.path().join("installers");
    fs::create_dir_all(&downloads).expect("create Downloads");
    fs::create_dir_all(&custom).expect("create configured root");
    fs::write(downloads.join(".hidden.deb"), b"hidden").expect("seed hidden");
    fs::write(downloads.join("visible.rpm"), b"visible").expect("seed visible");
    fs::write(custom.join("configured.deb"), b"configured").expect("seed configured");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["installer", "--dry-run", "--type", "deb", "--hidden"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join(".config"))
        .env("XDG_DATA_HOME", temp.path().join(".local/share"))
        .env("XDG_STATE_HOME", temp.path().join(".local/state"))
        .env("XDG_CACHE_HOME", temp.path().join(".cache"))
        .env("MOLE_INSTALLER_PATHS", &custom)
        .output()
        .expect("run configured installer scan");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(".hidden.deb"), "{stdout}");
    assert!(stdout.contains("configured.deb"), "{stdout}");
    assert!(!stdout.contains("visible.rpm"), "{stdout}");
}

#[test]
fn discovery_skips_descendant_symlinks_but_accepts_a_symlink_scan_root() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().expect("temp home");
    let real = temp.path().join("real");
    let external = temp.path().join("outside");
    fs::create_dir_all(&real).expect("create real root");
    fs::create_dir_all(&external).expect("create external root");
    fs::write(real.join("included.deb"), b"included").expect("seed included");
    fs::write(external.join("excluded.rpm"), b"excluded").expect("seed excluded");
    symlink(&real, temp.path().join("Downloads")).expect("link scan root");
    symlink(&external, real.join("linked-child")).expect("link descendant directory");
    symlink(external.join("excluded.rpm"), real.join("linked-file.rpm"))
        .expect("link descendant file");

    let output = run(temp.path(), &["installer", "--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("included.deb"), "{stdout}");
    assert!(!stdout.contains("excluded.rpm"), "{stdout}");
}

#[test]
fn zip_content_after_entry_fifty_is_not_a_candidate() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(&downloads).expect("create Downloads");
    let entries = (0..50)
        .map(|index| format!("ordinary-{index}.txt"))
        .collect::<Vec<_>>();
    let mut entries = entries.iter().map(String::as_str).collect::<Vec<_>>();
    entries.push("late/App.deb");
    write_zip_with_entries(&downloads.join("late.zip"), &entries);

    let output = run(temp.path(), &["installer", "--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("late.zip"));
}

#[test]
fn scan_depth_environment_default_is_honored() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(downloads.join("nested")).expect("create scan tree");
    fs::write(downloads.join("root.deb"), b"root").expect("seed root candidate");
    fs::write(downloads.join("nested/child.rpm"), b"child").expect("seed nested candidate");

    let output = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["installer", "--dry-run"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join(".config"))
        .env("XDG_DATA_HOME", temp.path().join(".local/share"))
        .env("XDG_STATE_HOME", temp.path().join(".local/state"))
        .env("XDG_CACHE_HOME", temp.path().join(".cache"))
        .env("MOLE_INSTALLER_SCAN_MAX_DEPTH", "1")
        .output()
        .expect("run limited installer scan");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("root.deb"), "{stdout}");
    assert!(!stdout.contains("child.rpm"), "{stdout}");
}

#[test]
fn missing_roots_are_an_empty_successful_scan() {
    let temp = tempdir().expect("temp home");
    let output = run(temp.path(), &["installer", "--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("No installer files found."));
}

#[test]
fn unreadable_subdirectories_keep_candidates_and_report_a_partial_scan() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(downloads.join("unreadable")).expect("create scan tree");
    fs::write(downloads.join("visible.deb"), b"visible").expect("seed readable candidate");
    fs::write(downloads.join("unreadable/hidden.rpm"), b"hidden")
        .expect("seed unreadable candidate");
    fs::set_permissions(
        downloads.join("unreadable"),
        fs::Permissions::from_mode(0o000),
    )
    .expect("make subdirectory unreadable");

    let output = run(temp.path(), &["installer", "--dry-run"]);
    fs::set_permissions(
        downloads.join("unreadable"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("restore directory permissions");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("visible.deb"), "{stdout}");
    assert!(!stdout.contains("hidden.rpm"), "{stdout}");
    assert!(stdout.contains("Scan incomplete"), "{stdout}");
}

#[test]
fn whitelist_is_rechecked_before_apply() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(&downloads).expect("create Downloads");
    let target = downloads.join("protected.deb");
    fs::write(&target, b"protected").expect("seed protected candidate");
    fs::create_dir_all(temp.path().join(".config/mole-linux")).expect("create config");
    fs::write(
        temp.path().join(".config/mole-linux/whitelist"),
        format!("{}\n", target.display()),
    )
    .expect("write whitelist");

    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["installer"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join(".config"))
        .env("XDG_DATA_HOME", temp.path().join(".local/share"))
        .env("XDG_STATE_HOME", temp.path().join(".local/state"))
        .env("XDG_CACHE_HOME", temp.path().join(".cache"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn installer");
    let mut input = child.stdin.take().expect("stdin");
    writeln!(input, "{}", target.display()).expect("select path");
    writeln!(input).expect("finish selection");
    writeln!(input, "yes").expect("confirm selection");
    drop(input);
    let output = child.wait_with_output().expect("wait installer");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(target.exists(), "whitelisted target was removed");
    assert!(
        !temp
            .path()
            .join(".local/share/Trash/files/protected.deb")
            .exists()
    );
}

#[test]
fn unsafe_trash_failure_is_reported_as_incomplete_cleanup() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    let outside = temp.path().join("outside-trash");
    fs::create_dir_all(&downloads).expect("create Downloads");
    fs::create_dir_all(&outside).expect("create outside path");
    let target = downloads.join("candidate.deb");
    fs::write(&target, b"candidate").expect("seed candidate");
    fs::create_dir_all(temp.path().join(".local/share")).expect("create XDG data");
    symlink(&outside, temp.path().join(".local/share/Trash")).expect("create unsafe Trash link");

    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["installer"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join(".config"))
        .env("XDG_DATA_HOME", temp.path().join(".local/share"))
        .env("XDG_STATE_HOME", temp.path().join(".local/state"))
        .env("XDG_CACHE_HOME", temp.path().join(".cache"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn installer");
    let mut input = child.stdin.take().expect("stdin");
    writeln!(input, "{}", target.display()).expect("select path");
    writeln!(input).expect("finish selection");
    writeln!(input, "yes").expect("confirm selection");
    drop(input);
    let output = child.wait_with_output().expect("wait installer");
    assert!(!output.status.success(), "unsafe Trash link was accepted");
    assert!(target.exists(), "failed Trash move removed its source");
    assert!(!outside.join("candidate.deb").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("installer cleanup incomplete"));
}

#[test]
fn apply_requires_explicit_path_confirmation_and_trashes_only_confirmed_identity() {
    let temp = tempdir().expect("temp home");
    let downloads = temp.path().join("Downloads");
    fs::create_dir_all(&downloads).expect("create Downloads");
    fs::write(downloads.join("keep.rpm"), b"keep").expect("seed unselected installer");
    fs::write(downloads.join("remove.deb"), b"remove").expect("seed selected installer");

    let mut child = Command::new(env!("CARGO_BIN_EXE_mole-linux"))
        .args(["installer"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join(".config"))
        .env("XDG_DATA_HOME", temp.path().join(".local/share"))
        .env("XDG_STATE_HOME", temp.path().join(".local/state"))
        .env("XDG_CACHE_HOME", temp.path().join(".cache"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn installer");
    let mut input = child.stdin.take().expect("stdin");
    writeln!(input, "{}", downloads.join("remove.deb").display()).expect("select path");
    writeln!(input).expect("finish selection");
    writeln!(input, "yes").expect("confirm selection");
    drop(input);
    let output = child.wait_with_output().expect("wait installer");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(downloads.join("keep.rpm").exists());
    assert!(!downloads.join("remove.deb").exists());
    assert!(
        temp.path()
            .join(".local/share/Trash/files/remove.deb")
            .exists()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Space is not freed until Trash is emptied."),
        "{stdout}"
    );
    assert!(
        temp.path()
            .join(".local/state/mole-linux/operations.log")
            .exists()
    );
}

fn write_zip_with_entries(path: &std::path::Path, entries: &[&str]) {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for name in entries {
        let name_bytes = name.as_bytes();
        let offset = u32::try_from(bytes.len()).expect("local offset");
        bytes.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        bytes.extend_from_slice(&20_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(
            &u16::try_from(name_bytes.len())
                .expect("name length")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(name_bytes);

        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(
            &u16::try_from(name_bytes.len())
                .expect("name length")
                .to_le_bytes(),
        );
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let central_offset = u32::try_from(bytes.len()).expect("central offset");
    let central_size = u32::try_from(central.len()).expect("central size");
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    let count = u16::try_from(entries.len()).expect("entry count");
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&central_size.to_le_bytes());
    bytes.extend_from_slice(&central_offset.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    fs::write(path, bytes).expect("write ZIP fixture");
}
