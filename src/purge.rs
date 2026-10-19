// Ported from tw93/Mole lib/clean/purge_shared.sh and lib/clean/project.sh
// (GPL-3.0).
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

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
const AGENT_WORKTREE_DIRS: &[&str] = &[
    ".codex/worktrees",
    ".claude/worktrees",
    ".superset/worktrees",
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
    pub size: SizeEstimate,
    pub cachedir_tag: bool,
    pub selected_by_default: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activity {
    Recent,
    Old,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeEstimate {
    Known(u64),
    Unknown,
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

pub fn is_safe_project_artifact(path: &Path, search_root: &Path, home: &Path) -> bool {
    if !path.is_absolute() || !search_root.is_absolute() || search_root == Path::new("/") {
        return false;
    }
    let (Ok(home), Ok(path), Ok(root)) = (
        home.canonicalize(),
        path.canonicalize(),
        search_root.canonicalize(),
    ) else {
        return false;
    };
    if !path.starts_with(&home) || path == home || !path.starts_with(&root) || path == root {
        return false;
    }
    let Ok(relative) = path.strip_prefix(&root) else {
        return false;
    };
    let components = relative.components().count();
    components >= 2 || (components == 1 && is_project_root(&root))
}

pub fn is_safe_configured_artifact(path: &Path, roots: &[PathBuf], home: &Path) -> bool {
    if !path.is_absolute() || !path.starts_with(home) || path == home {
        return false;
    }
    // Check lexical owners first, then physical aliases; this avoids resolving unrelated roots.
    for root in roots {
        if path.starts_with(root) && is_safe_project_artifact(path, root, home) {
            return true;
        }
    }
    roots
        .iter()
        .filter(|root| !path.starts_with(root))
        .any(|root| is_safe_project_artifact(path, root, home))
}

pub fn filter_nested_artifacts(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut sorted = paths.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut filtered: Vec<PathBuf> = Vec::new();
    for path in sorted {
        if !filtered.iter().any(|parent| path.starts_with(parent)) {
            filtered.push(path);
        }
    }
    filtered
}

pub fn is_cloud_synced_path(path: &Path, home: &Path, mountinfo: &str) -> bool {
    const CLOUD_DIRS: &[&str] = &[
        "Dropbox",
        "OneDrive",
        "Nextcloud",
        "pCloudDrive",
        "MEGA",
        "Insync",
    ];
    let homes = [Some(home.to_path_buf()), home.canonicalize().ok()];
    for resolved_home in homes.into_iter().flatten() {
        if CLOUD_DIRS
            .iter()
            .any(|name| is_at_or_below(path, &resolved_home.join(name)))
        {
            return true;
        }
    }
    mountinfo.lines().any(|line| {
        let Some((_, details)) = line.split_once(" - ") else {
            return false;
        };
        let mut fields = line.split_whitespace();
        let mount_point = fields.nth(4);
        let filesystem = details.split_whitespace().next();
        matches!((mount_point, filesystem), (Some(point), Some(fs)) if fs.starts_with("fuse.") && is_at_or_below(path, &PathBuf::from(decode_mount_field(point))))
    })
}

fn decode_mount_field(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\134", "\\")
}

fn is_at_or_below(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

pub fn find_project_root_for_artifact(path: &Path, home: &Path) -> Option<PathBuf> {
    const MONOREPO_MARKERS: &[&str] = &[
        "lerna.json",
        "pnpm-workspace.yaml",
        "nx.json",
        "rush.json",
        ".git",
    ];
    let mut current = path.parent()?;
    let mut nearest_project = None;
    while current.starts_with(home) && current != home {
        if MONOREPO_MARKERS
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            return Some(current.to_path_buf());
        }
        if nearest_project.is_none() && is_project_root(current) {
            nearest_project = Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    nearest_project
}

pub fn discover_project_dirs(home: &Path) -> io::Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    for relative in DEFAULT_SEARCH_DIRS.iter().chain(AGENT_WORKTREE_DIRS) {
        let path = home.join(relative);
        if path.is_dir() {
            roots.push(path);
        }
    }
    for entry in fs::read_dir(home)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if !entry.file_type()?.is_dir()
            || name.to_string_lossy().starts_with('.')
            || is_purge_target(&path)
            || roots.contains(&path)
        {
            continue;
        }
        if contains_project_marker(&path, 2)? {
            roots.push(path);
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn contains_project_marker(root: &Path, depth: usize) -> io::Result<bool> {
    if is_project_root(root) {
        return Ok(true);
    }
    if depth == 0 {
        return Ok(false);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && !entry.file_name().to_string_lossy().starts_with('.')
            && contains_project_marker(&entry.path(), depth - 1)?
        {
            return Ok(true);
        }
    }
    Ok(false)
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
    has_authored_content_with_budget(path, Duration::from_secs(30))
}

fn has_authored_content_with_budget(path: &Path, budget: Duration) -> io::Result<bool> {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        check_deadline(deadline)?;
        for entry in fs::read_dir(&dir)? {
            check_deadline(deadline)?;
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
    has_tracked_files(path, deadline)
}

fn has_tracked_files(path: &Path, deadline: Instant) -> io::Result<bool> {
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
                let mut command = Command::new("git");
                command
                    .args(["-c", "core.fsmonitor=false", "ls-files", "--"])
                    .arg(std::ffi::OsString::from_vec(pathspec))
                    .current_dir(parent)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE")
                    .env_remove("GIT_INDEX_FILE")
                    .env_remove("GIT_COMMON_DIR")
                    .env_remove("GIT_LITERAL_PATHSPECS")
                    .env_remove("GIT_GLOB_PATHSPECS")
                    .env_remove("GIT_NOGLOB_PATHSPECS")
                    .env_remove("GIT_ICASE_PATHSPECS")
                    .env("GIT_OPTIONAL_LOCKS", "0");
                return run_bounded_git_probe(command, deadline);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        repository = parent.parent();
    }
    Ok(false)
}

fn run_bounded_git_probe(mut command: Command, deadline: Instant) -> io::Result<bool> {
    check_deadline(deadline)?;
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("git probe stdout was not captured"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        use std::io::Read;
        let mut byte = [0_u8; 1];
        let result = stdout.read(&mut byte).map(|count| count > 0);
        let _ = sender.send(result);
    });
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            let _ = child.wait()?;
            reader
                .join()
                .map_err(|_| io::Error::other("git probe reader panicked"))?;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "git content probe timed out",
            ));
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(10))) {
            Ok(Ok(true)) => {
                if child.try_wait()?.is_none() {
                    child.kill()?;
                }
                let _ = child.wait()?;
                reader
                    .join()
                    .map_err(|_| io::Error::other("git probe reader panicked"))?;
                return Ok(true);
            }
            Ok(Ok(false)) => {
                if let Some(status) = child.try_wait()? {
                    reader
                        .join()
                        .map_err(|_| io::Error::other("git probe reader panicked"))?;
                    return if status.success() {
                        Ok(false)
                    } else {
                        Err(io::Error::other(
                            "git could not verify tracked artifact content",
                        ))
                    };
                }
            }
            Ok(Err(error)) => {
                if child.try_wait()?.is_none() {
                    child.kill()?;
                }
                let _ = child.wait()?;
                reader
                    .join()
                    .map_err(|_| io::Error::other("git probe reader panicked"))?;
                return Err(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("git probe reader disconnected"));
            }
        }
    }
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
    scan_artifacts_with_budget(root, max_depth, Duration::from_secs(30))
}

