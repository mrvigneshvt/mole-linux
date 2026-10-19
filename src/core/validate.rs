use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use super::paths::XdgPaths;
use super::whitelist::Whitelist;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    Empty,
    Relative,
    ParentTraversal,
    ControlByte,
    OutsideHome,
    CriticalRoot(PathBuf),
    Protected(PathBuf),
    SymlinkAncestor { path: PathBuf, resolved: PathBuf },
    SymlinkTargetProtected { path: PathBuf, resolved: PathBuf },
    Unverifiable(PathBuf),
}

pub fn validate_path_for_deletion(
    path: &Path,
    home: &Path,
    xdg: &XdgPaths,
) -> Result<(), ValidationError> {
    validate_path_for_deletion_inner(path, home, xdg, true)
}

pub(crate) fn validate_path_for_sink(
    path: &Path,
    home: &Path,
    xdg: &XdgPaths,
) -> Result<(), ValidationError> {
    validate_path_for_deletion_inner(path, home, xdg, false)
}

fn validate_path_for_deletion_inner(
    path: &Path,
    home: &Path,
    xdg: &XdgPaths,
    check_whitelist: bool,
) -> Result<(), ValidationError> {
    // Ported from tw93/Mole lib/core/file_ops.sh:validate_path_for_deletion (GPL-3.0).
    if path.as_os_str().is_empty() {
        return Err(ValidationError::Empty);
    }
    if !path.is_absolute() {
        return Err(ValidationError::Relative);
    }
    if contains_control(path.as_os_str()) {
        return Err(ValidationError::ControlByte);
    }
    let normalized = lexical_normalize(path);
    let normalized_home = lexical_normalize(home);
    if !is_strict_descendant(&normalized, &normalized_home) {
        return Err(ValidationError::OutsideHome);
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(ValidationError::ParentTraversal);
    }

    check_policy(&normalized, home, xdg, check_whitelist)?;
    let canonical_home = home
        .canonicalize()
        .map_err(|_| ValidationError::Unverifiable(home.to_path_buf()))?;
    let parent = normalized.parent().unwrap_or_else(|| Path::new("/"));
    match parent.canonicalize() {
        Ok(resolved_parent) => {
            let resolved = resolved_parent.join(normalized.file_name().unwrap_or(OsStr::new("")));
            if !is_strict_descendant(&resolved, &canonical_home) {
                return Err(ValidationError::OutsideHome);
            }
            if resolved_parent != parent {
                match check_policy(&resolved, home, xdg, check_whitelist) {
                    Ok(()) => {}
                    Err(error @ ValidationError::Unverifiable(_)) => return Err(error),
                    Err(_) => {
                        return Err(ValidationError::SymlinkAncestor {
                            path: normalized,
                            resolved,
                        });
                    }
                }
            }
        }
        Err(_) => return Err(ValidationError::Unverifiable(parent.to_path_buf())),
    }

    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match path.canonicalize() {
            Ok(resolved) if check_policy(&resolved, home, xdg, check_whitelist).is_err() => {
                return Err(ValidationError::SymlinkTargetProtected {
                    path: path.to_path_buf(),
                    resolved,
                });
            }
            Ok(_) => {}
            Err(_) => return Err(ValidationError::Unverifiable(path.to_path_buf())),
        },
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(ValidationError::Unverifiable(path.to_path_buf())),
    }
    Ok(())
}

fn is_strict_descendant(path: &Path, parent: &Path) -> bool {
    path != parent && path.starts_with(parent)
}

fn check_policy(
    path: &Path,
    home: &Path,
    xdg: &XdgPaths,
    check_whitelist: bool,
) -> Result<(), ValidationError> {
    let normalized = lexical_normalize(path);
    let critical = [
        "/", "/bin", "/sbin", "/lib", "/lib64", "/usr", "/etc", "/boot", "/var", "/opt", "/srv",
        "/root", "/home", "/proc", "/sys", "/dev", "/run", "/mnt", "/media",
    ];
    for root in critical {
        let root_path = Path::new(root);
        if normalized == root_path
            || (root_path != Path::new("/")
                && root_path != Path::new("/home")
                && normalized.starts_with(root_path))
        {
            return Err(ValidationError::CriticalRoot(root_path.to_path_buf()));
        }
    }
    if normalized == lexical_normalize(home)
        || xdg
            .roots()
            .iter()
            .any(|root| normalized == lexical_normalize(root))
    {
        return Err(ValidationError::Protected(normalized));
    }

    if is_protected_user_path(&normalized, home, xdg) {
        return Err(ValidationError::Protected(normalized));
    }
    if normalized
        .components()
        .any(|part| matches!(part.as_os_str().to_str(), Some("models" | "volumes")))
    {
        return Err(ValidationError::Protected(normalized));
    }
    if let Some(name) = normalized.file_name().and_then(OsStr::to_str) {
        let lower = name.to_ascii_lowercase();
        if lower == ".env" || lower.starts_with(".env.") {
            return Err(ValidationError::Protected(normalized));
        }
        if [
            ".db",
            ".sqlite",
            ".sqlite3",
            ".db-wal",
            ".db-shm",
            ".sqlite-wal",
            ".sqlite-shm",
            ".gguf",
            ".safetensors",
            ".pt",
            ".pth",
            ".onnx",
        ]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        {
            return Err(ValidationError::Protected(normalized));
        }
    }
    if normalized
        .components()
        .any(|part| part.as_os_str() == OsStr::new(".git"))
        || path_is_repository_root(&normalized)?
    {
        return Err(ValidationError::Protected(normalized));
    }
    if check_whitelist {
        let whitelist = Whitelist::load(xdg)
            .map_err(|_| ValidationError::Unverifiable(Whitelist::path(xdg)))?;
        if whitelist.is_whitelisted(&normalized) {
            return Err(ValidationError::Protected(normalized));
        }
    }
    Ok(())
}

fn path_is_repository_root(path: &Path) -> Result<bool, ValidationError> {
    match std::fs::symlink_metadata(path.join(".git")) {
        Ok(_) => Ok(true),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(false)
        }
        Err(_) => Err(ValidationError::Unverifiable(path.to_path_buf())),
    }
}

fn is_protected_user_path(path: &Path, home: &Path, xdg: &XdgPaths) -> bool {
    let protected_trees = [
        home.join(".mozilla"),
        xdg.config.join("chromium"),
        xdg.config.join("google-chrome"),
        xdg.config.join("BraveSoftware"),
        xdg.config.join("microsoft-edge"),
        xdg.config.join("vivaldi"),
        home.join("Documents"),
        home.join("Pictures"),
        home.join("Videos"),
        home.join("Music"),
        home.join("Desktop"),
        home.join(".ssh"),
        home.join(".gnupg"),
        xdg.data.join("keyrings"),
    ];
    if protected_trees
        .iter()
        .any(|root| path == root || path.starts_with(root))
    {
        return true;
    }

    let downloads = home.join("Downloads");
    if path == downloads {
        return true;
    }
    false
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn contains_control(value: &OsStr) -> bool {
    value
        .as_encoded_bytes()
        .iter()
        .any(|byte| *byte < 0x20 || *byte == 0x7f)
}
