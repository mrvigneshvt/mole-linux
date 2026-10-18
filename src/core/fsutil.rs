use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Resolve every symlink in the final path while allowing a missing, unlinked target.
pub fn resolve_symlink_chain(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    let mut followed_link = false;
    let mut seen = std::collections::HashSet::new();

    loop {
        let parent = current.parent().unwrap_or_else(|| Path::new("."));
        let Some(name) = current.file_name() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "shell config path has no filename",
            ));
        };
        let canonical_parent = match fs::canonicalize(parent) {
            Ok(parent) => parent,
            Err(error) if error.kind() == io::ErrorKind::NotFound && !followed_link => {
                return Ok(current);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("broken symbolic link in shell config chain: {current:?}"),
                ));
            }
            Err(error) => return Err(error),
        };
        current = canonical_parent.join(name);

        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound && followed_link => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("broken symbolic link in shell config chain: {current:?}"),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_symlink() {
            return Ok(current);
        }
        if !seen.insert(current.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cyclic symbolic link in shell config chain: {current:?}"),
            ));
        }
        let target = fs::read_link(&current)?;
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(target)
        };
        followed_link = true;
    }
}

/// Replace a regular file by writing a same-directory temporary file and renaming it over the target.
pub fn atomic_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    super::sink::atomic_replace(path, bytes)
}
