// Ported from tw93/Mole lib/clean/dev.sh:clean_dev_npm (bun block) (GPL-3.0).

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Bun;

impl OwnerAdapter for Bun {
    fn label(&self) -> &'static str {
        "bun cache"
    }

    fn program(&self) -> &'static str {
        "bun"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["bun"]
    }

    // ponytail: upstream retries with a filesystem delete when `bun pm cache rm` fails. Ceiling: a failed owner command reports Failed and deletes nothing. Upgrade: sink fallback once the M5 CLI wiring exists. Logged in KNOWN-GAPS.md.
    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let text = query_text(binary, &["pm", "cache"], env)?;
        validated_absolute(&text).ok_or_else(|| Refusal {
            outcome: Outcome::Unavailable,
            reason: "bun pm cache did not print an absolute path".to_owned(),
        })
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["pm", "cache", "rm"]
    }
}
