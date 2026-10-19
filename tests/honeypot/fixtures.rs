use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = include_str!("manifest.tsv");

const TOOLS: &[&str] = &[
    "pacman",
    "paccache",
    "journalctl",
    "coredumpctl",
    "flatpak",
    "docker",
    "sudo",
    "npm",
    "pnpm",
    "uv",
    "cargo",
    "go",
    "gradle",
];

pub struct HomeFixture {
    root: PathBuf,
    canaries: BTreeMap<PathBuf, u64>,
}

impl HomeFixture {
    pub fn build(root: &Path) -> io::Result<Self> {
        let temp_root = std::env::temp_dir().canonicalize()?;
        let parent = root
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "HOME needs a parent"))?
            .canonicalize()?;
        if !parent.starts_with(&temp_root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "honeypot HOME must be inside the system temporary directory",
            ));
        }
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "honeypot HOME must be a real directory",
                ));
            }
            Ok(_) => {
                if fs::read_dir(root)?.next().transpose()?.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "honeypot HOME must be empty",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir_all(root)?;
        let entries = manifest_entries()?;
        for (kind, relative) in &entries {
            let path = root.join(relative);
            match kind {
                EntryKind::Directory => fs::create_dir_all(path)?,
                EntryKind::GitRepository | EntryKind::Worktree => {}
                EntryKind::GitTrack => {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(path, file_contents(*kind, relative))?;
                }
                EntryKind::Canary
                | EntryKind::SqliteCanary
                | EntryKind::TargetFile
                | EntryKind::AuthoredFile
                | EntryKind::CargoManifest => {
                    if is_canary(*kind) && is_git_generated_canary(&entries, relative) {
                        continue;
                    }
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(path, file_contents(*kind, relative))?;
                }
            }
        }
        initialize_git_fixture(root, &entries)?;

        let mut canaries = BTreeMap::new();
        for (kind, relative) in entries {
            if is_canary(kind) || kind == EntryKind::GitTrack {
                let path = PathBuf::from(relative);
                canaries.insert(path.clone(), checksum(&fs::read(root.join(&path))?));
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            canaries,
        })
    }

    pub fn assert_canaries_intact(&self) -> io::Result<()> {
        for (relative, expected) in &self.canaries {
            let actual = checksum(&fs::read(self.root.join(relative))?);
            if actual != *expected {
                return Err(io::Error::other(format!(
                    "canary changed: {}",
                    relative.display()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Directory,
    Canary,
    SqliteCanary,
    TargetFile,
    AuthoredFile,
    CargoManifest,
    GitRepository,
    GitTrack,
    Worktree,
}

fn manifest_entries() -> io::Result<Vec<(EntryKind, String)>> {
    MANIFEST
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|(index, line)| {
            let (kind, relative) = line.split_once('\t').ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid honeypot manifest row {}", index + 1),
                )
            })?;
            let path = Path::new(relative);
            if relative.is_empty()
                || relative.contains('\0')
                || path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsafe honeypot manifest path on row {}", index + 1),
                ));
            }
            let kind = match kind {
                "dir" => EntryKind::Directory,
                "canary" => EntryKind::Canary,
                "sqlite-canary" => EntryKind::SqliteCanary,
                "target-file" => EntryKind::TargetFile,
                "authored-file" => EntryKind::AuthoredFile,
                "cargo-manifest" => EntryKind::CargoManifest,
                "git-repo" => EntryKind::GitRepository,
                "git-track" => EntryKind::GitTrack,
                "worktree" => EntryKind::Worktree,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown honeypot manifest kind on row {}", index + 1),
                    ));
                }
            };
            Ok((kind, relative.to_owned()))
        })
        .collect()
}

