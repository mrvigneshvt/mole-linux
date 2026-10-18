use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "replacement path has no filename",
        )
    })?;
    let existing_mode = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to replace a symbolic link directly",
            ));
        }
        Ok(metadata) => Some(metadata.permissions().mode() & 0o7777),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let (temporary_path, mut temporary_file) = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_name = format!(".mole-linux-{}-{sequence}.tmp", std::process::id());
        let candidate = parent.join(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    let operation = (|| {
        if let Some(mode) = existing_mode {
            temporary_file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        temporary_file.write_all(bytes)?;
        temporary_file.sync_all()?;
        drop(temporary_file);
        fs::rename(&temporary_path, path)
    })();

    if let Err(error) = operation {
        match fs::remove_file(&temporary_path) {
            Ok(()) => {}
            Err(cleanup_error) if cleanup_error.kind() == io::ErrorKind::NotFound => {}
            Err(cleanup_error) => {
                return Err(io::Error::other(format!(
                    "{error}; also failed to remove temporary file {temporary_path:?}: {cleanup_error}",
                )));
            }
        }
        return Err(error);
    }
    Ok(())
}
