// Ported from tw93/Mole lib/clean/dev.sh:clean_dev_python (pip3 block) (GPL-3.0).

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Pip;

impl OwnerAdapter for Pip {
    fn rule_id(&self) -> &'static str {
        "pip"
    }

    fn label(&self) -> &'static str {
        "pip cache"
    }

    fn program(&self) -> &'static str {
        "pip3"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["pip", "pip3"]
    }

    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let text = query_text(binary, &["cache", "dir"], env)?;
        validated_absolute(&text).ok_or_else(|| Refusal {
            outcome: Outcome::Unavailable,
            reason: "pip3 cache dir did not print an absolute path".to_owned(),
        })
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["cache", "purge"]
    }
}
