// Ported from tw93/Mole lib/clean/dev.sh:clean_pnpm_stores, is_safe_pnpm_store_path,
// list_installed_pnpm_binaries (GPL-3.0).

use std::io;
use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, resolve_on_path};
use crate::core::outcome::Outcome;
use crate::core::privilege::is_executable_file;

pub struct Pnpm;

impl OwnerAdapter for Pnpm {
    fn label(&self) -> &'static str {
        "pnpm cache"
    }

    fn program(&self) -> &'static str {
        "pnpm"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["pnpm", "pnpm.cjs"]
    }

    /// The PATH pnpm first, then every mise-installed pnpm, so each major prunes its own store.
    fn binaries(&self, env: &AdapterEnv) -> Result<Vec<PathBuf>, String> {
        let mut binaries: Vec<PathBuf> = resolve_on_path(self.program(), &env.path)
            .into_iter()
            .collect();
        let mise_root = env.home.join(".local/share/mise/installs/pnpm");
        let entries = match std::fs::read_dir(&mise_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(binaries),
            Err(error) => return Err(format!("cannot list {}: {error}", mise_root.display())),
        };
        let mut installed = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("cannot list {}: {error}", mise_root.display()))?;
            let candidate = entry.path().join(self.program());
            if is_executable_file(&candidate) {
                installed.push(candidate);
            }
        }
        installed.sort();
        binaries.extend(installed);
        Ok(binaries)
    }

    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let text = query_text(binary, &["store", "path"], env)?;
        if !is_safe_pnpm_store_path(&text) {
            return Err(Refusal {
                outcome: Outcome::Unavailable,
                reason: format!("unsafe or empty pnpm store path: {text:?}"),
            });
        }
        let trimmed = text.trim_end_matches('/');
        if trimmed.is_empty() {
            return Err(Refusal {
                outcome: Outcome::Unavailable,
                reason: "pnpm store path is the filesystem root".to_owned(),
            });
        }
        Ok(PathBuf::from(trimmed))
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["store", "prune"]
    }
}

/// Absolute store path without ".." components or control characters.
fn is_safe_pnpm_store_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains("/../")
        && !path.ends_with("/..")
        && !path.chars().any(char::is_control)
}
