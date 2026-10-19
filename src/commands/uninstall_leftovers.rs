// M8.2 leftover discovery: read-only. Finds what an installed app left in the XDG roots and
// classifies each match. Nothing is removed here; removal is M8.3 (#36).
// Ported from tw93/Mole lib/uninstall/batch.sh:_uninstall_strip_version_suffix_into (GPL-3.0):
// the version-suffix list. Name matching follows the contract of lib/core/app_protection.sh:
// find_app_files (GPL-3.0), not its code: keys are normalized, paths are XDG, not macOS.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::uninstall::{
    Entry, Inventory, Status, escape_controls, inventory, list_dir, read_capped,
};
use crate::core::paths::XdgPaths;
use crate::core::privilege::{Consent, OwnerCommand, OwnerProgram, OwnerRunner};
use crate::scan::{self, Node, NodeStatus};

/// Keys shorter than this match too many unrelated names to act on.
const MIN_KEY_CHARS: usize = 3;
// ponytail: GENERIC_KEYS is a fixed list of launcher basenames. Upgrade path: derive the
// real app id from `flatpak run <id>` and `env VAR=… <app>` forms. Logged in KNOWN-GAPS.md.
const GENERIC_KEYS: &[&str] = &[
    "appimage", "bash", "env", "flatpak", "gio", "java", "node", "python", "python3", "run", "sh",
    "sudo", "wine", "xdgopen",
];
/// Version words stripped from a derived key (ported list from `_uninstall_strip_version_suffix_into`).
const VERSION_SUFFIXES: &[&str] = &[
    "alpha",
    "beta",
    "canary",
    "dev",
    "developeredition",
    "edge",
    "insider",
    "insiders",
    "lts",
    "nightly",
    "preview",
    "rc",
    "release",
    "stable",
    "technologypreview",
];
/// Shared locations: a match here is never offered, whatever app it was found for.
/// Keys are normalized the same way as `key()`.
const SHARED_NAMES: &[&str] = &[
    "applications",
    "autostart",
    "dbus1",
    "dconf",
    "environmentd",
    "flatpak",
    "fontconfig",
    "fonts",
    "gtk30",
    "gtk40",
    "gvfsmetadata",
    "icons",
    "keyrings",
    "libsecret",
    "mime",
    "pipewire",
    "pulse",
    "recentlyusedxbel",
    "seahorse",
    "ssh",
    "gnupg",
    "steam",
    "systemd",
    "themes",
    "thumbnails",
    "trash",
    "wireplumber",
];
/// Browser profile names: never offered outside the cache root (profiles hold cookies and sessions).
const BROWSER_NAMES: &[&str] = &[
    "brave",
    "bravesoftware",
    "chrome",
    "chromium",
    "firefox",
    "googlechrome",
    "librewolf",
    "microsoftedge",
    "mozilla",
    "opera",
    "operasoftware",
    "vivaldi",
    "zen",
];

