// M8.1 inventory: read-only list of installed apps per Linux source (roadmap M8).
// Upstream inventory is macOS `.app` bundles, so no upstream logic is ported here.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::core::paths::XdgPaths;
use crate::core::privilege::{Consent, OwnerCommand, OwnerProgram, OwnerRunner};

const MAX_DIR_ENTRIES: usize = 10_000;
const MAX_DESKTOP_BYTES: u64 = 64 * 1024;
// ponytail: desktop entries larger than 64 KiB are listed by file stem without reading them.
// Upgrade path: stream the [Desktop Entry] section with a larger cap. Logged in KNOWN-GAPS.md.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub name: String,
    pub detail: String,
    /// The file behind the entry (AppImage, desktop file). `None` for package-manager sources.
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Listed(Vec<Entry>),
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub status: Status,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inventory {
    pub sections: Vec<Section>,
}

/// Prints the inventory. Read-only: nothing is removed and HOME is not written.
pub fn run_list() -> io::Result<()> {
    let paths = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    let owner = OwnerRunner::system(Consent::Unasked);
    let text = render(&inventory(&owner, &paths));
    io::stdout().lock().write_all(text.as_bytes())
}

pub fn inventory(owner: &OwnerRunner, paths: &XdgPaths) -> Inventory {
    let (explicit, aur) = pacman_sections(owner);
    let mut sections = vec![
        Section {
            title: "pacman explicit",
            status: explicit,
        },
        Section {
            title: "AUR/foreign",
            status: aur,
        },
        Section {
            title: "flatpak",
            status: owner_list(
                owner,
                OwnerCommand::new(
                    OwnerProgram::Flatpak,
                    ["list", "--app", "--columns=application,version"],
                    false,
                ),
                parse_flatpak_list,
            ),
        },
        Section {
            title: "AppImage",
            status: appimages(paths),
        },
        Section {
            title: "desktop entries",
            status: desktop_entries(paths),
        },
    ];
    for section in &mut sections {
        if let Status::Listed(entries) = &mut section.status {
            entries.sort_by(|left, right| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
                    .then_with(|| left.name.cmp(&right.name))
            });
        }
    }
    Inventory { sections }
}

/// Explicit pacman packages, with foreign (`pacman -Qm`, mostly AUR) ones split out.
/// If the foreign query fails, the explicit list stays whole and the AUR section says why.
fn pacman_sections(owner: &OwnerRunner) -> (Status, Status) {
    let explicit = owner_list(owner, pacman_query(&["-Qe"]), parse_pacman_list);
    let entries = match explicit {
        Status::Listed(entries) => entries,
        Status::Unavailable(reason) => {
            return (
                Status::Unavailable(reason.clone()),
                Status::Unavailable(format!("not checked: {reason}")),
            );
        }
    };
    match owner_list(owner, pacman_query(&["-Qm"]), parse_pacman_list) {
        Status::Listed(foreign) => {
            let foreign_names: HashSet<&str> =
                foreign.iter().map(|entry| entry.name.as_str()).collect();
            let (aur, repo): (Vec<Entry>, Vec<Entry>) = entries
                .into_iter()
                .partition(|entry| foreign_names.contains(entry.name.as_str()));
            (Status::Listed(repo), Status::Listed(aur))
        }
        Status::Unavailable(reason) => (
            Status::Listed(entries),
            Status::Unavailable(format!(
                "{reason}; foreign packages are listed under pacman explicit"
            )),
        ),
    }
}

/// pacman's query exits 1 with no output when nothing matches; that is an empty list, not a failure.
fn pacman_query(args: &[&str]) -> OwnerCommand {
    OwnerCommand::new(OwnerProgram::Pacman, args, false).no_match_on_exit(1)
}

fn owner_list(
    owner: &OwnerRunner,
    query: OwnerCommand,
    parse: fn(&str) -> Result<Vec<Entry>, String>,
) -> Status {
    match owner.query(&query) {
        Ok(stdout) => match parse(&stdout) {
            Ok(entries) => Status::Listed(entries),
            Err(reason) => Status::Unavailable(reason),
        },
        Err(run) => Status::Unavailable(
            run.reason
                .unwrap_or_else(|| "owner query failed".to_owned()),
        ),
    }
}

/// `pacman -Qe` / `-Qm` rows: `name version`. Any other shape is an error, not a partial list.
pub fn parse_pacman_list(output: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        match line.split_once(' ') {
            Some((name, version))
                if !name.is_empty() && !version.is_empty() && !version.contains(' ') =>
            {
                entries.push(Entry {
                    name: name.to_owned(),
                    detail: version.to_owned(),
                    path: None,
                });
            }
            _ => return Err(format!("unexpected pacman output line: {line:?}")),
        }
    }
    Ok(entries)
}

