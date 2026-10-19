// Ported from tw93/Mole lib/clean/dev.sh:clean_conda_metadata_caches (GPL-3.0). Upstream reads
// `pkgs_dirs` with macOS plutil; this port reads the same key from `conda info --json`.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::{AdapterEnv, OwnerAdapter, Refusal, query_text, validated_absolute};
use crate::core::outcome::Outcome;

pub struct Conda;

impl OwnerAdapter for Conda {
    fn rule_id(&self) -> &'static str {
        "conda"
    }

    fn label(&self) -> &'static str {
        "conda index/tarball/log caches"
    }

    fn caveat(&self) -> Option<&'static str> {
        Some(
            "removes only index cache, tarballs and logs; size is the first package root, the owner cleans every root",
        )
    }

    fn program(&self) -> &'static str {
        "conda"
    }

    fn busy_names(&self) -> &'static [&'static str] {
        &["conda"]
    }

    fn target(&self, binary: &Path, env: &AdapterEnv) -> Result<PathBuf, Refusal> {
        self.targets(binary, env)?
            .into_iter()
            .next()
            .ok_or_else(|| Refusal {
                outcome: Outcome::Unavailable,
                reason: "conda reports no package roots".to_owned(),
            })
    }

    /// Every `pkgs_dirs` root: conda's clean command acts on all of them.
    fn targets(&self, binary: &Path, env: &AdapterEnv) -> Result<Vec<PathBuf>, Refusal> {
        let text = query_text(binary, &["info", "--json"], env)?;
        let info: Value = serde_json::from_str(&text).map_err(|error| Refusal {
            outcome: Outcome::Unavailable,
            reason: format!("conda info --json is not JSON: {error}"),
        })?;
        let roots = info
            .get("pkgs_dirs")
            .and_then(Value::as_array)
            .ok_or_else(|| Refusal {
                outcome: Outcome::Unavailable,
                reason: "conda info --json has no pkgs_dirs array".to_owned(),
            })?;
        if roots.is_empty() {
            return Err(Refusal {
                outcome: Outcome::Unavailable,
                reason: "conda reports no package roots".to_owned(),
            });
        }
        roots
            .iter()
            .map(|root| {
                root.as_str()
                    .and_then(validated_absolute)
                    .filter(|path| !path.components().any(|part| part == Component::ParentDir))
                    .ok_or_else(|| Refusal {
                        outcome: Outcome::Unavailable,
                        reason: format!(
                            "conda pkgs_dirs entry is not a clean absolute path: {root}"
                        ),
                    })
            })
            .collect()
    }

    // ponytail: upstream allows conda clean 30s (MOLE_TIMEOUT_DISK_VERIFY_SEC); this port uses the shared 20s CLEAN_TIMEOUT. Ceiling: a slower clean is killed and reported Unavailable. Upgrade: per-adapter clean timeout. Logged in KNOWN-GAPS.md.
    fn clean_args(&self) -> &'static [&'static str] {
        &[
            "clean",
            "--yes",
            "--index-cache",
            "--tarballs",
            "--logfiles",
        ]
    }
}