/// How a leftover is classified. Only regenerable caches are selected by default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Class {
    Regenerable,
    Personal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Size {
    Measured { apparent: u64, allocated: u64 },
    Partial { apparent: u64, allocated: u64 },
    Unknown(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Leftover {
    pub path: PathBuf,
    pub class: Class,
    pub size: Size,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preview {
    pub app: String,
    pub sources: Vec<&'static str>,
    pub match_keys: Vec<String>,
    pub leftovers: Vec<Leftover>,
    pub refused: Vec<Refusal>,
    pub notes: Vec<String>,
}

/// Name for matching: lowercase alphanumerics only, so `Maestro Studio`, `maestro-studio`
/// and `MaestroStudio` are one key. Never used to build a path.
fn key(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn usable(key: &str) -> bool {
    key.chars().count() >= MIN_KEY_CHARS && !GENERIC_KEYS.contains(&key)
}

/// Keys one inventory entry answers to. `exact` joins entries of the same app; `derived`
/// (version word stripped) only matches leftovers, so `Code - Insiders` never joins `Code`.
#[derive(Clone, Debug, Default)]
struct Identity {
    exact: BTreeSet<String>,
    derived: BTreeSet<String>,
}

impl Identity {
    fn add(&mut self, raw: &str) {
        let mut names = vec![key(raw)];
        if let Some((_, last)) = raw.rsplit_once('.') {
            // Reverse-DNS app ids name the product in their last component.
            names.push(key(last));
        }
        for name in names {
            if usable(&name) {
                self.exact.insert(name.clone());
            }
            for suffix in VERSION_SUFFIXES {
                if let Some(base) = name.strip_suffix(*suffix)
                    && usable(base)
                    && !self.exact.contains(base)
                {
                    self.derived.insert(base.to_owned());
                }
            }
        }
    }

    fn all(&self) -> impl Iterator<Item = &String> {
        self.exact.iter().chain(self.derived.iter())
    }
}

struct Row {
    title: &'static str,
    name: String,
    identity: Identity,
}

/// Desktop entries: `StartupWMClass` and the `Exec` basename name the same app, under other keys.
fn desktop_identity(identity: &mut Identity, path: &Path) -> Result<(), String> {
    let text = read_capped(path).map_err(|error| error.to_string())?;
    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
        identity.add(stem);
    }
    if let Some(class) = desktop_value(&text, "StartupWMClass") {
        identity.add(&class);
    }
    if let Some(exec) = desktop_value(&text, "Exec")
        && let Some(program) = exec.split_whitespace().next()
        && let Some(base) = Path::new(program.trim_matches('"')).file_name()
        && let Some(base) = base.to_str()
    {
        identity.add(base);
    }
    Ok(())
}

/// The value of `wanted=` inside `[Desktop Entry]`; localized `Name[de]=` does not match.
fn desktop_value(contents: &str, wanted: &str) -> Option<String> {
    let mut in_entry = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && let Some(value) = line
                .strip_prefix(wanted)
                .and_then(|rest| rest.strip_prefix('='))
            && !value.trim().is_empty()
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// `Obsidian-1.5.0-x86_64` → `Obsidian`: cut at the first dash that starts a version.
fn strip_version(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        if *byte == b'-' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
            return &raw[..index];
        }
    }
    raw
}

fn row_identity(title: &str, entry: &Entry, notes: &mut Vec<String>) -> Identity {
    let mut identity = Identity::default();
    match title {
        "AppImage" => {
            identity.add(strip_version(&entry.name));
            identity.add(&entry.name);
        }
        "desktop entries" => {
            identity.add(&entry.name);
            if let Some(path) = &entry.path
                && let Err(reason) = desktop_identity(&mut identity, path)
            {
                notes.push(format!(
                    "desktop entry {} unreadable, matched by its name only: {reason}",
                    entry.name
                ));
            }
        }
        _ => identity.add(&entry.name),
    }
    identity
}

/// Where leftovers live, and what a match there means.
fn roots(paths: &XdgPaths) -> [(PathBuf, Class); 5] {
    [
        (paths.cache.clone(), Class::Regenerable),
        (paths.config.clone(), Class::Personal),
        (paths.data.clone(), Class::Personal),
        (paths.state.clone(), Class::Personal),
        // Flatpak sandbox homes: personal data under the app's id.
        (paths.home.join(".var/app"), Class::Personal),
    ]
}

/// Read-only leftover preview for the installed app that `query` names.
pub fn preview(
    owner: &OwnerRunner,
    paths: &XdgPaths,
    inventory: &Inventory,
    query: &str,
) -> Result<Preview, String> {
    let mut notes = Vec::new();
    let mut rows = Vec::new();
    for section in &inventory.sections {
        match &section.status {
            Status::Listed(entries) => {
                for entry in entries {
                    let identity = row_identity(section.title, entry, &mut notes);
                    rows.push(Row {
                        title: section.title,
                        name: entry.name.clone(),
                        identity,
                    });
                }
            }
            Status::Unavailable(reason) => {
                notes.push(format!(
                    "{} not checked ({reason}); apps only in that source cannot rule out shared leftovers",
                    section.title
                ));
            }
        }
    }

    let query_key = key(query);
    let seeds: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| !query_key.is_empty() && row.identity.exact.contains(&query_key))
        .map(|(index, _)| index)
        .collect();
    let Some(&first) = seeds.first() else {
        return Err(format!(
            "no installed app named {query:?}; see `mole-linux uninstall --list`"
        ));
    };

    // One app across sources: a desktop entry joins when its exact keys meet the seed's.
    let seed_exact: BTreeSet<String> = seeds
        .iter()
        .flat_map(|&index| rows[index].identity.exact.iter().cloned())
        .collect();
    let mut group: BTreeSet<usize> = seeds.iter().copied().collect();
    for (index, row) in rows.iter().enumerate() {
        if row.title == "desktop entries"
            && row
                .identity
                .exact
                .iter()
                .any(|name| seed_exact.contains(name))
        {
            group.insert(index);
        }
    }
    let group_keys: BTreeSet<String> = group
        .iter()
        .flat_map(|&index| rows[index].identity.all().cloned())
        .collect();
    let mut others: BTreeMap<String, String> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        if !group.contains(&index) {
            for name in row.identity.all() {
                others
                    .entry(name.clone())
                    .or_insert_with(|| row.name.clone());
            }
        }
    }
    let mut sources: Vec<&'static str> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if group.contains(&index) && !sources.contains(&row.title) {
            sources.push(row.title);
        }
    }

    let mut leftovers = Vec::new();
    let mut refused = Vec::new();
    for (root, class) in roots(paths) {
        let children = match list_dir(&root) {
            Ok(children) => children,
            Err(reason) => {
                notes.push(format!("not checked: {reason}"));
                continue;
            }
        };
        for child in children {
            let Some(name) = child.file_name().and_then(|name| name.to_str()) else {
                notes.push(format!("skipped a non-UTF-8 name in {}", root.display()));
                continue;
            };
            let name_key = key(name);
            if !group_keys.contains(&name_key) {
                continue;
            }
            match judge(owner, &child, &name_key, class, &others) {
                Verdict::Offer(size) => leftovers.push(Leftover {
                    path: child,
                    class,
                    size,
                }),
                Verdict::Refuse(reason) => refused.push(Refusal {
                    path: child,
                    reason,
                }),
            }
        }
    }
    leftovers.sort_by(|left, right| left.path.cmp(&right.path));
    refused.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(Preview {
        app: rows[first].name.clone(),
        sources,
        match_keys: group_keys.into_iter().collect(),
        leftovers,
        refused,
        notes,
    })
}

