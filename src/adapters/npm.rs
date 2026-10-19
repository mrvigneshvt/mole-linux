// Ported from tw93/Mole lib/clean/dev.sh:clean_dev_npm (GPL-3.0).

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Npm;

impl OwnerAdapter for Npm {
    fn label(&self) -> &'static str {
        "npm cache"
    }

    fn program(&self) -> &'static str {
        "npm"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["npm", "npm-cli.js"]
    }

    // ponytail: upstream also sweeps npm residual dirs (_cacache, _npx, _logs, _prebuilds) via safe_clean. Ceiling: only `npm cache clean --force` runs. Upgrade: sink residual sweep. Logged in KNOWN-GAPS.md.
    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let text = query_text(binary, &["config", "get", "cache"], env)?;
        validated_absolute(&text).ok_or_else(|| Refusal {
            outcome: Outcome::Unavailable,
            reason: "npm config get cache did not print an absolute path".to_owned(),
        })
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["cache", "clean", "--force"]
    }
}
