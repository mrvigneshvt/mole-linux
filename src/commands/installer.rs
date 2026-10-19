// Ported from tw93/Mole bin/installer.sh:scan_installers_in_path,
// is_installer_zip, and delete_selected_installers (GPL-3.0).
use std::fs;
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::core::oplog::{OpLog, SessionSummary};
use crate::core::outcome::{Outcome, OutcomeLedger};
use crate::core::paths::XdgPaths;
use crate::core::sink::{DeleteMode, Identity, SinkContext, delete};
use crate::core::whitelist::Whitelist;

const DEFAULT_DEPTH: u32 = 2;
const MAX_DEPTH: u32 = 16;
const MAX_VISITED: usize = 100_000;
const MAX_ZIP_DIRECTORY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ZIP_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 50;
// ponytail: std-only file reads have no cancellable wall-clock deadline; cap traversal and ZIP bytes,
// with interruptible archive I/O as the upgrade path if that API becomes approved.

#[derive(Default)]
struct ScanBudget {
    visited: usize,
    zip_directory_bytes: u64,
}

#[derive(Clone, Debug)]
struct Candidate {
    path: PathBuf,
    identity: Identity,
    apparent_size: u64,
    allocated_size: u64,
}

struct ScanInventory {
    candidates: Vec<Candidate>,
    unavailable_paths: usize,
}

