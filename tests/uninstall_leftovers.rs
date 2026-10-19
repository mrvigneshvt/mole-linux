use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

#[path = "support/home.rs"]
mod home_support;

use mole_linux::commands::uninstall::{Entry, Inventory, Section, Status};
use mole_linux::commands::uninstall_leftovers::{
    Class, Leftover, Preview, Refusal, Size, preview, render,
};
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::privilege::{Consent, OwnerRunner};

struct Env {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    bin: PathBuf,
    paths: XdgPaths,
}

/// Shims are written and then exec'd; a sibling test forking in that window fails with ETXTBSY.
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
    let paths = XdgPaths::from_values(home.clone(), None, None, None, None).expect("paths");
    Env {
        bin: root.join("usr-bin"),
        root,
        home,
        paths,
        _temp: temp,
    }
}

fn dir(env: &Env, relative: &str) -> PathBuf {
    let path = env.home.join(relative);
    fs::create_dir_all(&path).expect("create leftover dir");
    path
}

fn file(env: &Env, relative: &str, bytes: &[u8]) {
    let path = env.home.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
    fs::write(&path, bytes).expect("write leftover file");
}

fn write_shim(dir: &Path, name: &str, body: &str) {
    fs::create_dir_all(dir).expect("shim dir");
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write shim");
    fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod shim");
}

/// Owner answers for `pacman -Qo <path>`: `.config/foo` is owned, `.local/share/foo` is an
/// unanswerable owner query, every other path is unowned.
const PACMAN_OWNERS: &str = r#"[ "$1" = "-Qo" ] || exit 9
case "$2" in
  */.config/foo) echo "$2 is owned by foo-pkg 1.0-1"; exit 0 ;;
  */.local/share/foo) echo "error: failed to open database" >&2; exit 1 ;;
  *) echo "error: No package owns $2" >&2; exit 1 ;;
esac"#;

fn owner(env: &Env) -> OwnerRunner {
    write_shim(&env.bin, "pacman", PACMAN_OWNERS);
    OwnerRunner::new(env.bin.clone(), env.bin.join("no-sudo"), Consent::Unasked)
}

fn entry(name: &str) -> Entry {
    Entry {
        name: name.to_owned(),
        detail: String::new(),
        path: None,
    }
}

fn section(title: &'static str, entries: Vec<Entry>) -> Section {
    Section {
        title,
        status: Status::Listed(entries),
    }
}

fn inventory(sections: Vec<Section>) -> Inventory {
    Inventory { sections }
}

fn rel(env: &Env, path: &Path) -> String {
    path.strip_prefix(&env.home)
        .expect("path under temporary HOME")
        .to_string_lossy()
        .into_owned()
}

fn offered(env: &Env, preview: &Preview) -> Vec<(Class, String)> {
    let mut list: Vec<(Class, String)> = preview
        .leftovers
        .iter()
        .map(|leftover| (leftover.class, rel(env, &leftover.path)))
        .collect();
    list.sort_by(|left, right| left.1.cmp(&right.1));
    list
}

fn refused(env: &Env, preview: &Preview) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = preview
        .refused
        .iter()
        .map(|refusal| (rel(env, &refusal.path), refusal.reason.clone()))
        .collect();
    list.sort_by(|left, right| left.0.cmp(&right.0));
    list
}

fn reason_for(env: &Env, preview: &Preview, relative: &str) -> String {
    refused(env, preview)
        .into_iter()
        .find(|(path, _)| path == relative)
        .unwrap_or_else(|| panic!("{relative} was not refused: {:?}", refused(env, preview)))
        .1
}

fn run_preview(env: &Env, inventory: &Inventory, query: &str) -> Preview {
    preview(&owner(env), &env.paths, inventory, query).expect("preview")
}

