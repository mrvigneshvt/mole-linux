// Ported from tw93/Mole lib/core/file_ops.sh:mole_delete (GPL-3.0)
// and cmd/analyze/delete.go:deleteMultiplePathsCmd (GPL-3.0).
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustix::fs::{self as rfs, AtFlags, Mode as FileMode, OFlags, RenameFlags, StatxFlags};

use super::paths::XdgPaths;
use super::validate::{ValidationError, validate_path_for_deletion};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteMode {
    DryRun,
    Trash,
    Permanent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    device: u64,
    inode: u64,
    file_type: u32,
    mtime: i64,
    mtime_nsec: u64,
    mount_id: Option<u64>,
}

impl Identity {
    pub fn capture(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            file_type: metadata.mode() & 0o170000,
            mtime: metadata.mtime(),
            mtime_nsec: u64::try_from(metadata.mtime_nsec()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid nanosecond timestamp")
            })?,
            mount_id: mount_id_at(rfs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)?,
        })
    }

    fn matches(&self, path: &Path) -> io::Result<bool> {
        Ok(self == &Self::capture(path)?)
    }
}

#[derive(Debug)]
pub enum DeleteError {
    InvalidPath(ValidationError),
    IdentityChanged,
    Cancelled,
    Partial(io::Error),
    MountIdUnavailable,
    CrossFilesystemTrash,
    TrashMetadataCleanup(io::Error),
    Io(io::Error),
}

impl std::fmt::Display for DeleteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(error) => write!(f, "path rejected by deletion policy: {error:?}"),
            Self::IdentityChanged => f.write_str("target identity changed after preview; skipped"),
            Self::Cancelled => f.write_str("batch cancelled before this item"),
            Self::Partial(error) => write!(f, "partial permanent removal: {error}"),
            Self::MountIdUnavailable => {
                f.write_str("mount ID unavailable; refusing permanent directory removal")
            }
            Self::CrossFilesystemTrash => {
                f.write_str("Trash is on another filesystem; refusing Trash move")
            }
            Self::TrashMetadataCleanup(error) => {
                write!(f, "failed to remove Trash metadata: {error}")
            }
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DeleteError {}

impl From<io::Error> for DeleteError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Revalidates policy and identity at the only destructive boundary.
pub fn delete(
    path: &Path,
    expected: &Identity,
    mode: DeleteMode,
    home: &Path,
    xdg: &XdgPaths,
) -> Result<PathBuf, DeleteError> {
    validate_path_for_deletion(path, home, xdg).map_err(DeleteError::InvalidPath)?;
    if !expected.matches(path).map_err(DeleteError::Io)? {
        return Err(DeleteError::IdentityChanged);
    }
    if mode == DeleteMode::DryRun {
        return Ok(path.to_path_buf());
    }
    let parent = path.parent().ok_or_else(|| {
        DeleteError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent",
        ))
    })?;
    let name = path.file_name().ok_or_else(|| {
        DeleteError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no name",
        ))
    })?;
    let parent_fd = open_source_parent(parent)?;

    // The held parent fd anchors all mutation; the target is looked up again
    // relative to it immediately before renameat2/unlinkat.
    let current =
        rfs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
    let current_mount_id = mount_id_at(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if current.st_dev != expected.device
        || current.st_ino != expected.inode
        || current.st_mode & 0o170000 != expected.file_type
        || current.st_mtime != expected.mtime
        || current.st_mtime_nsec != expected.mtime_nsec
        || current_mount_id != expected.mount_id
    {
        return Err(DeleteError::IdentityChanged);
    }

    match mode {
        DeleteMode::DryRun => Ok(path.to_path_buf()),
        DeleteMode::Trash => move_to_trash(path, name, &parent_fd, home, expected),
        DeleteMode::Permanent => {
            if current.st_mode & 0o170000 == 0o040000 {
                let root_mount_id = require_mount_id(expected.mount_id)?;
                let target_fd = rfs::openat(
                    &parent_fd,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    FileMode::empty(),
                )
                .map_err(io::Error::from)?;
                let opened = rfs::fstat(&target_fd).map_err(io::Error::from)?;
                let opened_mount_id = mount_id_at(&target_fd, ".", AtFlags::empty())?;
                if opened.st_dev != expected.device
                    || opened.st_ino != expected.inode
                    || opened_mount_id != Some(root_mount_id)
                {
                    return Err(DeleteError::IdentityChanged);
                }
                remove_tree(&target_fd, expected.device, root_mount_id)
                    .map_err(|error| DeleteError::Partial(io::Error::other(error.to_string())))?;
                let rebound = rfs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(io::Error::from)?;
                if rebound.st_dev != expected.device
                    || rebound.st_ino != expected.inode
                    || rebound.st_mode & 0o170000 != 0o040000
                {
                    return Err(DeleteError::IdentityChanged);
                }
                rfs::unlinkat(&parent_fd, name, AtFlags::REMOVEDIR).map_err(io::Error::from)?;
                return Ok(path.to_path_buf());
            }
            rfs::unlinkat(&parent_fd, name, AtFlags::empty()).map_err(io::Error::from)?;
            Ok(path.to_path_buf())
        }
    }
}