fn file_contents(kind: EntryKind, relative: &str) -> String {
    match kind {
        EntryKind::GitTrack => "fn main() { println!(\"fixture project\"); }\n".to_owned(),
        EntryKind::SqliteCanary => format!("SQLite format 3\0mole-linux canary: {relative}\n"),
        EntryKind::AuthoredFile => "// authored local package, deliberately untracked\n".to_owned(),
        EntryKind::CargoManifest => {
            "[package]\nname = \"honeypot-target\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
                .to_owned()
        }
        _ => format!("mole-linux fixture file: {relative}\n"),
    }
}

fn is_canary(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Canary | EntryKind::SqliteCanary)
}

fn initialize_git_fixture(root: &Path, entries: &[(EntryKind, String)]) -> io::Result<()> {
    let repository = entry_path(entries, EntryKind::GitRepository)?;
    let tracked_files = entries
        .iter()
        .filter(|(kind, _)| *kind == EntryKind::GitTrack)
        .map(|(_, path)| path.as_str())
        .collect::<Vec<_>>();
    let worktree = entry_path(entries, EntryKind::Worktree)?;
    let repository_path = Path::new(repository);
    let mut tracked_relatives = Vec::new();
    for tracked_file in tracked_files {
        let tracked_relative = Path::new(tracked_file)
            .strip_prefix(repository_path)
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "git track file is outside repo")
            })?;
        let tracked_relative = tracked_relative
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 tracked path"))?;
        tracked_relatives.push(tracked_relative.to_owned());
    }

    run_git(root, &["-c", "init.defaultBranch=main", "init", repository])?;
    run_git(
        root,
        &["-C", repository, "config", "user.name", "Honeypot Fixture"],
    )?;
    run_git(
        root,
        &[
            "-C",
            repository,
            "config",
            "user.email",
            "honeypot@example.invalid",
        ],
    )?;
    for tracked_relative in &tracked_relatives {
        run_git(root, &["-C", repository, "add", tracked_relative])?;
    }
    run_git(
        root,
        &["-C", repository, "commit", "-m", "seed honeypot project"],
    )?;
    let worktree_absolute = root.join(worktree);
    let worktree_absolute = worktree_absolute
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 worktree path"))?;
    run_git(
        root,
        &[
            "-C",
            repository,
            "worktree",
            "add",
            "-b",
            "linked",
            worktree_absolute,
        ],
    )
}

fn entry_path(entries: &[(EntryKind, String)], kind: EntryKind) -> io::Result<&str> {
    entries
        .iter()
        .find(|(entry_kind, _)| *entry_kind == kind)
        .map(|(_, path)| path.as_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Git manifest entry"))
}

fn is_git_generated_canary(entries: &[(EntryKind, String)], relative: &str) -> bool {
    let Ok(repository) = entry_path(entries, EntryKind::GitRepository) else {
        return false;
    };
    let Ok(worktree) = entry_path(entries, EntryKind::Worktree) else {
        return false;
    };
    let Some(worktree_name) = Path::new(worktree).file_name() else {
        return false;
    };
    relative == format!("{repository}/.git/HEAD")
        || relative
            == format!(
                "{repository}/.git/worktrees/{}/HEAD",
                worktree_name.to_string_lossy()
            )
}

fn run_git(root: &Path, arguments: &[&str]) -> io::Result<()> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env("HOME", root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn checksum(bytes: &[u8]) -> u64 {
    // FNV-1a gives a deterministic corruption check without adding a runtime dependency.
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn generate_tool_shims(bin: &Path, log: &Path) -> io::Result<()> {
    fs::create_dir_all(bin)?;
    let log = shell_quote(&log.to_string_lossy());
    for tool in TOOLS {
        let script = format!(
            "#!/bin/sh\nprintf '%s' '{}' >> {}\nfor arg in \"$@\"; do printf '\\t%s' \"$arg\" >> {}; done\nprintf '\\n' >> {}\nprintf '%s\\n' 'mole-linux-shim {}'\n",
            tool, log, log, log, tool
        );
        let path = bin.join(tool);
        fs::write(&path, script)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