enum Verdict {
    Offer(Size),
    Refuse(String),
}

/// Safety checks in order: name-based rules, then the entry itself, then its contents,
/// then the owner query. The first refusal wins and says why.
fn judge(
    owner: &OwnerRunner,
    child: &Path,
    name_key: &str,
    class: Class,
    others: &BTreeMap<String, String>,
) -> Verdict {
    if SHARED_NAMES.contains(&name_key) {
        return Verdict::Refuse("shared location, never offered".to_owned());
    }
    if class == Class::Personal && BROWSER_NAMES.contains(&name_key) {
        return Verdict::Refuse("browser profile, never offered outside the cache".to_owned());
    }
    if let Some(other) = others.get(name_key) {
        return Verdict::Refuse(format!("also used by installed app {other}"));
    }
    let meta = match std::fs::symlink_metadata(child) {
        Ok(meta) => meta,
        Err(error) => return Verdict::Refuse(format!("cannot inspect: {error}")),
    };
    if meta.file_type().is_symlink() {
        return Verdict::Refuse("symbolic link, not followed".to_owned());
    }
    let report = scan::scan(child, &AtomicBool::new(false));
    let mut contents = Contents::default();
    contents.visit(&report.root);
    if contents.sqlite {
        return Verdict::Refuse("contains a SQLite database, never touched".to_owned());
    }
    if contents.git {
        return Verdict::Refuse("contains git metadata, never touched".to_owned());
    }
    match owner_of(owner, child) {
        Ok(Some(package)) => return Verdict::Refuse(format!("owned by package {package}")),
        Ok(None) => {}
        Err(reason) => return Verdict::Refuse(format!("owner check unavailable: {reason}")),
    }
    Verdict::Offer(size_of(&report.root, &contents))
}

#[derive(Default)]
struct Contents {
    sqlite: bool,
    git: bool,
    incomplete: Option<String>,
}

impl Contents {
    fn visit(&mut self, node: &Node) {
        let name = node.name.to_string_lossy();
        if name == ".git" {
            self.git = true;
        }
        if name.ends_with(".db") || name.ends_with("-wal") || name.ends_with("-shm") {
            self.sqlite = true;
        }
        match &node.status {
            NodeStatus::Complete => {}
            NodeStatus::Partial => {
                self.incomplete
                    .get_or_insert_with(|| "partial scan".to_owned());
            }
            NodeStatus::Unavailable(reason) | NodeStatus::Excluded(reason) => {
                self.incomplete.get_or_insert_with(|| reason.clone());
            }
            NodeStatus::Scanning => {
                self.incomplete
                    .get_or_insert_with(|| "scan did not finish".to_owned());
            }
        }
        for child in &node.children {
            self.visit(child);
        }
    }
}

fn size_of(root: &Node, contents: &Contents) -> Size {
    match &root.status {
        NodeStatus::Unavailable(reason) | NodeStatus::Excluded(reason) => {
            Size::Unknown(reason.clone())
        }
        NodeStatus::Scanning => Size::Unknown("scan did not finish".to_owned()),
        NodeStatus::Complete | NodeStatus::Partial => match &contents.incomplete {
            None => Size::Measured {
                apparent: root.apparent,
                allocated: root.allocated,
            },
            Some(_) => Size::Partial {
                apparent: root.apparent,
                allocated: root.allocated,
            },
        },
    }
}

