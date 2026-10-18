use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CANARY_MANIFEST: &str = include_str!("canary-manifest.txt");

const DIRECTORIES: &[&str] = &[
    ".config",
    ".cache",
    ".local/share",
    ".local/state",
    ".mozilla/firefox/profile",
    ".npm/_cacache",
    ".cache/pnpm/store/v3",
    ".cache/uv/archive-v0",
    ".cargo/registry/src",
    ".gradle/caches/modules-2",
    "project/node_modules",
    "project/target",
    "project/.git/objects",
    "project/.git/worktrees/linked",
    "Downloads",
    ".local/share/Trash/files",
    ".local/share/Trash/info",
    "browser/Default/Cache",
];

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
        for relative in DIRECTORIES {
            fs::create_dir_all(root.join(relative))?;
        }
        fs::write(
            root.join("Downloads/installer.AppImage"),
            b"appimage fixture",
        )?;
        fs::write(
            root.join(".local/share/Trash/files/old"),
            b"trashed file fixture",
        )?;
        for relative in canary_paths() {
            let path = root.join(&relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let contents = if relative.ends_with(".db") {
                format!("SQLite format 3\0mole-linux protected canary: {relative}\n")
            } else {
                format!("mole-linux protected canary: {relative}\n")
            };
            fs::write(path, contents)?;
        }
        // Separate SQLite fixture files keep the browser database, WAL and SHM together.
        fs::write(
            root.join("project/.git/config"),
            "[core]\n\trepositoryformatversion = 0\n",
        )?;
        fs::write(root.join("project/.git/HEAD"), "ref: refs/heads/main\n")?;
        fs::write(
            root.join("project/.git/worktrees/linked/gitdir"),
            "../../../../project-linked/.git\n",
        )?;
        fs::create_dir_all(root.join("project-linked"))?;
        fs::write(
            root.join("project-linked/.git"),
            "gitdir: ../project/.git/worktrees/linked\n",
        )?;

        let mut canaries = BTreeMap::new();
        for relative in canary_paths() {
            let path = PathBuf::from(relative);
            canaries.insert(path.clone(), checksum(&fs::read(root.join(&path))?));
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

fn canary_paths() -> impl Iterator<Item = String> {
    CANARY_MANIFEST
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
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