/// `flatpak list --app --columns=application,version`: tab-separated rows.
/// Only a first row may be a column header; any other non-app-id row is an error.
pub fn parse_flatpak_list(output: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (index, line) in output
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let mut fields = line.split('\t');
        let id = fields.next().unwrap_or_default();
        let version = fields.next().unwrap_or_default();
        if is_app_id(id) {
            entries.push(Entry {
                name: id.to_owned(),
                detail: version.to_owned(),
                path: None,
            });
        } else if index != 0 {
            return Err(format!("unexpected flatpak output line: {line:?}"));
        }
    }
    Ok(entries)
}

fn is_app_id(id: &str) -> bool {
    id.contains('.')
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn appimages(paths: &XdgPaths) -> Status {
    let mut entries = Vec::new();
    for dir in [
        paths.home.join("Applications"),
        paths.home.join("AppImages"),
    ] {
        let files = match list_dir(&dir) {
            Ok(files) => files,
            Err(reason) => return Status::Unavailable(reason),
        };
        for path in files {
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(error) => {
                    return Status::Unavailable(format!(
                        "cannot inspect {}: {error}",
                        path.display()
                    ));
                }
            };
            // Symlinks are not followed: a link may point outside the scanned directory.
            let is_appimage = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("appimage"));
            if is_appimage && meta.is_file() {
                entries.push(Entry {
                    name: stem(&path),
                    detail: path.display().to_string(),
                    path: Some(path),
                });
            }
        }
    }
    Status::Listed(entries)
}

fn desktop_entries(paths: &XdgPaths) -> Status {
    let dir = paths.data.join("applications");
    let files = match list_dir(&dir) {
        Ok(files) => files,
        Err(reason) => return Status::Unavailable(reason),
    };
    let mut entries = Vec::new();
    for path in files {
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) => {
                return Status::Unavailable(format!("cannot inspect {}: {error}", path.display()));
            }
        };
        let is_desktop = path
            .extension()
            .is_some_and(|extension| extension == "desktop");
        if !is_desktop || !meta.is_file() {
            continue;
        }
        let detail = path.display().to_string();
        if meta.len() > MAX_DESKTOP_BYTES {
            entries.push(Entry {
                name: stem(&path),
                detail: format!("{detail} (not parsed: larger than 64 KiB)"),
                path: Some(path),
            });
            continue;
        }
        let name = match read_capped(&path) {
            Ok(text) => parse_desktop_name(&text).unwrap_or_else(|| stem(&path)),
            Err(error) => {
                entries.push(Entry {
                    name: stem(&path),
                    detail: format!("{detail} (unreadable: {error})"),
                    path: Some(path),
                });
                continue;
            }
        };
        entries.push(Entry {
            name,
            detail,
            path: Some(path),
        });
    }
    Status::Listed(entries)
}

/// The unlocalized `Name=` inside `[Desktop Entry]`; `Name[de]=` and other sections are ignored.
pub fn parse_desktop_name(contents: &str) -> Option<String> {
    let mut in_entry = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if let Some(value) = line.strip_prefix("Name=").map(str::trim)
            && in_entry
            && !value.is_empty()
        {
            return Some(value.to_owned());
        }
    }
    None
}

/// Lists a directory's paths. A missing directory is empty, not an error.
pub(crate) fn list_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let reader = match fs::read_dir(dir) {
        Ok(reader) => reader,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", dir.display())),
    };
    let mut paths = Vec::new();
    for (count, entry) in reader.enumerate() {
        if count >= MAX_DIR_ENTRIES {
            return Err(format!(
                "{} has more than {MAX_DIR_ENTRIES} entries; listing stopped",
                dir.display()
            ));
        }
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        paths.push(entry.path());
    }
    Ok(paths)
}

pub(crate) fn read_capped(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_DESKTOP_BYTES + 1)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Human-readable inventory. Names, versions and reasons come from other tools and files,
/// so control characters are escaped before they reach the terminal.
pub fn render(inventory: &Inventory) -> String {
    let mut out = String::from("Installed apps: inventory only, nothing is removed.\n");
    for section in &inventory.sections {
        out.push('\n');
        match &section.status {
            Status::Listed(entries) => {
                let noun = if entries.len() == 1 { "item" } else { "items" };
                out.push_str(&format!("{}: {} {noun}\n", section.title, entries.len()));
                for entry in entries {
                    let name = escape_controls(&entry.name);
                    if entry.detail.is_empty() {
                        out.push_str(&format!("  {name}\n"));
                    } else {
                        out.push_str(&format!("  {name}  {}\n", escape_controls(&entry.detail)));
                    }
                }
            }
            Status::Unavailable(reason) => out.push_str(&format!(
                "{}: unavailable ({})\n",
                section.title,
                escape_controls(reason)
            )),
        }
    }
    out
}

pub(crate) fn escape_controls(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}