/// `pacman -Qo <path>`: `Ok(Some(pkg))` owned, `Ok(None)` unowned, `Err` when the answer is unknown.
fn owner_of(owner: &OwnerRunner, path: &Path) -> Result<Option<String>, String> {
    let command = OwnerCommand::new(
        OwnerProgram::Pacman,
        [OsString::from("-Qo"), path.as_os_str().to_owned()],
        false,
    );
    match owner.query(&command) {
        Ok(stdout) => {
            let package = stdout
                .lines()
                .find_map(|line| line.split_once(" is owned by "))
                .and_then(|(_, rest)| rest.split_whitespace().next());
            match package {
                Some(package) => Ok(Some(package.to_owned())),
                None => Err(format!("unexpected pacman -Qo output: {stdout:?}")),
            }
        }
        Err(run) => {
            let reason = run
                .reason
                .unwrap_or_else(|| "owner query failed".to_owned());
            if reason.contains("No package owns ") {
                Ok(None)
            } else {
                Err(reason)
            }
        }
    }
}

/// Human-readable preview. Names, paths and reasons come from the filesystem and other tools,
/// so control characters are escaped first.
pub fn render(preview: &Preview) -> String {
    let mut out = format!(
        "Uninstall preview: {} (nothing is removed)\n",
        escape_controls(&preview.app)
    );
    out.push_str(&format!("Sources: {}\n", preview.sources.join(", ")));
    out.push_str(&format!(
        "Match keys: {}\n",
        escape_controls(&preview.match_keys.join(", "))
    ));

    out.push_str(&format!("\nLeftovers: {}\n", preview.leftovers.len()));
    for leftover in &preview.leftovers {
        let (selection, label) = match leftover.class {
            Class::Regenerable => ("selected", "cache"),
            Class::Personal => ("not selected", "personal data"),
        };
        out.push_str(&format!(
            "  {selection:<12} {label:<13} {}  {}\n",
            escape_controls(&leftover.path.display().to_string()),
            describe_size(&leftover.size)
        ));
    }

    out.push_str(&format!("\nNot offered: {}\n", preview.refused.len()));
    for refusal in &preview.refused {
        out.push_str(&format!(
            "  {}  {}\n",
            escape_controls(&refusal.path.display().to_string()),
            escape_controls(&refusal.reason)
        ));
    }

    if !preview.notes.is_empty() {
        out.push_str("\nNotes:\n");
        for note in &preview.notes {
            out.push_str(&format!("  {}\n", escape_controls(note)));
        }
    }
    out.push_str(
        "\nAllocated is the filesystem's block count. Btrfs snapshots and Trash can keep extents,\n\
         so removing a leftover can free less than its allocated size. Nothing is measured as freed here.\n",
    );
    out
}

fn describe_size(size: &Size) -> String {
    match size {
        Size::Measured {
            apparent,
            allocated,
        } => format!(
            "apparent {}, allocated {}",
            si_bytes(*apparent),
            si_bytes(*allocated)
        ),
        Size::Partial {
            apparent,
            allocated,
        } => format!(
            "partial, apparent {}, allocated {}",
            si_bytes(*apparent),
            si_bytes(*allocated)
        ),
        Size::Unknown(reason) => format!("unknown ({})", escape_controls(reason)),
    }
}

/// SI units (kB, MB, …) for disk sizes, per the repo's unit rule.
fn si_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut index = 0;
    let mut divisor: u64 = 1;
    while index + 1 < UNITS.len() && bytes / divisor >= 1000 {
        divisor *= 1000;
        index += 1;
    }
    if index == 0 {
        return format!("{bytes} B");
    }
    let whole = bytes / divisor;
    let tenth = (bytes % divisor) / (divisor / 10);
    format!("{whole}.{tenth} {}", UNITS[index])
}

/// The `uninstall --dry-run <name>` entry point: inventory, then leftover preview. Read-only.
pub fn run_preview(name: &str) -> io::Result<()> {
    let paths = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    let owner = OwnerRunner::system(Consent::Unasked);
    let apps = inventory(&owner, &paths);
    let report = preview(&owner, &paths, &apps, name)
        .map_err(|reason| io::Error::new(io::ErrorKind::NotFound, reason))?;
    io::stdout().lock().write_all(render(&report).as_bytes())
}