fn remove_tree(directory: &OwnedFd, device: u64, mount_id: u64) -> Result<(), DeleteError> {
    let entries = fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let stat =
            rfs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
        let entry_mount_id = mount_id_at(directory, &name, AtFlags::SYMLINK_NOFOLLOW)?;
        if stat.st_dev != device {
            return Err(DeleteError::Io(io::Error::other(
                "permanent removal refuses to cross a filesystem boundary",
            )));
        }
        let entry_mount_id = require_mount_id(entry_mount_id)?;
        if !mount_boundary_decision(device, Some(mount_id), stat.st_dev, Some(entry_mount_id)) {
            return Err(DeleteError::Io(io::Error::other(
                "permanent removal refuses to cross a mount boundary",
            )));
        }
        if stat.st_mode & 0o170000 == 0o040000 {
            let child = rfs::openat(
                directory,
                &name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                FileMode::empty(),
            )
            .map_err(io::Error::from)?;
            let opened = rfs::fstat(&child).map_err(io::Error::from)?;
            let opened_mount_id = mount_id_at(&child, ".", AtFlags::empty())?;
            if opened.st_dev != stat.st_dev
                || opened.st_ino != stat.st_ino
                || opened_mount_id != Some(entry_mount_id)
            {
                return Err(DeleteError::IdentityChanged);
            }
            remove_tree(&child, device, mount_id)?;
            let rebound = rfs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(io::Error::from)?;
            if rebound.st_dev != stat.st_dev || rebound.st_ino != stat.st_ino {
                return Err(DeleteError::IdentityChanged);
            }
            rfs::unlinkat(directory, &name, AtFlags::REMOVEDIR).map_err(io::Error::from)?;
        } else {
            rfs::unlinkat(directory, &name, AtFlags::empty()).map_err(io::Error::from)?;
        }
    }
    Ok(())
}

/// Processes deepest paths first; cancellation stops before the next item.
pub fn delete_batch<F>(
    mut entries: Vec<(PathBuf, Identity)>,
    mode: DeleteMode,
    home: &Path,
    xdg: &XdgPaths,
    mut cancelled: F,
) -> Vec<(PathBuf, Result<PathBuf, DeleteError>)>
where
    F: FnMut() -> bool,
{
    entries.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    let mut results = Vec::with_capacity(entries.len());
    let mut pending = entries.into_iter();
    while let Some((path, identity)) = pending.next() {
        if cancelled() {
            results.push((path, Err(DeleteError::Cancelled)));
            for (remaining, _) in pending {
                results.push((remaining, Err(DeleteError::Cancelled)));
            }
            break;
        }
        results.push((path.clone(), delete(&path, &identity, mode, home, xdg)));
    }
    results
}

fn open_dir(path: &Path) -> io::Result<OwnedFd> {
    Ok(rfs::openat(
        rfs::CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        FileMode::empty(),
    )?)
}

fn open_source_parent(path: &Path) -> io::Result<OwnedFd> {
    let mut current = rfs::openat(
        rfs::CWD,
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        FileMode::empty(),
    )?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                current = rfs::openat(
                    &current,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    FileMode::empty(),
                )?;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid parent path",
                ));
            }
        }
    }
    Ok(current)
}