#[test]
fn variants_match_hyphen_case_and_spacing_across_xdg_roots() {
    let _serial = serial();
    let env = env();
    dir(&env, ".config/maestro-studio");
    dir(&env, ".cache/MaestroStudio");
    dir(&env, ".local/share/maestrostudio");
    dir(&env, ".config/maestro"); // a different name: must not match
    let inv = inventory(vec![section(
        "pacman explicit",
        vec![entry("maestro-studio")],
    )]);

    let preview = run_preview(&env, &inv, "Maestro Studio");

    assert_eq!(
        offered(&env, &preview),
        vec![
            (Class::Regenerable, ".cache/MaestroStudio".to_owned()),
            (Class::Personal, ".config/maestro-studio".to_owned()),
            (Class::Personal, ".local/share/maestrostudio".to_owned()),
        ]
    );
}

#[test]
fn version_suffix_and_reverse_dns_id_find_the_base_app() {
    let _serial = serial();
    let env = env();
    dir(&env, ".config/zed");
    dir(&env, ".local/share/zed");
    dir(&env, ".cache/zed-nightly");
    dir(&env, ".config/zedit"); // a different name: must not match
    let inv = inventory(vec![section("flatpak", vec![entry("dev.zed.Zed-Nightly")])]);

    let preview = run_preview(&env, &inv, "Zed Nightly");

    assert_eq!(
        offered(&env, &preview),
        vec![
            (Class::Regenerable, ".cache/zed-nightly".to_owned()),
            (Class::Personal, ".config/zed".to_owned()),
            (Class::Personal, ".local/share/zed".to_owned()),
        ]
    );
}

#[test]
fn flatpak_sandbox_is_personal_data_and_cache_is_selected() {
    let _serial = serial();
    let env = env();
    dir(&env, ".var/app/org.mozilla.firefox");
    dir(&env, ".cache/firefox");
    let inv = inventory(vec![section("flatpak", vec![entry("org.mozilla.firefox")])]);

    let preview = run_preview(&env, &inv, "org.mozilla.firefox");

    assert_eq!(
        offered(&env, &preview),
        vec![
            (Class::Regenerable, ".cache/firefox".to_owned()),
            (Class::Personal, ".var/app/org.mozilla.firefox".to_owned()),
        ]
    );
}

#[test]
fn shared_locations_such_as_steam_libraries_are_never_offered() {
    let _serial = serial();
    let env = env();
    dir(&env, ".local/share/Steam/steamapps");
    dir(&env, ".cache/steam");
    let inv = inventory(vec![section("pacman explicit", vec![entry("steam")])]);

    let preview = run_preview(&env, &inv, "steam");

    assert!(
        offered(&env, &preview).is_empty(),
        "{:?}",
        offered(&env, &preview)
    );
    assert!(
        reason_for(&env, &preview, ".local/share/Steam").contains("shared location"),
        "Steam library must be refused as a shared location"
    );
}

#[test]
fn browser_profiles_are_refused_outside_cache() {
    let _serial = serial();
    let env = env();
    file(&env, ".local/share/firefox/profiles.ini", b"[General]\n");
    dir(&env, ".cache/firefox");
    let inv = inventory(vec![section(
        "pacman explicit",
        vec![entry("firefox-developer-edition")],
    )]);

    let preview = run_preview(&env, &inv, "Firefox Developer Edition");

    assert_eq!(
        offered(&env, &preview),
        vec![(Class::Regenerable, ".cache/firefox".to_owned())]
    );
    assert!(
        reason_for(&env, &preview, ".local/share/firefox").contains("browser profile"),
        "browser profile data must never be offered"
    );
}

#[test]
fn package_owned_paths_are_refused_and_unknown_owner_is_not_offered() {
    let _serial = serial();
    let env = env();
    dir(&env, ".config/foo"); // shim: owned by foo-pkg
    dir(&env, ".cache/foo"); // shim: no owner
    dir(&env, ".local/share/foo"); // shim: owner query fails
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);

    let preview = run_preview(&env, &inv, "foo");

    assert_eq!(
        offered(&env, &preview),
        vec![(Class::Regenerable, ".cache/foo".to_owned())]
    );
    assert!(
        reason_for(&env, &preview, ".config/foo").contains("owned by package foo-pkg"),
        "package-owned path must be refused"
    );
    assert!(
        reason_for(&env, &preview, ".local/share/foo").contains("owner check unavailable"),
        "a failed owner query must refuse, never offer"
    );
}