pub fn scan_artifacts_with_budget(
    root: &Path,
    max_depth: usize,
    budget: Duration,
) -> io::Result<Vec<Candidate>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")?;
    scan_artifacts_with_context(root, max_depth, budget, &home, &mountinfo)
}

pub fn scan_artifacts_with_context(
    root: &Path,
    max_depth: usize,
    budget: Duration,
    home: &Path,
    mountinfo: &str,
) -> io::Result<Vec<Candidate>> {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    check_deadline(deadline)?;
    let root_metadata = fs::metadata(root)?;
    if !root_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "purge scan root is not a directory",
        ));
    }
    if is_cloud_synced_path(root, home, mountinfo) {
        return Ok(Vec::new());
    }
    let mut projects = Vec::new();
    if !is_purge_target(root) && is_project_root(root) {
        projects.push(root.to_path_buf());
    } else {
        collect_projects(root, 0, max_depth, deadline, home, mountinfo, &mut projects)?;
    }
    let mut candidates = Vec::new();
    for project in projects {
        collect_artifacts(
            &project,
            0,
            max_depth,
            deadline,
            home,
            mountinfo,
            &mut candidates,
        )?;
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates.dedup_by(|left, right| left.path == right.path);
    Ok(candidates)
}

fn collect_projects(
    root: &Path,
    depth: usize,
    max_depth: usize,
    deadline: Instant,
    home: &Path,
    mountinfo: &str,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    check_deadline(deadline)?;
    if depth > max_depth
        || !root.is_dir()
        || is_purge_target(root)
        || is_cloud_synced_path(root, home, mountinfo)
    {
        return Ok(());
    }
    if is_project_root(root) {
        found.push(root.to_path_buf());
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        check_deadline(deadline)?;
        let entry = entry?;
        if entry.file_type()?.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
            collect_projects(
                &entry.path(),
                depth + 1,
                max_depth,
                deadline,
                home,
                mountinfo,
                found,
            )?;
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
    deadline: Instant,
    home: &Path,
    mountinfo: &str,
    found: &mut Vec<Candidate>,
) -> io::Result<()> {
    check_deadline(deadline)?;
    if depth > max_depth {
        return Ok(());
    }
    for entry in fs::read_dir(project)? {
        check_deadline(deadline)?;
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        if is_cloud_synced_path(&path, home, mountinfo) {
            continue;
        }
        let cachedir_tag = has_valid_cachedir_tag(&path)?;
        if TARGETS.contains(&entry.file_name().to_string_lossy().as_ref()) || cachedir_tag {
            let protected_content = has_authored_content_with_budget(&path, remaining(deadline))?;
            let activity =
                classify_activity_with_budget(&path, SystemTime::now(), remaining(deadline));
            let protected = protected_content
                || is_protected_artifact(&path)
                || activity == Activity::Uncertain;
            found.push(Candidate {
                activity,
                protected,
                size: measure_size_with_budget(&path, remaining(deadline)),
                cachedir_tag,
                selected_by_default: activity == Activity::Old && !protected,
                path,
            });
        } else {
            collect_artifacts(
                &path,
                depth + 1,
                max_depth,
                deadline,
                home,
                mountinfo,
                found,
            )?;
        }
    }
    Ok(())
}

fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "purge scan budget exhausted",
        ))
    } else {
        Ok(())
    }
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