fn move_to_trash(
    path: &Path,
    name: &std::ffi::OsStr,
    parent_fd: &OwnedFd,
    home: &Path,
    identity: &Identity,
) -> Result<PathBuf, DeleteError> {
    let target_dev = identity.device;
    let home_dev = fs::metadata(home)?.dev();
    let (trash, trash_fd) = if target_dev == home_dev {
        let home_fd = open_dir(home)?;
        let local_fd = ensure_child_dir(&home_fd, ".local")?;
        let share_fd = ensure_child_dir(&local_fd, "share")?;
        let trash = ensure_child_dir(&share_fd, "Trash")?;
        (home.join(".local/share/Trash"), trash)
    } else {
        let parent = path.parent().ok_or(DeleteError::CrossFilesystemTrash)?;
        let topdir = filesystem_topdir(parent, target_dev)?;
        let topdir_fd = open_dir(&topdir)?;
        let trash_name = format!(".Trash-{}", rustix::process::getuid().as_raw());
        let trash_fd = ensure_child_dir(&topdir_fd, &trash_name)
            .map_err(|_| DeleteError::CrossFilesystemTrash)?;
        (topdir.join(trash_name), trash_fd)
    };
    let files_fd = ensure_child_dir(&trash_fd, "files")?;
    let info_fd = ensure_child_dir(&trash_fd, "info")?;
    if rfs::fstat(&files_fd).map_err(io::Error::from)?.st_dev != target_dev {
        return Err(DeleteError::CrossFilesystemTrash);
    }

    let base = path.file_name().ok_or_else(|| {
        DeleteError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no name",
        ))
    })?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| DeleteError::Io(io::Error::other(e)))?
        .as_secs();
    for attempt in 0_u32..100 {
        let mut candidate = base.to_os_string();
        if attempt > 0 {
            let mut bytes = candidate.into_vec();
            bytes.extend_from_slice(format!(".{}.{}", stamp, attempt).as_bytes());
            candidate = std::ffi::OsString::from_vec(bytes);
        }
        let mut info_name_bytes = candidate.as_encoded_bytes().to_vec();
        info_name_bytes.extend_from_slice(b".trashinfo");
        let info_name = std::ffi::OsString::from_vec(info_name_bytes);
        let contents = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            percent_path(path),
            utc_timestamp(stamp)
        );
        let info_file = match rfs::openat(
            &info_fd,
            &info_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            FileMode::from_raw_mode(0o600),
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(DeleteError::Io(error.into())),
        };
        let mut file = fs::File::from(info_file);
        use std::io::Write;
        if let Err(error) = file.write_all(contents.as_bytes()) {
            return Err(cleanup_trash_info(
                &info_fd,
                &info_name,
                DeleteError::Io(error),
            ));
        }
        drop(file);
        let rebound = match rfs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(error) => {
                return Err(cleanup_trash_info(
                    &info_fd,
                    &info_name,
                    DeleteError::Io(error.into()),
                ));
            }
        };
        if rebound.st_dev != identity.device
            || rebound.st_ino != identity.inode
            || rebound.st_mtime != identity.mtime
            || rebound.st_mtime_nsec != identity.mtime_nsec
        {
            return Err(cleanup_trash_info(
                &info_fd,
                &info_name,
                DeleteError::IdentityChanged,
            ));
        }
        match rfs::renameat_with(
            parent_fd,
            name,
            &files_fd,
            &candidate,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => return Ok(trash.join("files").join(candidate)),
            Err(error) => {
                let is_collision = error.raw_os_error() == rustix::io::Errno::EXIST.raw_os_error();
                let mapped_error = trash_rename_error(error);
                if matches!(mapped_error, DeleteError::CrossFilesystemTrash) {
                    return Err(cleanup_trash_info(&info_fd, &info_name, mapped_error));
                }
                let error = cleanup_trash_info(&info_fd, &info_name, mapped_error);
                if matches!(error, DeleteError::TrashMetadataCleanup(_)) {
                    return Err(error);
                }
                if is_collision {
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(DeleteError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate collision-free Trash name",
    )))
}

fn cleanup_trash_info(
    directory: &OwnedFd,
    name: &std::ffi::OsStr,
    original: DeleteError,
) -> DeleteError {
    match rfs::unlinkat(directory, name, AtFlags::empty()) {
        Ok(()) => original,
        Err(error) => DeleteError::TrashMetadataCleanup(io::Error::other(format!(
            "{original}; failed to remove Trash metadata: {error}"
        ))),
    }
}

fn ensure_child_dir(parent: &OwnedFd, name: &str) -> io::Result<OwnedFd> {
    if let Err(error) = rfs::mkdirat(parent, name, FileMode::from_raw_mode(0o700))
        && error != rustix::io::Errno::EXIST
    {
        return Err(error.into());
    }
    let directory = rfs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        FileMode::empty(),
    )?;
    let metadata = rfs::fstat(&directory)?;
    if metadata.st_uid != rustix::process::getuid().as_raw() || metadata.st_mode & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Trash directory is unsafe or not owned by current user",
        ));
    }
    Ok(directory)
}

