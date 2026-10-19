use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use mole_linux::commands::uninstall::{
    Inventory, Section, Status, inventory, parse_desktop_name, parse_flatpak_list,
    parse_pacman_list, render,
};
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};

struct Env {
    _temp: tempfile::TempDir,
    bin: PathBuf,
    paths: XdgPaths,
}

/// Shim files are written and then exec'd; a sibling test thread forking in that window
/// makes exec fail with ETXTBSY. Tests in this file therefore run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn env() -> Env {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).expect("home");
    let paths = XdgPaths::from_values(home, None, None, None, None).expect("paths");
    Env {
        bin: root.join("usr-bin"),
        paths,
        _temp: temp,
    }
}

fn write_shim(dir: &Path, name: &str, body: &str) {
    fs::create_dir_all(dir).expect("shim dir");
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write shim");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

fn owner(env: &Env) -> OwnerRunner {
    OwnerRunner::new(env.bin.clone(), env.bin.join("no-sudo"), Consent::Unasked)
}

fn section<'a>(inventory: &'a Inventory, title: &str) -> &'a Section {
    inventory
        .sections
        .iter()
        .find(|section| section.title == title)
        .unwrap_or_else(|| panic!("no section titled {title}"))
}

fn listed_names(section: &Section) -> Vec<String> {
    match &section.status {
        Status::Listed(entries) => entries.iter().map(|entry| entry.name.clone()).collect(),
        Status::Unavailable(reason) => panic!("{} unavailable: {reason}", section.title),
    }
}

const PACMAN_SHIM: &str = r#"case "$1" in
  -Qe) printf 'bash 5.2.37-1\nparu-bin 2.0-1\nzsh 5.9-1\n' ;;
  -Qm) printf 'paru-bin 2.0-1\n' ;;
  *) exit 9 ;;
esac"#;

#[test]
fn pacman_explicit_packages_split_foreign_ones_into_the_aur_section() {
    let _serial = serial();
    let env = env();
    write_shim(&env.bin, "pacman", PACMAN_SHIM);

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "pacman explicit")),
        vec!["bash", "zsh"]
    );
    assert_eq!(
        listed_names(section(&inventory, "AUR/foreign")),
        vec!["paru-bin"]
    );
    let Status::Listed(entries) = &section(&inventory, "pacman explicit").status else {
        panic!("pacman section must be listed");
    };
    assert_eq!(entries[0].detail, "5.2.37-1", "detail carries the version");
}

#[test]
fn missing_pacman_is_unavailable_never_an_empty_list() {
    let _serial = serial();
    let env = env();

    let inventory = inventory(&owner(&env), &env.paths);

    for title in ["pacman explicit", "AUR/foreign"] {
        match &section(&inventory, title).status {
            Status::Unavailable(reason) => assert!(
                reason.contains("missing or not executable"),
                "{title} reason: {reason}"
            ),
            Status::Listed(entries) => {
                panic!("{title} must be unavailable, got {} entries", entries.len())
            }
        }
    }
}

#[test]
fn failing_aur_query_keeps_the_explicit_list_and_marks_aur_unavailable() {
    let _serial = serial();
    let env = env();
    write_shim(
        &env.bin,
        "pacman",
        r#"case "$1" in
  -Qe) printf 'bash 5.2.37-1\nparu-bin 2.0-1\n' ;;
  -Qm) echo 'error: failed to open database' >&2; exit 1 ;;
esac"#,
    );

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "pacman explicit")),
        vec!["bash", "paru-bin"]
    );
    match &section(&inventory, "AUR/foreign").status {
        Status::Unavailable(reason) => assert!(reason.contains("exited"), "reason: {reason}"),
        Status::Listed(_) => panic!("AUR must be unavailable when -Qm fails"),
    }
}

#[test]
fn foreign_query_with_no_matches_is_an_empty_aur_section() {
    let env = env();
    write_shim(
        &env.bin,
        "pacman",
        r#"case "$1" in
  -Qe) printf 'bash 5.2.37-1\n' ;;
  -Qm) exit 1 ;;
esac"#,
    );

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "AUR/foreign")),
        Vec::<String>::new()
    );
    assert_eq!(
        listed_names(section(&inventory, "pacman explicit")),
        vec!["bash"]
    );
}

#[test]
fn malformed_pacman_output_is_unavailable_not_silently_dropped() {
    let _serial = serial();
    let env = env();
    write_shim(
        &env.bin,
        "pacman",
        r#"case "$1" in
  -Qe) printf 'bash 5.2.37-1\nbroken-line-without-version\n' ;;
  -Qm) exit 0 ;;
esac"#,
    );

    let inventory = inventory(&owner(&env), &env.paths);

    match &section(&inventory, "pacman explicit").status {
        Status::Unavailable(reason) => {
            assert!(reason.contains("unexpected"), "reason: {reason}")
        }
        Status::Listed(_) => panic!("a malformed line must not produce a partial list"),
    }
}

#[test]
fn parse_pacman_list_rejects_lines_without_a_version() {
    let _serial = serial();
    assert!(parse_pacman_list("bash 5.2-1\n").is_ok());
    assert!(parse_pacman_list("bash\n").is_err());
    assert_eq!(parse_pacman_list("").expect("empty ok").len(), 0);
}

#[test]
fn flatpak_apps_are_listed_and_a_header_row_is_skipped() {
    let _serial = serial();
    let env = env();
    write_shim(
        &env.bin,
        "flatpak",
        "printf 'Application ID\\tVersion\\norg.gimp.GIMP\\t3.0\\n'",
    );

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "flatpak")),
        vec!["org.gimp.GIMP"]
    );
}

