//! Editor caches (M5.4): VS Code, VS Code Insiders and Cursor `CachedData`, and extension
//! directories an editor marked obsolete. Both are owner-gated: a running or unknown editor keeps
//! its candidates unselected. Editor settings and installed extensions are never offered.
// Ported from tw93/Mole lib/clean/app_caches.sh: clean_editor_obsolete_extensions and the
// CachedData lines of clean_code_editors (GPL-3.0). Linux config and home roots only.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use super::clean::{
    Candidate, Category, CleanRule, Discovery, DiscoveryStatus, RecoveryMode, RuleContext,
    process_gated,
};

/// Obsolete-extension manifests are small JSON objects; a larger file is refused unread.
const OBSOLETE_LIMIT: u64 = 1024 * 1024;

/// (config directory under XDG config, editor process name, label)
const CACHED_DATA_EDITORS: &[(&str, &str, &str)] = &[
    ("Code", "code", "VS Code CachedData"),
    (
        "Code - Insiders",
        "code-insiders",
        "VS Code Insiders CachedData",
    ),
    ("Cursor", "cursor", "Cursor CachedData"),
];

/// (extensions root under HOME, editor process name, label)
const EXTENSION_ROOTS: &[(&str, &str, &str)] = &[
    (".vscode/extensions", "code", "VS Code"),
    (
        ".vscode-insiders/extensions",
        "code-insiders",
        "VS Code Insiders",
    ),
    (".cursor/extensions", "cursor", "Cursor"),
];

pub struct EditorCachedDataRule;

impl CleanRule for EditorCachedDataRule {
    fn id(&self) -> &str {
        "editor-cached-data"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        for (directory, process, label) in CACHED_DATA_EDITORS {
            let root = ctx.xdg.config.join(directory).join("CachedData");
            let entries = match fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    issues.push(format!("{}: {error}", root.display()));
                    continue;
                }
            };
            for entry in entries {
                let path = match entry {
                    Ok(entry) => entry.path(),
                    Err(error) => {
                        issues.push(format!("{}: {error}", root.display()));
                        continue;
                    }
                };
                match fs::symlink_metadata(&path) {
                    // A symlink is never followed into or deleted through.
                    Ok(metadata) if metadata.file_type().is_symlink() => {}
                    Ok(_) => match process_gated(
                        &path,
                        format!("{label} · regenerated on next launch"),
                        RecoveryMode::Regenerate,
                        process,
                        ctx,
                    ) {
                        Ok(item) => candidates.push(item),
                        Err(error) => issues.push(format!("{}: {error}", path.display())),
                    },
                    Err(error) => issues.push(format!("{}: {error}", path.display())),
                }
            }
        }
        finish(candidates, issues)
    }
}

pub struct EditorObsoleteExtensionsRule;

impl CleanRule for EditorObsoleteExtensionsRule {
    fn id(&self) -> &str {
        "editor-obsolete-extensions"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        for (relative, process, label) in EXTENSION_ROOTS {
            let root = ctx.home.join(relative);
            let keys = match obsolete_keys(&root.join(".obsolete")) {
                Ok(Some(keys)) => keys,
                Ok(None) => continue,
                Err(reason) => {
                    issues.push(format!("{}: {reason}", root.join(".obsolete").display()));
                    continue;
                }
            };
            let families = installed_families(&root, &keys);
            for key in keys {
                let path = root.join(&key);
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if !metadata.is_dir() {
                    continue;
                }
                // Only an obsolete version with a non-obsolete sibling of the same extension id is
                // offered, so the editor keeps one working copy. Ids are split at the first `-<digit>`.
                if !family(&key).is_some_and(|id| families.contains(id)) {
                    continue;
                }
                match process_gated(
                    &path,
                    format!("Obsolete {label} extension · newer version installed"),
                    RecoveryMode::Trash,
                    process,
                    ctx,
                ) {
                    Ok(item) => candidates.push(item),
                    Err(error) => issues.push(format!("{}: {error}", path.display())),
                }
            }
        }
        finish(candidates, issues)
    }
}

/// Every rule this module contributes to the catalog.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    vec![
        Box::new(EditorCachedDataRule),
        Box::new(EditorObsoleteExtensionsRule),
    ]
}

fn finish(candidates: Vec<Candidate>, issues: Vec<String>) -> Discovery {
    let status = if issues.is_empty() {
        DiscoveryStatus::Complete
    } else {
        DiscoveryStatus::Partial(issues.join("; "))
    };
    Discovery { candidates, status }
}

/// Reads the editor's `.obsolete` manifest: a JSON object whose `true` keys name extension
/// directories. Absent is `None`. A symlink, an oversized file, or malformed JSON is an error.
/// Keys that are not plain child names are dropped, so none can escape the extensions root.
fn obsolete_keys(file: &Path) -> Result<Option<Vec<String>>, String> {
    let metadata = match fs::symlink_metadata(file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() {
        return Err("not a regular file".to_owned());
    }
    if metadata.len() > OBSOLETE_LIMIT {
        return Err(format!("larger than {OBSOLETE_LIMIT} bytes"));
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|handle| handle.take(OBSOLETE_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    let read = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
    if read > OBSOLETE_LIMIT {
        return Err(format!("larger than {OBSOLETE_LIMIT} bytes"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("malformed JSON: {error}"))?;
    let map = value
        .as_object()
        .ok_or_else(|| "expected a JSON object".to_owned())?;
    Ok(Some(
        map.iter()
            .filter(|(_, flag)| flag.as_bool() == Some(true))
            .map(|(key, _)| key.clone())
            .filter(|key| is_plain_child_name(key))
            .collect(),
    ))
}

fn is_plain_child_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}

// ponytail: ids are split at the first `-<digit>`, so `some-2d-1.0` and `some-3d-1.0` share the id `some`. An obsolete `some-2d` can then be offered because `some-3d` is installed. Upgrade: read the extension id from its package.json. Logged in KNOWN-GAPS.md.
/// The extension id of a directory name: everything before the first `-` followed by a digit.
/// `ms-python.python-2024.1.0-linux-x64` gives `ms-python.python`. Names with no version give None.
fn family(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    (1..bytes.len())
        .find(|&index| bytes[index - 1] == b'-' && bytes[index].is_ascii_digit())
        .map(|index| &name[..index - 1])
}

/// Extension ids with at least one installed, non-obsolete, real directory in `root`.
fn installed_families(root: &Path, obsolete: &[String]) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| !name.starts_with('.') && !obsolete.contains(name))
        .filter_map(|name| family(&name).map(str::to_owned))
        .collect()
}
