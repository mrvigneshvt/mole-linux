use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use mole_linux::commands::analyze_delete::trash_confirmed;
use mole_linux::core::oplog::OpLog;
use mole_linux::core::paths::XdgPaths;
use mole_linux::core::sink::{DeleteMode, Identity, SinkContext, delete};
use mole_linux::core::whitelist::Whitelist;

#[path = "honeypot/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

fn sink_context(home: &Path) -> SinkContext {
    let xdg = XdgPaths::from_values(home.to_path_buf(), None, None, None, None)
        .expect("absolute temporary HOME");
    let whitelist = Whitelist::load(&xdg).expect("empty temporary whitelist");
    SinkContext::new(xdg, whitelist)
}

#[test]
fn confirmed_paths_are_trashed_and_unconfirmed_paths_and_canaries_survive() {
    let temp = tempfile::tempdir().expect("temporary honeypot");
    let home = temp.path().join("home");
    let fixture = fixtures::HomeFixture::build(&home).expect("build honeypot home");
    let selected = home.join("Downloads/large.bin");
    let declined = home.join("Downloads/keep.bin");
    fs::create_dir_all(selected.parent().expect("parent")).expect("downloads");
    fs::write(&selected, b"selected payload").expect("selected file");
    fs::write(&declined, b"declined payload").expect("declined file");

    let preview = vec![
        (
            selected.clone(),
            Identity::capture(&selected).expect("identity"),
        ),
        (
            declined.clone(),
            Identity::capture(&declined).expect("identity"),
        ),
    ];
    let confirmed = vec![selected.clone()];
    let oplog = OpLog::with_state_home(&home.join(".local/state"));
    let results =
        trash_confirmed(preview, &confirmed, &sink_context(&home), &oplog).expect("delete session");

    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok(), "selected path should be trashed");
    assert!(!selected.exists());
    assert_eq!(
        fs::read(&declined).expect("declined path survives"),
        b"declined payload"
    );
    fixture
        .assert_canaries_intact()
        .expect("all honeypot canaries survive");
    let trash_files = home.join(".local/share/Trash/files");
    assert!(
        fs::read_dir(trash_files)
            .expect("Trash files")
            .next()
            .is_some()
    );
    let operations = fs::read_to_string(home.join(".local/state/mole-linux/operations.log"))
        .expect("operation session logged");
    assert!(operations.contains("trash applied"));
    assert!(operations.contains(&selected.display().to_string()));
    assert!(!operations.contains(&declined.display().to_string()));
}

#[test]
fn confirmation_matches_paths_and_not_preview_indexes() {
    let temp = tempfile::tempdir().expect("temporary home");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let target = home.join("candidate");
    fs::write(&target, b"payload").expect("target");
    let preview = vec![(
        target.clone(),
        Identity::capture(&target).expect("identity"),
    )];
    let oplog = OpLog::with_state_home(&home.join("state"));

    let results = trash_confirmed(
        preview,
        &[PathBuf::from("/unrelated/path")],
        &sink_context(&home),
        &oplog,
    )
    .expect("empty confirmation ends cleanly");

    assert!(results.is_empty());
    assert_eq!(
        fs::read(target).expect("unconfirmed target survives"),
        b"payload"
    );
}

#[test]
fn bind_mount_boundary_honeypot() {
    let Ok(parent_value) = std::env::var("MOLE_HONEYPOT_MOUNT_PARENT") else {
        return;
    };
    let parent = PathBuf::from(parent_value);
    let home = PathBuf::from(std::env::var_os("HOME").expect("honeypot HOME"));
    let trash_parent = parent.join("trash-parent");
    let permanent_parent = parent.join("permanent-parent");
    let mounted_canary = trash_parent.join("mounted/canary");
    let permanent_canary = permanent_parent.join("mounted/canary");
    assert_eq!(
        fs::read(&mounted_canary).expect("mounted canary"),
        b"bind-mount-canary"
    );
    assert_eq!(
        fs::read(&permanent_canary).expect("Permanent mounted canary"),
        b"bind-mount-canary"
    );
    assert_eq!(
        fs::metadata(&trash_parent)
            .expect("Trash parent metadata")
            .dev(),
        fs::metadata(trash_parent.join("mounted"))
            .expect("mounted directory metadata")
            .dev(),
        "bind mount must share the parent device"
    );
    let context = sink_context(&home);

    let trash_identity = Identity::capture(&trash_parent).expect("capture Trash target");
    let trashed_path = delete(&trash_parent, &trash_identity, DeleteMode::Trash, &context)
        .expect("Trash moves containing directory without descending");
    assert_eq!(
        fs::read(trashed_path.join("mounted/canary")).expect("Trash preserves mounted canary"),
        b"bind-mount-canary"
    );

    let permanent_identity =
        Identity::capture(&permanent_parent).expect("capture permanent target");
    assert!(
        delete(
            &permanent_parent,
            &permanent_identity,
            DeleteMode::Permanent,
            &context
        )
        .is_err()
    );
    assert_eq!(
        fs::read(&permanent_canary).expect("Permanent preserves mounted canary"),
        b"bind-mount-canary"
    );
}
