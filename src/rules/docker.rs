//! Docker (M5.5). `docker` is an advisory row: it reads `docker system df` through the owner
//! runner and removes nothing. `docker-build-cache` removes the BuildX cache directory, and only
//! when its own id is named with `--select`; a category selection or the default run never does.
// Ported from tw93/Mole lib/clean/dev.sh:clean_dev_docker (GPL-3.0). The daemon-managed data is
// reported, never pruned; the OrbStack lines are macOS-only and dropped.

use std::io;
use std::path::PathBuf;

use serde::Deserialize;

use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRunner};
use crate::core::proc::ProcessState;
use crate::rules::clean::{
    Category, CleanRule, Discovery, DiscoveryStatus, RecoveryMode, RuleContext, candidate,
};
use crate::rules::dev_caches::list_children;

const BUILDX_CACHE: &str = ".docker/buildx/cache";
const BUILDX_LABEL: &str = "Docker BuildX cache (explicit --select docker-build-cache only)";

/// One row of `docker system df --format '{{json .}}'`. Docker prints every value as a string.
#[derive(Deserialize)]
struct DfRow {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "Size")]
    size: String,
    #[serde(rename = "Reclaimable")]
    reclaimable: String,
}

pub fn registered() -> Vec<Box<dyn CleanRule>> {
    vec![Box::new(DockerAdvisory), Box::new(BuildxCache)]
}

struct DockerAdvisory;

impl CleanRule for DockerAdvisory {
    fn id(&self) -> &str {
        "docker"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    /// Reading `docker system df` needs the owner runner, so this row cannot discover without it.
    fn discover(&self, _ctx: &RuleContext<'_>) -> Discovery {
        Discovery {
            candidates: Vec::new(),
            status: DiscoveryStatus::Unavailable("docker report needs the owner runner".to_owned()),
        }
    }

    fn discover_with_owner(&self, _ctx: &RuleContext<'_>, owner: &OwnerRunner) -> Discovery {
        // ponytail: the report is the raw per-type totals. Upstream only says "review with docker system df". Upgrade path: per-image or per-volume listings, which need more queries. Logged in KNOWN-GAPS.md.
        let command = OwnerCommand::new(
            OwnerProgram::Docker,
            ["system", "df", "--format", "{{json .}}"],
            false,
        );
        let text = match owner.query(&command) {
            Ok(text) => text,
            Err(run) => {
                let reason = run.reason.unwrap_or_else(|| "query failed".to_owned());
                return unavailable(&format!("docker system df: {reason}"));
            }
        };
        match parse_report(&text) {
            Ok(report) => Discovery {
                candidates: Vec::new(),
                status: DiscoveryStatus::Advisory(format!(
                    "docker system df (advisory, nothing removed): {report}"
                )),
            },
            Err(reason) => unavailable(&reason),
        }
    }
}

/// Every row must parse; a partly readable report is unavailable, not a shorter list.
fn parse_report(text: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let row: DfRow = serde_json::from_str(line)
            .map_err(|error| format!("unexpected docker system df output: {error}"))?;
        parts.push(format!(
            "{}: {} total, {} reclaimable",
            row.kind, row.size, row.reclaimable
        ));
    }
    if parts.is_empty() {
        return Err("unexpected docker system df output: no rows".to_owned());
    }
    Ok(parts.join("; "))
}

struct BuildxCache;

impl CleanRule for BuildxCache {
    fn id(&self) -> &str {
        "docker-build-cache"
    }

    fn category(&self) -> Category {
        Category::Development
    }

    /// Named ids only: `--select development` or a category run must not remove the build cache.
    fn selected_by_category(&self) -> bool {
        false
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let root: PathBuf = ctx.home.join(BUILDX_CACHE);
        let children = match list_children(&root, None) {
            Ok(children) => children,
            Err(error) => {
                return Discovery {
                    candidates: Vec::new(),
                    status: DiscoveryStatus::Unavailable(format!("{}: {error}", root.display())),
                };
            }
        };
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        for child in children {
            match candidate(
                &child,
                BUILDX_LABEL,
                RecoveryMode::Regenerate,
                ProcessState::NotRunning,
            ) {
                Ok(mut item) => {
                    item.selected_by_default = false;
                    candidates.push(item);
                }
                // Gone since listing: nothing left to clean, so it is not an issue.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => issues.push(format!("{}: {error}", child.display())),
            }
        }
        let status = if issues.is_empty() {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial(issues.join("; "))
        };
        Discovery { candidates, status }
    }
}

fn unavailable(reason: &str) -> Discovery {
    Discovery {
        candidates: Vec::new(),
        status: DiscoveryStatus::Unavailable(reason.to_owned()),
    }
}