pub fn run(
    dry_run: bool,
    max_depth: Option<u32>,
    include_hidden: bool,
    kind: Option<&str>,
) -> io::Result<()> {
    if rustix::process::getuid().as_raw() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "run mole-linux without sudo",
        ));
    }
    let depth = match max_depth {
        Some(depth) => depth,
        None => match std::env::var("MOLE_INSTALLER_SCAN_MAX_DEPTH") {
            Ok(value) => value.parse::<u32>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MOLE_INSTALLER_SCAN_MAX_DEPTH must be an integer",
                )
            })?,
            Err(std::env::VarError::NotPresent) => DEFAULT_DEPTH,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MOLE_INSTALLER_SCAN_MAX_DEPTH is not valid Unicode",
                ));
            }
        },
    };
    if depth == 0 || depth > MAX_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("--max-depth must be between 1 and {MAX_DEPTH}"),
        ));
    }
    let kind = kind.map(|value| value.to_ascii_lowercase());
    if let Some(kind) = &kind
        && !known_kind(kind)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown installer type: {kind}"),
        ));
    }

    let xdg = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    let mut roots = vec![xdg.home.join("Downloads"), xdg.home.join("Desktop")];
    if let Some(configured) = std::env::var_os("MOLE_INSTALLER_PATHS") {
        roots.extend(std::env::split_paths(&configured).filter(|path| path.is_absolute()));
    }
    roots.sort();
    roots.dedup();
    let inventory = scan_roots(&roots, depth, include_hidden, kind.as_deref())?;
    if inventory.candidates.is_empty() {
        if inventory.unavailable_paths > 0 {
            println!(
                "No installers found in readable paths. Scan incomplete: unable to inspect {} path(s).",
                inventory.unavailable_paths
            );
            return Ok(());
        }
        println!("No installer files found.");
        return Ok(());
    }
    for candidate in &inventory.candidates {
        println!(
            "{}  apparent {}  allocated {}",
            display_path(&candidate.path),
            format_disk_size(candidate.apparent_size),
            format_disk_size(candidate.allocated_size)
        );
    }
    if inventory.unavailable_paths > 0 {
        println!(
            "Scan incomplete: unable to inspect {} path(s); listed candidates are from readable paths only.",
            inventory.unavailable_paths
        );
    }
    if dry_run {
        println!("Dry run: no files changed.");
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    println!("Enter exact candidate paths to Trash, one per line; submit an empty line when done.");
    let mut selected = Vec::new();
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        let path = PathBuf::from(line.trim_end_matches(['\r', '\n']));
        if path.as_os_str().is_empty() {
            break;
        }
        if !inventory
            .candidates
            .iter()
            .any(|candidate| candidate.path == path)
        {
            eprintln!("Not a discovered candidate: {}", display_path(&path));
            continue;
        }
        if !selected.contains(&path) {
            selected.push(path);
        }
    }
    if selected.is_empty() {
        println!("No paths selected; nothing changed.");
        return Ok(());
    }
    println!(
        "Type 'yes' to Trash these paths: {}",
        selected
            .iter()
            .map(|path| display_path(path))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut confirmation = String::new();
    input.read_line(&mut confirmation)?;
    if confirmation.trim() != "yes" {
        println!("Cancelled; nothing changed.");
        return Ok(());
    }

    let whitelist = Whitelist::load(&xdg).map_err(|error| match error {
        crate::core::whitelist::WhitelistError::Io(error) => error,
    })?;
    let context = SinkContext::new(xdg.clone(), whitelist);
    let oplog = OpLog::with_state_home(&xdg.state);
    let mut session = oplog.start_session("installer", false)?;
    let mut outcomes = OutcomeLedger::default();
    for (index, path) in selected.iter().enumerate() {
        let candidate = inventory
            .candidates
            .iter()
            .find(|candidate| &candidate.path == path)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "selected candidate disappeared from plan",
                )
            })?;
        let result = delete(
            &candidate.path,
            &candidate.identity,
            DeleteMode::Trash,
            &context,
        );
        let (outcome_name, outcome) = match &result {
            Ok(_) => ("applied", Outcome::Applied),
            Err(crate::core::sink::DeleteError::IdentityChanged)
            | Err(crate::core::sink::DeleteError::Whitelisted)
            | Err(crate::core::sink::DeleteError::InvalidPath(_)) => ("skipped", Outcome::Skipped),
            Err(_) => ("failed", Outcome::Failed),
        };
        session.record("trash", outcome_name, &candidate.path)?;
        outcomes.start().map_err(|error| {
            io::Error::other(format!("invalid installer outcome state: {error:?}"))
        })?;
        outcomes
            .report(outcome)
            .map_err(|error| io::Error::other(format!("invalid installer outcome: {error:?}")))?;
        outcomes
            .finish(&format!("trash_{index}"))
            .map_err(|error| io::Error::other(format!("invalid installer outcome: {error:?}")))?;
        match result {
            Ok(_) => println!("Trashed {}", display_path(&candidate.path)),
            Err(error) => eprintln!("Skipped/failed {}: {error}", display_path(&candidate.path)),
        }
    }
    let count = u64::try_from(selected.len())
        .map_err(|_| io::Error::other("selected path count exceeds operation log range"))?;
    session.end(&SessionSummary {
        items: count,
        size: format!("{count} paths"),
    })?;
    println!(
        "Installer cleanup: {} trashed, {} skipped or failed. Space is not freed until Trash is emptied.",
        outcomes.count(Outcome::Applied),
        outcomes.count(Outcome::Skipped) + outcomes.count(Outcome::Failed)
    );
    if outcomes.count(Outcome::Failed) > 0 {
        return Err(io::Error::other(
            "installer cleanup incomplete; see failed paths above",
        ));
    }
    Ok(())
}

