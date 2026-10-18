use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};

use super::fsutil::{atomic_replace, resolve_symlink_chain};
use super::paths::XdgPaths;

/// User configured paths that must be kept by every deletion policy check.
#[derive(Debug)]
pub struct Whitelist {
    patterns: Vec<Pattern>,
    pub warnings: Vec<WhitelistWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhitelistWarning {
    pub line: usize,
    pub reason: String,
}

#[derive(Debug)]
struct Pattern {
    source: String,
    matcher: GlobMatcher,
    has_glob: bool,
}

#[derive(Debug)]
pub enum WhitelistError {
    Io(io::Error),
}

impl Whitelist {
    /// Load `$XDG_CONFIG_HOME/mole-linux/whitelist`; a missing file is empty.
    pub fn load(xdg: &XdgPaths) -> Result<Self, WhitelistError> {
        // Ported from tw93/Mole lib/manage/whitelist.sh:load_whitelist (GPL-3.0).
        let path = xdg.config.join("mole-linux/whitelist");
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(WhitelistError::Io(error)),
        };
        let mut patterns = Vec::new();
        let mut warnings = Vec::new();
        for (index, raw) in contents.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let expanded = expand_home(line, &xdg.home);
            let invalid_reason = if !expanded.starts_with('/') {
                Some("relative path")
            } else if expanded.contains("..") {
                Some("path traversal")
            } else if expanded.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
                Some("control character")
            } else {
                None
            };
            if let Some(reason) = invalid_reason {
                warnings.push(WhitelistWarning {
                    line: index + 1,
                    reason: reason.to_owned(),
                });
                continue;
            }
            let normalized = normalize_slashes(&expanded);
            let has_glob =
                normalized.contains('*') || normalized.contains('?') || normalized.contains('[');
            let matcher = match GlobBuilder::new(&normalized)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
            {
                Ok(glob) => glob.compile_matcher(),
                Err(_) => {
                    warnings.push(WhitelistWarning {
                        line: index + 1,
                        reason: "invalid glob pattern".to_owned(),
                    });
                    continue;
                }
            };
            if !patterns
                .iter()
                .any(|pattern: &Pattern| pattern.source == normalized)
            {
                patterns.push(Pattern {
                    source: normalized,
                    matcher,
                    has_glob,
                });
            }
        }
        Ok(Self { patterns, warnings })
    }

    /// Return whether a path or a containing directory must be preserved.
    /// The deletion sink checks this policy before removing a path.
    pub fn is_whitelisted(&self, path: &Path) -> bool {
        // Ported from tw93/Mole lib/core/app_protection.sh:is_path_whitelisted (GPL-3.0).
        let Some(target) = path.to_str().map(normalize_slashes) else {
            return self
                .patterns
                .iter()
                .any(|pattern| pattern.matcher.is_match(path));
        };
        let target = target.trim_end_matches('/');
        self.patterns.iter().any(|pattern| {
            pattern.matcher.is_match(path)
                || pattern.matcher.is_match(target)
                || pattern.source.starts_with(&format!("{target}/"))
                || (!pattern.has_glob && target.starts_with(&format!("{}/", pattern.source)))
        })
    }

    pub fn matches(&self, path: &Path) -> bool {
        self.is_whitelisted(path)
    }

    pub fn path(xdg: &XdgPaths) -> PathBuf {
        xdg.config.join("mole-linux/whitelist")
    }

    /// Persist a deduplicated list of user patterns at the XDG whitelist path.
    pub fn save_patterns(xdg: &XdgPaths, patterns: &[String]) -> Result<(), WhitelistError> {
        let path = Self::path(xdg);
        let parent = path.parent().ok_or_else(|| {
            WhitelistError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "whitelist path has no parent directory",
            ))
        })?;
        fs::create_dir_all(parent).map_err(WhitelistError::Io)?;
        let mut contents = String::from(
            "# Mole Linux Whitelist - Protected paths will not be deleted\n# Add one glob pattern per line.\n",
        );
        let mut unique = Vec::<String>::new();
        for pattern in patterns {
            let normalized = normalize_slashes(pattern.trim());
            if !normalized.is_empty() && !unique.contains(&normalized) {
                unique.push(normalized);
            }
        }
        for pattern in unique {
            contents.push_str(&pattern);
            contents.push('\n');
        }
        let resolved_path = resolve_symlink_chain(&path).map_err(WhitelistError::Io)?;
        atomic_replace(&resolved_path, contents.as_bytes()).map_err(WhitelistError::Io)
    }
}

fn expand_home(pattern: &str, home: &Path) -> String {
    let Some(home) = home.to_str() else {
        return pattern.to_owned();
    };
    if pattern == "~" {
        home.to_owned()
    } else if let Some(rest) = pattern.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if let Some(rest) = pattern.strip_prefix("$HOME/") {
        format!("{home}/{rest}")
    } else if let Some(rest) = pattern.strip_prefix("${HOME}/") {
        format!("{home}/{rest}")
    } else {
        pattern.to_owned()
    }
}

fn normalize_slashes(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_slash = false;
    for character in value.chars() {
        if character == '/' {
            if !previous_slash {
                normalized.push(character);
            }
            previous_slash = true;
        } else {
            normalized.push(character);
            previous_slash = false;
        }
    }
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}
