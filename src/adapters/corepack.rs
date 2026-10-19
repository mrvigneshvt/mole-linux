// Ported from tw93/Mole lib/clean/dev.sh:clean_corepack_cache (GPL-3.0).

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal};
use crate::core::outcome::Outcome;

pub struct Corepack;

impl OwnerAdapter for Corepack {
    fn label(&self) -> &'static str {
        "Corepack cache"
    }

    fn program(&self) -> &'static str {
        "corepack"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["corepack", "corepack.cjs"]
    }

    // ponytail: upstream falls back to deleting ~/.cache/node/corepack when corepack is absent (safe_clean).
    // Ceiling: a missing owner reports Unavailable and deletes nothing. Upgrade: route the fallback through core::sink once the M5 CLI wiring exists. Logged in KNOWN-GAPS.md.
    fn target(&self, _binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let target = match env
            .corepack_home
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            Some(value) => PathBuf::from(value),
            None => env.home.join(".cache/node/corepack"),
        };
        let library = env.home.join("Library");
        let unsafe_path = !target.is_absolute()
            || target.parent().is_none()
            || target == env.home
            || target == library;
        if unsafe_path {
            return Err(Refusal {
                outcome: Outcome::Skipped,
                reason: format!("unsafe Corepack cache path: {}", target.display()),
            });
        }
        Ok(target)
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["cache", "clean"]
    }
}
