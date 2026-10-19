// Ported from tw93/Mole lib/clean/dev.sh:get_mise_cache_path and clean_dev_mise (GPL-3.0).

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, absolute_env, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Mise;

impl OwnerAdapter for Mise {
    fn rule_id(&self) -> &'static str {
        "mise"
    }

    fn label(&self) -> &'static str {
        "mise cache"
    }

    fn program(&self) -> &'static str {
        "mise"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["mise"]
    }

    /// MISE_CACHE_DIR when absolute (no query); otherwise the path mise reports.
    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        if let Some(path) = absolute_env(env.mise_cache_dir.as_ref()) {
            return Ok(path);
        }
        let text = query_text(binary, &["cache", "path"], env)?;
        validated_absolute(&text).ok_or_else(|| Refusal {
            outcome: Outcome::Unavailable,
            reason: "mise cache path did not print an absolute path".to_owned(),
        })
    }

    // ponytail: upstream falls back to a direct sweep of the cache when mise is absent, and mise's own XDG cache
    // location is ~/.cache/mise on Linux. Ceiling: fallback only when mise is installed but cannot run, using ~/.cache/mise
    // (XDG_CACHE_HOME not read). Upgrade: honor XDG_CACHE_HOME. Logged in KNOWN-GAPS.md.
    fn fallback_dir(&self, env: &AdapterEnv) -> Option<PathBuf> {
        absolute_env(env.mise_cache_dir.as_ref()).or_else(|| Some(env.home.join(".cache/mise")))
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["cache", "clear"]
    }
}