#[test]
fn flatpak_output_with_a_non_app_id_row_is_unavailable() {
    let _serial = serial();
    assert!(parse_flatpak_list("org.gimp.GIMP\t3.0\n").is_ok());
    assert!(parse_flatpak_list("Application ID\tVersion\n").is_ok());
    assert!(parse_flatpak_list("org.gimp.GIMP\t3.0\nnot an id\tx\n").is_err());
}

#[test]
fn missing_flatpak_is_unavailable_with_reason() {
    let _serial = serial();
    let env = env();

    let inventory = inventory(&owner(&env), &env.paths);

    match &section(&inventory, "flatpak").status {
        Status::Unavailable(reason) => assert!(reason.contains("flatpak")),
        Status::Listed(_) => panic!("flatpak must be unavailable when absent"),
    }
}

#[test]
fn appimages_in_common_dirs_are_listed_and_symlinks_are_not_followed() {
    let _serial = serial();
    let env = env();
    let home = &env.paths.home;
    fs::create_dir_all(home.join("Applications")).expect("Applications");
    fs::create_dir_all(home.join("AppImages")).expect("AppImages");
    fs::write(home.join("Applications/Foo-1.2.AppImage"), b"x").expect("foo");
    fs::write(home.join("AppImages/bar.appimage"), b"x").expect("bar");
    fs::write(home.join("Applications/notes.txt"), b"x").expect("notes");
    symlink("/etc/hostname", home.join("Applications/link.AppImage")).expect("symlink");

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "AppImage")),
        vec!["bar", "Foo-1.2"]
    );
}

#[test]
fn appimage_directory_over_the_entry_budget_is_unavailable() {
    let _serial = serial();
    let env = env();
    let dir = env.paths.home.join("Applications");
    fs::create_dir_all(&dir).expect("Applications");
    for index in 0..10_001 {
        fs::write(dir.join(format!("f{index}.txt")), b"").expect("filler");
    }

    let inventory = inventory(&owner(&env), &env.paths);

    match &section(&inventory, "AppImage").status {
        Status::Unavailable(reason) => assert!(reason.contains("10000"), "reason: {reason}"),
        Status::Listed(_) => panic!("an over-budget directory must not be partially listed"),
    }
}

#[test]
fn desktop_entries_use_the_unlocalized_entry_name_with_stem_fallback() {
    let _serial = serial();
    let env = env();
    let apps = env.paths.data.join("applications");
    fs::create_dir_all(&apps).expect("applications");
    fs::write(
        apps.join("alpha.desktop"),
        "[Desktop Entry]\nName[de]=Alfa\nName=Alpha\nExec=alpha\n",
    )
    .expect("alpha");
    fs::write(
        apps.join("gamma.desktop"),
        "[Desktop Action new]\nName=Action\n[Desktop Entry]\nName=Gamma\n",
    )
    .expect("gamma");
    fs::write(apps.join("beta.desktop"), "[Desktop Entry]\nExec=beta\n").expect("beta");
    fs::write(apps.join("readme.txt"), "Name=Ignored\n").expect("readme");

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "desktop entries")),
        vec!["Alpha", "beta", "Gamma"]
    );
}

#[test]
fn parse_desktop_name_only_reads_the_desktop_entry_section() {
    let _serial = serial();
    assert_eq!(
        parse_desktop_name("[Desktop Entry]\nName=Alpha\n").as_deref(),
        Some("Alpha")
    );
    assert_eq!(
        parse_desktop_name("[Desktop Action x]\nName=Action\n"),
        None
    );
}

#[test]
fn oversized_desktop_file_is_listed_by_stem_and_not_read() {
    let _serial = serial();
    let env = env();
    let apps = env.paths.data.join("applications");
    fs::create_dir_all(&apps).expect("applications");
    let mut body = String::from("[Desktop Entry]\nName=Huge\n");
    body.push_str(&"# padding\n".repeat(10_000));
    fs::write(apps.join("huge.desktop"), body).expect("huge");

    let inventory = inventory(&owner(&env), &env.paths);

    assert_eq!(
        listed_names(section(&inventory, "desktop entries")),
        vec!["huge"]
    );
}

#[test]
fn rendered_names_escape_control_characters() {
    let _serial = serial();
    let env = env();
    let apps = env.paths.data.join("applications");
    fs::create_dir_all(&apps).expect("applications");
    fs::write(
        apps.join("evil.desktop"),
        "[Desktop Entry]\nName=\x1b[31mRed\x07\n",
    )
    .expect("evil");

    let text = render(&inventory(&owner(&env), &env.paths));

    assert!(!text.contains('\u{1b}'), "raw ESC leaked: {text:?}");
    assert!(!text.contains('\u{7}'), "raw BEL leaked: {text:?}");
    assert!(text.contains("\\u{1b}"), "escaped form expected: {text}");
}

#[test]
fn render_states_every_source_with_its_count_or_reason() {
    let _serial = serial();
    let env = env();

    let text = render(&inventory(&owner(&env), &env.paths));

    for title in [
        "pacman explicit",
        "AUR/foreign",
        "flatpak",
        "AppImage",
        "desktop entries",
    ] {
        assert!(text.contains(title), "missing {title}: {text}");
    }
    assert!(
        text.contains("unavailable ("),
        "unknown state is explicit: {text}"
    );
    assert!(
        text.contains("nothing is removed"),
        "read-only promise: {text}"
    );
}
