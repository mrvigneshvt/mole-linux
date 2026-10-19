// Ported from tw93/Mole cmd/analyze/delete.go:deleteMultiplePathsCmd (GPL-3.0).
use std::io;
use std::path::PathBuf;

use crate::core::oplog::{OpLog, SessionSummary};
use crate::core::sink::{DeleteError, DeleteMode, Identity, SinkContext, delete_batch};

/// Trashes only previewed identities whose complete paths were explicitly confirmed.
pub fn trash_confirmed(
    preview: Vec<(PathBuf, Identity)>,
    confirmed_paths: &[PathBuf],
    context: &SinkContext,
    oplog: &OpLog,
) -> io::Result<Vec<(PathBuf, Result<PathBuf, DeleteError>)>> {
    let mut entries = preview
        .into_iter()
        .filter(|(path, _)| confirmed_paths.contains(path))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries.dedup_by(|left, right| left.0 == right.0);
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let item_count = u64::try_from(entries.len())
        .map_err(|_| io::Error::other("selected path count exceeds operation log range"))?;
    let mut session = oplog.start_session("analyze-delete", false)?;
    let results = delete_batch(entries, DeleteMode::Trash, context, || false);
    for (path, result) in &results {
        let (outcome, action) = match result {
            Ok(_) => ("applied", "trash"),
            Err(
                DeleteError::Cancelled | DeleteError::IdentityChanged | DeleteError::Whitelisted,
            ) => ("skipped", "trash"),
            Err(
                DeleteError::InvalidPath(_)
                | DeleteError::CrossFilesystemTrash
                | DeleteError::MountIdUnavailable,
            ) => ("skipped", "trash"),
            Err(
                DeleteError::Partial(_) | DeleteError::TrashMetadataCleanup(_) | DeleteError::Io(_),
            ) => ("failed", "trash"),
        };
        session.record(action, outcome, path)?;
    }
    session.end(&SessionSummary {
        items: item_count,
        size: format!("{item_count} paths"),
    })?;
    Ok(results)
}
