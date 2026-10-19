//! Development caches with a Linux path (M5.5). Each rule offers the children of named cache
//! roots under HOME, the way upstream's `safe_clean ~/x/*` globs do: hidden entries are skipped,
//! and removal goes through the sink. Nothing here touches config, installs, or credentials.
// Ported from tw93/Mole lib/clean/dev.sh: clean_dev_ruby, clean_dev_perl, clean_dev_elixir,
// clean_dev_ocaml, clean_dev_shell, clean_dev_network, clean_dev_cicd (GPL-3.0).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::proc::ProcessState;
use crate::rules::clean::{
    Category, CleanRule, Discovery, DiscoveryStatus, RecoveryMode, RuleContext, candidate,
};

/// One upstream glob: the children of `dir` (relative to HOME) are the candidates.
pub struct Root {
    /// One `*` segment (as in `.gem/ruby/*/cache`) expands to every directory at that level.
    pub dir: &'static str,
    /// When set, only children whose names end with this suffix are offered.
    pub suffix: Option<&'static str>,
    pub label: &'static str,
}

pub struct ChildCacheRule {
    id: &'static str,
    roots: &'static [Root],
}

const RUBY: &[Root] = &[
    Root {
        dir: ".rbenv/cache",
        suffix: None,
        label: "rbenv download cache",
    },
    Root {
        dir: ".gem/specs",
        suffix: None,
        label: "gem spec cache",
    },
    Root {
        dir: ".gem/ruby/*/cache",
        suffix: Some(".gem"),
        label: "gem package cache",
    },
    Root {
        dir: ".bundle/cache",
        suffix: None,
        label: "Ruby Bundler cache",
    },
];

// ponytail: upstream also removes ~/.cpan/build/* only; the distribution store ~/.cpan/sources is kept because CPAN reuses it. Ceiling: nothing else under ~/.cpan is offered. Upgrade: none needed unless a build-tree guard is required. Logged in KNOWN-GAPS.md.
const PERL: &[Root] = &[Root {
    dir: ".cpan/build",
    suffix: None,
    label: "CPAN build artifacts",
}];

const ELIXIR: &[Root] = &[Root {
    dir: ".hex/cache",
    suffix: None,
    label: "Hex cache",
}];

const OCAML: &[Root] = &[Root {
    dir: ".opam/download-cache",
    suffix: None,
    label: "Opam cache",
}];

const SHELL: &[Root] = &[
    Root {
        dir: ".oh-my-zsh/cache",
        suffix: None,
        label: "Oh My Zsh cache",
    },
    Root {
        dir: ".cache/pre-commit",
        suffix: None,
        label: "pre-commit cache",
    },
];

// The macOS ~/Library/Caches lines of upstream clean_dev_network have no Linux equivalent here; the XDG paths below are the Linux cache locations.
const NETWORK: &[Root] = &[
    Root {
        dir: ".cache/curl",
        suffix: None,
        label: "curl cache",
    },
    Root {
        dir: ".cache/wget",
        suffix: None,
        label: "wget cache",
    },
];

// ponytail: upstream cicd paths are used as written. Ceiling: they are not verified against each tool's own docs on this host. Upgrade: confirm each path against the tool's documentation before relying on it. Logged in KNOWN-GAPS.md.
// Dropped from upstream clean_dev_cicd: ~/.jenkins/workspace/*/target/* (per-job build output, not a cache), ~/.github/cache (no Linux cache location for GitHub Actions), ~/.sonar/* (the scanner install sits beside the cache, so only ~/.sonar/cache is offered).
const CICD: &[Root] = &[
    Root {
        dir: ".cache/terraform",
        suffix: None,
        label: "Terraform cache",
    },
    Root {
        dir: ".grafana/cache",
        suffix: None,
        label: "Grafana cache",
    },
    Root {
        dir: ".cache/gitlab-runner",
        suffix: None,
        label: "GitLab Runner cache",
    },
    Root {
        dir: ".circleci/cache",
        suffix: None,
        label: "CircleCI cache",
    },
    Root {
        dir: ".sonar/cache",
        suffix: None,
        label: "SonarQube cache",
    },
];

/// One rule per upstream function that has a Linux cache path.
pub fn registered() -> Vec<Box<dyn CleanRule>> {
    [
        ("ruby", RUBY),
        ("perl", PERL),
        ("elixir", ELIXIR),
        ("ocaml", OCAML),
        ("shell-caches", SHELL),
        ("network-caches", NETWORK),
        ("cicd-caches", CICD),
    ]
    .into_iter()
    .map(|(id, roots)| Box::new(ChildCacheRule { id, roots }) as Box<dyn CleanRule>)
    .collect()
}

impl CleanRule for ChildCacheRule {
    fn id(&self) -> &str {
        self.id
    }

    fn category(&self) -> Category {
        Category::Development
    }

    fn discover(&self, ctx: &RuleContext<'_>) -> Discovery {
        let mut candidates = Vec::new();
        let mut issues = Vec::new();
        for root in self.roots {
            let dirs = match expand(ctx.home, root.dir) {
                Ok(dirs) => dirs,
                Err(reason) => {
                    issues.push(reason);
                    continue;
                }
            };
            for dir in dirs {
                let children = match list_children(&dir, root.suffix) {
                    Ok(children) => children,
                    Err(error) => {
                        issues.push(format!("{}: {error}", dir.display()));
                        continue;
                    }
                };
                for child in children {
                    // Defaults to selected: these are regenerable caches, as upstream's dev section runs them.
                    match candidate(
                        &child,
                        root.label,
                        RecoveryMode::Regenerate,
                        ProcessState::NotRunning,
                    ) {
                        Ok(item) => candidates.push(item),
                        Err(error) => issues.push(format!("{}: {error}", child.display())),
                    }
                }
            }
        }
        let status = if issues.is_empty() {
            DiscoveryStatus::Complete
        } else if candidates.is_empty() {
            DiscoveryStatus::Unavailable(issues.join("; "))
        } else {
            DiscoveryStatus::Partial(issues.join("; "))
        };
        Discovery { candidates, status }
    }
}

/// Resolves a root under HOME. A `*` segment lists its parent and keeps the directories found;
/// a missing parent means nothing to clean, not an error.
fn expand(home: &Path, dir: &str) -> Result<Vec<PathBuf>, String> {
    let Some((before, after)) = dir.split_once("/*/") else {
        return Ok(vec![home.join(dir)]);
    };
    let parent = home.join(before);
    let entries = match fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", parent.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", parent.display()))?;
        let is_dir = fs::symlink_metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
        if is_dir {
            found.push(entry.path().join(after));
        }
    }
    found.sort();
    Ok(found)
}

/// Direct children of `dir` that a shell glob `dir/*` would match: no hidden names, optional suffix.
pub(crate) fn list_children(dir: &Path, suffix: Option<&str>) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let bytes = name.as_encoded_bytes();
        if bytes.starts_with(b".") {
            continue;
        }
        if suffix.is_some_and(|suffix| !bytes.ends_with(suffix.as_bytes())) {
            continue;
        }
        found.push(entry.path());
    }
    found.sort();
    Ok(found)
}