fn scan_roots(
    roots: &[PathBuf],
    max_depth: u32,
    include_hidden: bool,
    kind: Option<&str>,
) -> io::Result<ScanInventory> {
    let mut budget = ScanBudget::default();
    let mut found = Vec::new();
    let mut unavailable_paths = 0_usize;
    for root in roots {
        match fs::metadata(root) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                mark_unavailable(&mut unavailable_paths)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut stack = vec![(root.clone(), 0_u32)];
        while let Some((directory, depth)) = stack.pop() {
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    mark_unavailable(&mut unavailable_paths)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            for entry in entries {
                budget.visited = budget
                    .visited
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("scan count overflow"))?;
                if budget.visited > MAX_VISITED {
                    return Err(io::Error::other("installer scan exceeded entry budget"));
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                        mark_unavailable(&mut unavailable_paths)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let name = entry.file_name();
                if !include_hidden && name.as_bytes().first() == Some(&b'.') {
                    continue;
                }
                let path = entry.path();
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                        mark_unavailable(&mut unavailable_paths)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    if depth
                        .checked_add(1)
                        .is_some_and(|entry_depth| entry_depth < max_depth)
                    {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                if !metadata.is_file() || !is_installer_name(&name, kind) {
                    continue;
                }
                if ends_ascii_case(name.as_bytes(), b".zip") {
                    match zip_has_installer(&path, &mut budget) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                            ) =>
                        {
                            continue;
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound
                            ) =>
                        {
                            mark_unavailable(&mut unavailable_paths)?;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
                let identity = match Identity::capture(&path) {
                    Ok(identity) => identity,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        mark_unavailable(&mut unavailable_paths)?;
                        continue;
                    }
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                        mark_unavailable(&mut unavailable_paths)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                found.push(Candidate {
                    path,
                    identity,
                    apparent_size: metadata.len(),
                    allocated_size: metadata
                        .blocks()
                        .checked_mul(512)
                        .ok_or_else(|| io::Error::other("allocated size overflow"))?,
                });
            }
        }
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    found.dedup_by(|left, right| left.path == right.path);
    Ok(ScanInventory {
        candidates: found,
        unavailable_paths,
    })
}

fn mark_unavailable(count: &mut usize) -> io::Result<()> {
    *count = count
        .checked_add(1)
        .ok_or_else(|| io::Error::other("unavailable path count overflow"))?;
    Ok(())
}

fn known_kind(kind: &str) -> bool {
    matches!(
        kind,
        "deb" | "rpm" | "appimage" | "pkg.tar.zst" | "iso" | "tar" | "zip" | "flatpakref"
    )
}

fn is_installer_name(name: &std::ffi::OsStr, kind: Option<&str>) -> bool {
    let bytes = name.as_bytes();
    match kind {
        Some("deb") => ends_ascii_case(bytes, b".deb"),
        Some("rpm") => ends_ascii_case(bytes, b".rpm"),
        Some("appimage") => ends_ascii_case(bytes, b".appimage"),
        Some("pkg.tar.zst") => ends_ascii_case(bytes, b".pkg.tar.zst"),
        Some("iso") => ends_ascii_case(bytes, b".iso"),
        Some("tar") => is_tar_name(bytes),
        Some("zip") => ends_ascii_case(bytes, b".zip"),
        Some("flatpakref") => ends_ascii_case(bytes, b".flatpakref"),
        Some(_) => false,
        None => {
            [
                b".deb".as_slice(),
                b".rpm",
                b".appimage",
                b".pkg.tar.zst",
                b".iso",
                b".flatpakref",
            ]
            .iter()
            .any(|extension| ends_ascii_case(bytes, extension))
                || is_tar_name(bytes)
                || ends_ascii_case(bytes, b".zip")
        }
    }
}

fn is_tar_name(name: &[u8]) -> bool {
    ends_ascii_case(name, b".tar")
        || ends_ascii_case(name, b".tgz")
        || name
            .windows(5)
            .rposition(|window| window.eq_ignore_ascii_case(b".tar."))
            .is_some_and(|index| index + 5 < name.len())
}

fn ends_ascii_case(value: &[u8], suffix: &[u8]) -> bool {
    value.len() >= suffix.len() && value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn format_disk_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0_usize;
    let mut scale = 1_u64;
    while value >= 1000 && unit + 1 < UNITS.len() {
        value /= 1000;
        scale *= 1000;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        let whole = value;
        let tenth = (bytes % scale) / (scale / 10);
        format!("{whole}.{tenth} {}", UNITS[unit])
    }
}

fn zip_has_installer(path: &Path, budget: &mut ScanBudget) -> io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    let tail_size = length.min(65_557);
    let tail_len = usize::try_from(tail_size)
        .map_err(|_| io::Error::other("ZIP tail exceeds address space"))?;
    let tail_offset = length - tail_size;
    file.seek(SeekFrom::Start(tail_offset))?;
    let mut tail = vec![0_u8; tail_len];
    if let Err(error) = file.read_exact(&mut tail) {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(false);
        }
        return Err(error);
    }
    let eocd = tail.windows(4).rposition(|window| window == b"PK\x05\x06");
    let Some(eocd) = eocd else { return Ok(false) };
    let record = &tail[eocd..];
    if record.len() < 22 {
        return Ok(false);
    }
    let entries = usize::from(u16::from_le_bytes([record[10], record[11]]));
    let central_size = u64::from(u32::from_le_bytes([
        record[12], record[13], record[14], record[15],
    ]));
    let central_offset = u64::from(u32::from_le_bytes([
        record[16], record[17], record[18], record[19],
    ]));
    let comment_size = usize::from(u16::from_le_bytes([record[20], record[21]]));
    if record.len() != 22_usize.saturating_add(comment_size)
        || record[4..8] != [0, 0, 0, 0]
        || record[8..10] != record[10..12]
        || entries == usize::from(u16::MAX)
        || central_size > MAX_ZIP_DIRECTORY_BYTES
        || central_offset
            .checked_add(central_size)
            .is_none_or(|end| end > length)
    {
        return Ok(false);
    }
    let archive_directory_end = central_offset + central_size;
    let eocd_offset = tail_offset
        .checked_add(
            u64::try_from(eocd)
                .map_err(|_| io::Error::other("ZIP offset exceeds address space"))?,
        )
        .ok_or_else(|| io::Error::other("ZIP offset overflow"))?;
    if archive_directory_end > eocd_offset {
        return Ok(false);
    }
    budget.zip_directory_bytes = budget
        .zip_directory_bytes
        .checked_add(central_size)
        .ok_or_else(|| io::Error::other("cumulative ZIP inspection size overflow"))?;
    if budget.zip_directory_bytes > MAX_ZIP_TOTAL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "cumulative ZIP inspection budget exceeded",
        ));
    }
    let central_len = usize::try_from(central_size)
        .map_err(|_| io::Error::other("ZIP directory exceeds address space"))?;
    let mut central = vec![0_u8; central_len];
    file.seek(SeekFrom::Start(central_offset))?;
    if let Err(error) = file.read_exact(&mut central) {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(false);
        }
        return Err(error);
    }
    let mut offset = 0_usize;
    let mut installer = false;
    for index in 0..entries {
        let fixed_end = offset
            .checked_add(46)
            .ok_or_else(|| io::Error::other("ZIP directory offset overflow"))?;
        if fixed_end > central.len() || central[offset..offset + 4] != *b"PK\x01\x02" {
            return Ok(false);
        }
        let name_len = usize::from(u16::from_le_bytes([
            central[offset + 28],
            central[offset + 29],
        ]));
        let extra_len = usize::from(u16::from_le_bytes([
            central[offset + 30],
            central[offset + 31],
        ]));
        let comment_len = usize::from(u16::from_le_bytes([
            central[offset + 32],
            central[offset + 33],
        ]));
        let name_start = fixed_end;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| io::Error::other("ZIP name length overflow"))?;
        let record_end = name_end
            .checked_add(extra_len)
            .and_then(|end| end.checked_add(comment_len))
            .ok_or_else(|| io::Error::other("ZIP record length overflow"))?;
        if record_end > central.len() {
            return Ok(false);
        }
        if index < MAX_ZIP_ENTRIES && installer_archive_path(&central[name_start..name_end]) {
            installer = true;
        }
        offset = record_end;
    }
    Ok(offset <= central.len() && installer)
}

fn installer_archive_path(name: &[u8]) -> bool {
    [
        b".deb".as_slice(),
        b".rpm",
        b".appimage",
        b".pkg.tar.zst",
        b".iso",
        b".flatpakref",
    ]
    .iter()
    .any(|extension| ends_ascii_case(name, extension))
        || is_tar_name(name)
}

fn display_path(path: &Path) -> String {
    let mut result = String::new();
    for byte in path.as_os_str().as_encoded_bytes() {
        match *byte {
            b' '..=b'~' if *byte != b'\\' => result.push(char::from(*byte)),
            b'\\' => result.push_str("\\\\"),
            b'\n' => result.push_str("\\n"),
            b'\r' => result.push_str("\\r"),
            b'\t' => result.push_str("\\t"),
            value => result.push_str(&format!("\\x{value:02x}")),
        }
    }
    result
}
