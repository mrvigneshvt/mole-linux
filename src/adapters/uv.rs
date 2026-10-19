// Ported from tw93/Mole lib/clean/dev.sh:clean_uv_cache (GPL-3.0).

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Uv;

impl OwnerAdapter for Uv {
    fn rule_id(&self) -> &'static str {
        "uv"
    }

    fn label(&self) -> &'static str {
        "uv cache"
    }

    fn caveat(&self) -> Option<&'static str> {
        Some("prune removes only unused entries; size is the whole cache")
    }

    fn program(&self) -> &'static str {
        "uv"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["uv"]
    }

    // ponytail: upstream falls back to a filesystem sweep of ~/.cache/uv when uv is absent. Ceiling: a missing or
    // unusable uv reports Unavailable and deletes nothing. Upgrade: sink fallback for the regenerable default root. Logged in KNOWN-GAPS.md.
    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let text = query_text(binary, &["cache", "dir"], env)?;
        validated_absolute(&text).ok_or_else(|| Refusal {
            outcome: Outcome::Unavailable,
            reason: "uv cache dir did not print an absolute path".to_owned(),
        })
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["cache", "prune"]
    }
}
