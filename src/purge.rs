// Ported from tw93/Mole lib/clean/purge_shared.sh and lib/clean/project.sh
// (GPL-3.0).
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

pub const TARGETS: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "venv",
    ".venv",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".nox",
    ".ruff_cache",
    ".gradle",
    ".terragrunt-cache",
    "__pycache__",
    ".next",
    ".nuxt",
    ".output",
    "vendor",
    "bin",
    "obj",
    ".turbo",
    ".parcel-cache",
    ".dart_tool",
    ".zig-cache",
    "zig-out",
    ".angular",
    ".svelte-kit",
    ".astro",
    "coverage",
    "DerivedData",
    "Pods",
    ".cxx",
    ".expo",
    ".build",
];

pub const DEFAULT_SEARCH_DIRS: &[&str] = &[
    "www",
    "dev",
    "Projects",
    "GitHub",
    "Code",
    "Workspace",
    "Repos",
    "Development",
];

const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "pom.xml",
    "build.gradle",
    "terragrunt.hcl",
    "Gemfile",
    "composer.json",
    "pubspec.yaml",
    "Package.swift",
    "Makefile",
    "build.zig",
    "build.zig.zon",
    ".git",
    "pnpm-workspace.yaml",
    "nx.json",
    "lerna.json",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub activity: Activity,
    pub protected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activity {
    Recent,
    Old,
}

/// Reads explicit roots; missing, empty, or comment-only files use the
/// upstream default roots. Config entries are data paths, never shell input.
pub fn load_search_paths(home: &Path, config: &Path) -> io::Result<Vec<PathBuf>> {
    let contents = match fs::read_to_string(config) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut paths = Vec::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let expanded = if line == "~" {
            home.to_path_buf()
        } else if let Some(suffix) = line.strip_prefix("~/") {
            home.join(suffix)
        } else {
            PathBuf::from(line)
        };
        if !paths.contains(&expanded) {
            paths.push(expanded);
        }
    }
    if paths.is_empty() {
        paths.extend(DEFAULT_SEARCH_DIRS.iter().map(|name| home.join(name)));
    }
    Ok(paths)
}

pub fn is_project_root(path: &Path) -> bool {
    PROJECT_MARKERS
        .iter()
        .any(|marker| path.join(marker).exists())
}

pub fn is_rails_project_root(path: &Path) -> bool {
    path.join("config/application.rb").is_file()
        && path.join("Gemfile").is_file()
        && (path.join("bin/rails").is_file() || path.join("config/environment.rb").is_file())
}

pub fn is_go_project_root(path: &Path) -> bool {
    path.join("go.mod").is_file()
}

pub fn is_php_project_root(path: &Path) -> bool {
    path.join("composer.json").is_file()
}

pub fn is_dotnet_bin_dir(path: &Path) -> io::Result<bool> {
    if path.file_name() != Some(OsStr::new("bin"))
        || !(path.join("Debug").is_dir() || path.join("Release").is_dir())
    {
        return Ok(false);
    }
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    for entry in fs::read_dir(parent)? {
        if matches!(
            entry?.path().extension().and_then(OsStr::to_str),
            Some("csproj" | "fsproj" | "vbproj")
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Conservative protection for authored or ambiguous data in artifact trees.
pub fn has_authored_content(path: &Path) -> io::Result<bool> {
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name();
            if name == OsStr::new(".git") || name.to_string_lossy().ends_with("-keypair.json") {
                return Ok(true);
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    has_tracked_files(path)
}

fn has_tracked_files(path: &Path) -> io::Result<bool> {
    let mut repository = path.parent();
    while let Some(parent) = repository {
        let marker = parent.join(".git");
        match fs::symlink_metadata(&marker) {
            Ok(_) => {
                let relative = path
                    .strip_prefix(parent)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                let mut pathspec = b":(top,literal)".to_vec();
                pathspec.extend_from_slice(relative.as_os_str().as_bytes());
                let output = Command::new("git")
                    .args(["-c", "core.fsmonitor=false", "ls-files", "--"])
                    .arg(std::ffi::OsString::from_vec(pathspec))
                    .current_dir(parent)
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE")
                    .env_remove("GIT_INDEX_FILE")
                    .env_remove("GIT_COMMON_DIR")
                    .env_remove("GIT_LITERAL_PATHSPECS")
                    .env_remove("GIT_GLOB_PATHSPECS")
                    .env_remove("GIT_NOGLOB_PATHSPECS")
                    .env_remove("GIT_ICASE_PATHSPECS")
                    .env("GIT_OPTIONAL_LOCKS", "0")
                    .output()?;
                if !output.status.success() {
                    return Err(io::Error::other(
                        "git could not verify tracked artifact content",
                    ));
                }
                return Ok(!output.stdout.is_empty());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        repository = parent.parent();
    }
    Ok(false)
}

pub fn is_protected_artifact(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default();
    match name.to_str() {
        Some("vendor") if is_php_project_root(path.parent().unwrap_or(Path::new("/"))) => false,
        Some("vendor") if is_go_project_root(path.parent().unwrap_or(Path::new("/"))) => true,
        Some("vendor") if is_rails_project_root(path.parent().unwrap_or(Path::new("/"))) => true,
        Some("vendor") => true,
        Some("bin") => !matches!(is_dotnet_bin_dir(path), Ok(true)),
        _ => false,
    }
}

/// Finds project-owned artifact directories; artifact directories are pruned,
/// so nested package contents can never be mistaken for independent targets.
pub fn scan_artifacts(root: &Path, max_depth: usize) -> io::Result<Vec<Candidate>> {
    let mut projects = Vec::new();
    if !is_purge_target(root) && is_project_root(root) {
        projects.push(root.to_path_buf());
    } else {
        collect_projects(root, 0, max_depth, &mut projects)?;
    }
    let mut candidates = Vec::new();
    for project in projects {
        collect_artifacts(&project, 0, max_depth, &mut candidates)?;
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates.dedup_by(|left, right| left.path == right.path);
    Ok(candidates)
}

fn collect_projects(
    root: &Path,
    depth: usize,
    max_depth: usize,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if depth > max_depth || !root.is_dir() || is_purge_target(root) {
        return Ok(());
    }
    if is_project_root(root) {
        found.push(root.to_path_buf());
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
            collect_projects(&entry.path(), depth + 1, max_depth, found)?;
        }
    }
    Ok(())
}

fn is_purge_target(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| TARGETS.contains(&name))
}

fn collect_artifacts(
    project: &Path,
    depth: usize,
    max_depth: usize,
    found: &mut Vec<Candidate>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    for entry in fs::read_dir(project)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        if TARGETS.contains(&entry.file_name().to_string_lossy().as_ref()) {
            let protected_content = has_authored_content(&path)?;
            found.push(Candidate {
                activity: classify_activity(&path)?,
                protected: protected_content || is_protected_artifact(&path),
                path,
            });
        } else {
            collect_artifacts(&path, depth + 1, max_depth, found)?;
        }
    }
    Ok(())
}

fn classify_activity(path: &Path) -> io::Result<Activity> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(7 * 24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if fs::metadata(&dir)?.modified()? >= cutoff {
            return Ok(Activity::Recent);
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if entry.metadata()?.modified()? >= cutoff {
                return Ok(Activity::Recent);
            }
        }
    }
    Ok(Activity::Old)
}