#[test]
fn leftover_shared_with_another_installed_app_is_kept() {
    let _serial = serial();
    let env = env();
    dir(&env, ".config/Code");
    dir(&env, ".cache/code");
    let inv = inventory(vec![section(
        "pacman explicit",
        vec![entry("code"), entry("code-insiders")],
    )]);

    let preview = run_preview(&env, &inv, "Code");

    assert!(
        offered(&env, &preview).is_empty(),
        "{:?}",
        offered(&env, &preview)
    );
    assert!(
        reason_for(&env, &preview, ".config/Code").contains("installed app code-insiders"),
        "a dir another installed app can claim must be kept"
    );
}

#[test]
fn desktop_entry_exec_joins_the_same_app_instead_of_counting_as_another() {
    let _serial = serial();
    let env = env();
    let desktop = env
        .home
        .join(".local/share/applications/firefox-browser.desktop");
    fs::create_dir_all(desktop.parent().expect("parent")).expect("applications dir");
    fs::write(
        &desktop,
        "[Desktop Entry]\nName=Mozilla Firefox Browser\nExec=firefox %u\n",
    )
    .expect("desktop file");
    dir(&env, ".cache/firefox");
    let inv = inventory(vec![
        section("pacman explicit", vec![entry("firefox")]),
        section(
            "desktop entries",
            vec![Entry {
                name: "Mozilla Firefox Browser".to_owned(),
                detail: desktop.display().to_string(),
                path: Some(desktop.clone()),
            }],
        ),
    ]);

    let preview = run_preview(&env, &inv, "firefox");

    assert!(
        preview.sources.contains(&"desktop entries"),
        "{:?}",
        preview.sources
    );
    assert_eq!(
        offered(&env, &preview),
        vec![(Class::Regenerable, ".cache/firefox".to_owned())]
    );
}

#[test]
fn sqlite_and_git_leftovers_are_refused() {
    let _serial = serial();
    let env = env();
    file(&env, ".cache/foo/index.db", b"SQLite format 3\0");
    file(&env, ".config/foo/.git/HEAD", b"ref: refs/heads/main\n");
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);

    let preview = run_preview(&env, &inv, "foo");

    assert!(
        offered(&env, &preview).is_empty(),
        "{:?}",
        offered(&env, &preview)
    );
    assert!(reason_for(&env, &preview, ".cache/foo").contains("SQLite"));
    assert!(reason_for(&env, &preview, ".config/foo").contains("git"));
}

#[test]
fn symlinked_leftover_is_refused_and_not_followed() {
    let _serial = serial();
    let env = env();
    let outside = env.root.join("outside");
    fs::create_dir_all(&outside).expect("outside dir");
    fs::create_dir_all(env.home.join(".cache")).expect("cache dir");
    symlink(&outside, env.home.join(".cache/foo")).expect("symlink leftover");
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);

    let preview = run_preview(&env, &inv, "foo");

    assert!(
        offered(&env, &preview).is_empty(),
        "{:?}",
        offered(&env, &preview)
    );
    assert!(reason_for(&env, &preview, ".cache/foo").contains("symbolic link"));
}

fn tree(root: &Path) -> Vec<String> {
    let mut list = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("metadata");
            list.push(format!("{} {}", path.display(), meta.len()));
            if meta.is_dir() {
                pending.push(path);
            }
        }
    }
    list.sort();
    list
}

#[test]
fn preview_never_writes_into_home() {
    let _serial = serial();
    let env = env();
    file(&env, ".cache/foo/data", &[7_u8; 64]);
    file(&env, ".config/foo/settings.toml", b"a = 1\n");
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);
    let before = tree(&env.home);

    let _ = run_preview(&env, &inv, "foo");

    assert_eq!(
        tree(&env.home),
        before,
        "dry-run preview must not change HOME"
    );
}