pub fn classify_activity_with_budget(path: &Path, now: SystemTime, budget: Duration) -> Activity {
    let start = Instant::now();
    let deadline = start.checked_add(budget).unwrap_or(start);
    let cutoff = now
        .checked_sub(Duration::from_secs(7 * 24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if Instant::now() >= deadline {
            return Activity::Uncertain;
        }
        match fs::symlink_metadata(&dir).and_then(|metadata| metadata.modified()) {
            Ok(modified) if modified >= cutoff => return Activity::Recent,
            Ok(_) => {}
            Err(_) => return Activity::Uncertain,
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return Activity::Uncertain,
        };
        for entry in entries {
            if Instant::now() >= deadline {
                return Activity::Uncertain;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return Activity::Uncertain,
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => return Activity::Uncertain,
            };
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                match entry.metadata().and_then(|metadata| metadata.modified()) {
                    Ok(modified) if modified >= cutoff => return Activity::Recent,
                    Ok(_) => {}
                    Err(_) => return Activity::Uncertain,
                }
            }
        }
    }
    Activity::Old
}

pub fn measure_size_with_budget(path: &Path, budget: Duration) -> SizeEstimate {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    let mut pending = vec![path.to_path_buf()];
    let mut total = 0_u64;
    while let Some(current) = pending.pop() {
        if check_deadline(deadline).is_err() {
            return SizeEstimate::Unknown;
        }
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            return SizeEstimate::Unknown;
        };
        if metadata.is_file() {
            let Some(size) = total.checked_add(metadata.len()) else {
                return SizeEstimate::Unknown;
            };
            total = size;
        } else if metadata.is_dir() {
            let Ok(entries) = fs::read_dir(current) else {
                return SizeEstimate::Unknown;
            };
            for entry in entries {
                if check_deadline(deadline).is_err() {
                    return SizeEstimate::Unknown;
                }
                let Ok(entry) = entry else {
                    return SizeEstimate::Unknown;
                };
                let Ok(file_type) = entry.file_type() else {
                    return SizeEstimate::Unknown;
                };
                if file_type.is_dir() || file_type.is_file() {
                    pending.push(entry.path());
                }
            }
        }
    }
    SizeEstimate::Known(total)
}

fn has_valid_cachedir_tag(directory: &Path) -> io::Result<bool> {
    const SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";
    let tag = directory.join("CACHEDIR.TAG");
    let metadata = match fs::symlink_metadata(&tag) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Ok(false);
    }
    let mut file = fs::File::open(tag)?;
    let mut signature = vec![0; SIGNATURE.len()];
    match std::io::Read::read_exact(&mut file, &mut signature) {
        Ok(()) => Ok(signature == SIGNATURE),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}
