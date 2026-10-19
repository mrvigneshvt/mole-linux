// Ported from tw93/Mole lib/clean/dev.sh:clean_dev_go, clean_go_cache_root and
// go_cache_process_state for GOCACHE (GPL-3.0). GOMODCACHE is never cleaned here.

use std::path::{Path, PathBuf};

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Go;

impl OwnerAdapter for Go {
    fn rule_id(&self) -> &'static str {
        "go"
    }

    fn label(&self) -> &'static str {
        "Go build cache"
    }

    fn program(&self) -> &'static str {
        "go"
    }

    /// Go documents GOCACHE as safe for concurrent local processes, so no owner process blocks it.
    fn busy_names(&self) -> &'static [&'static str] {
        &[]
    }

    // ponytail: upstream checks the physical root's ownership and parent identity before `go clean`. Ceiling: the reported GOCACHE path is trusted after an absolute-path check. Upgrade: port go_cache_root_physical_path. Logged in KNOWN-GAPS.md.
    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        let text = query_text(binary, &["env", "GOCACHE"], env)?;
        validated_absolute(&text).ok_or_else(|| Refusal {
            outcome: Outcome::Unavailable,
            reason: "go env GOCACHE did not print an absolute path".to_owned(),
        })
    }

    fn clean_args(&self) -> &'static [&'static str] {
        &["clean", "-cache"]
    }
}