#[test]
fn leftover_sizes_are_apparent_and_allocated_bytes() {
    let _serial = serial();
    let env = env();
    file(&env, ".cache/foo/data", &[1_u8; 1234]);
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);

    let preview = run_preview(&env, &inv, "foo");

    let leftover: &Leftover = preview
        .leftovers
        .iter()
        .find(|leftover| rel(&env, &leftover.path) == ".cache/foo")
        .expect("cache leftover offered");
    // Apparent size counts the directory entries' own st_size too (as `du --apparent-size` does).
    let dir_entry_bytes = fs::metadata(env.home.join(".cache/foo"))
        .expect("leftover metadata")
        .len();
    match &leftover.size {
        Size::Measured { apparent, .. } => assert_eq!(*apparent, 1234 + dir_entry_bytes),
        other => panic!("expected a measured size, got {other:?}"),
    }
}

#[test]
fn unreadable_part_of_a_leftover_is_unknown_not_zero() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = serial();
    let env = env();
    file(&env, ".cache/foo/locked/secret", b"x");
    let locked = env.home.join(".cache/foo/locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    if fs::read_dir(&locked).is_ok() {
        // Running as root: permissions do not block the read, so there is nothing to prove.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");
        return;
    }
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);

    let preview = run_preview(&env, &inv, "foo");

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");
    let leftover = preview
        .leftovers
        .iter()
        .find(|leftover| rel(&env, &leftover.path) == ".cache/foo")
        .expect("cache leftover offered");
    assert!(
        !matches!(leftover.size, Size::Measured { .. }),
        "an unreadable subtree must not be reported as a complete measurement: {:?}",
        leftover.size
    );
}

#[test]
fn names_with_path_syntax_match_no_installed_app() {
    let _serial = serial();
    let env = env();
    let inv = inventory(vec![section("pacman explicit", vec![entry("foo")])]);

    for query in ["../../etc", "", "/"] {
        let result = preview(&owner(&env), &env.paths, &inv, query);
        let error = result.expect_err("path-like query must not resolve to an app");
        assert!(
            error.contains("no installed app named"),
            "query {query:?}: {error}"
        );
    }
}

#[test]
fn render_escapes_control_characters_and_shows_both_sizes() {
    let preview = Preview {
        app: "Evil\u{1b}[2J".to_owned(),
        sources: vec!["pacman explicit"],
        match_keys: vec!["evil".to_owned()],
        leftovers: vec![Leftover {
            path: PathBuf::from("/home/u/.cache/evil\u{1b}[31m"),
            class: Class::Regenerable,
            size: Size::Measured {
                apparent: 1500,
                allocated: 4096,
            },
        }],
        refused: vec![Refusal {
            path: PathBuf::from("/home/u/.config/x\u{7}"),
            reason: "bad\u{1b}".to_owned(),
        }],
        notes: Vec::new(),
    };

    let text = render(&preview);

    assert!(
        !text.contains('\u{1b}'),
        "raw escape reached output: {text:?}"
    );
    assert!(!text.contains('\u{7}'), "raw bell reached output: {text:?}");
    assert!(text.contains("apparent 1.5 kB, allocated 4.0 kB"), "{text}");
    assert!(text.contains("nothing is removed"), "{text}");
}

#[test]
fn uninstall_dry_run_with_unknown_name_fails_and_names_the_problem() {
    let output = home_support::output(&["uninstall", "--dry-run", "no-such-app-xyz"]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no installed app named"),
        "stderr: {stderr}"
    );
}

// Characterization, not a fix: exact-key matching already excludes these, so it passes on
// the first run. It pins the negative half of the upstream versioned-IDE contract.
#[test]
fn similarly_named_products_and_backups_are_not_matched() {
    let _serial = serial();
    let env = env();
    dir(&env, ".cache/PyCharmTools");
    dir(&env, ".cache/PyCharm-backup");
    dir(&env, ".config/PyCharm2026.2-backup");
    let inv = inventory(vec![section("pacman explicit", vec![entry("pycharm")])]);

    let preview = run_preview(&env, &inv, "PyCharm");

    assert!(
        offered(&env, &preview).is_empty(),
        "{:?}",
        offered(&env, &preview)
    );
    assert!(
        refused(&env, &preview).is_empty(),
        "{:?}",
        refused(&env, &preview)
    );
}