fn mount_id_at<Fd: rustix::fd::AsFd, P: rustix::path::Arg>(
    dirfd: Fd,
    path: P,
    flags: AtFlags,
) -> io::Result<Option<u64>> {
    match rfs::statx(
        dirfd,
        path,
        flags,
        StatxFlags::BASIC_STATS | StatxFlags::MNT_ID,
    ) {
        Ok(stat) if stat.stx_mask & StatxFlags::MNT_ID.bits() != 0 => Ok(Some(stat.stx_mnt_id)),
        Ok(_) => Ok(None),
        Err(error) if error == rustix::io::Errno::NOSYS || error == rustix::io::Errno::INVAL => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn mount_boundary_decision(
    selected_device: u64,
    selected_mount_id: Option<u64>,
    entry_device: u64,
    entry_mount_id: Option<u64>,
) -> bool {
    selected_device == entry_device
        && selected_mount_id.is_some()
        && selected_mount_id == entry_mount_id
}

fn require_mount_id(mount_id: Option<u64>) -> Result<u64, DeleteError> {
    mount_id.ok_or(DeleteError::MountIdUnavailable)
}

fn filesystem_topdir(path: &Path, device: u64) -> Result<PathBuf, DeleteError> {
    let mut topdir = path.canonicalize()?;
    if !topdir.is_dir() {
        topdir.pop();
    }
    loop {
        let Some(parent) = topdir.parent() else {
            return Err(DeleteError::CrossFilesystemTrash);
        };
        if fs::metadata(parent)?.dev() != device {
            return Ok(topdir);
        }
        topdir = parent.to_path_buf();
    }
}

fn percent_path(path: &Path) -> String {
    let mut output = String::new();
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(byte) {
            output.push(char::from(*byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn utc_timestamp(seconds: u64) -> String {
    // ponytail: Keep UTC timestamps without a timezone conversion dependency; upgrade to local
    // time after an approved libc-based timezone conversion is available.
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let z = i128::from(days) + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        day_seconds / 3_600,
        day_seconds % 3_600 / 60,
        day_seconds % 60
    )
}

fn trash_rename_error(error: rustix::io::Errno) -> DeleteError {
    if error == rustix::io::Errno::XDEV {
        DeleteError::CrossFilesystemTrash
    } else {
        DeleteError::Io(error.into())
    }
}

#[cfg(test)]
mod mount_boundary_tests {
    use super::{
        DeleteError, mount_boundary_decision, open_dir, require_mount_id, trash_rename_error,
    };

    #[test]
    fn same_mount_requires_matching_device_and_present_mount_ids() {
        assert!(mount_boundary_decision(7, Some(12), 7, Some(12)));
        assert!(!mount_boundary_decision(7, Some(12), 7, Some(13)));
        assert!(!mount_boundary_decision(7, None, 7, Some(12)));
        assert!(!mount_boundary_decision(7, Some(12), 7, None));
        assert!(!mount_boundary_decision(7, None, 7, None));
        assert!(!mount_boundary_decision(7, Some(12), 8, Some(12)));
    }

    #[test]
    fn unavailable_mount_id_is_not_accepted_for_permanent_directory_removal() {
        assert!(matches!(
            require_mount_id(None),
            Err(DeleteError::MountIdUnavailable)
        ));
        assert_eq!(require_mount_id(Some(12)).expect("present ID"), 12);
    }

    #[test]
    fn cross_device_rename_error_is_reported_as_refused_trash() {
        let error = trash_rename_error(rustix::io::Errno::XDEV);
        assert!(matches!(error, super::DeleteError::CrossFilesystemTrash));
        assert!(error.to_string().contains("another filesystem"));
    }

    #[test]
    fn renameat2_across_mounts_returns_exdev() {
        let source = tempfile::tempdir_in("/tmp").expect("source tempdir");
        let destination = tempfile::tempdir_in("/dev/shm").expect("destination tempdir");
        std::fs::write(source.path().join("item"), "payload").expect("source item");
        let source_fd = open_dir(source.path()).expect("source dir fd");
        let destination_fd = open_dir(destination.path()).expect("destination dir fd");

        let error = rustix::fs::renameat_with(
            &source_fd,
            "item",
            &destination_fd,
            "item",
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .expect_err("rename across mounts must fail");
        assert_eq!(error, rustix::io::Errno::XDEV);
        assert!(matches!(
            trash_rename_error(error),
            DeleteError::CrossFilesystemTrash
        ));
        assert_eq!(
            std::fs::read(source.path().join("item")).expect("source remains"),
            b"payload"
        );
    }
}
