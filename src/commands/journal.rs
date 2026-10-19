//! M6.3 journald: report journal disk usage and vacuum archived journal files only.
//!
//! Behavior source: journalctl(1) and systemd-journald(8) at systemd 261. No upstream Mole
//! contract covers journald, so this unit is Linux-native (Adapt), not a port. Vacuum
//! removes archived files only; active journals are never rotated, flushed or deleted here.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRunner};

pub const MAX_JOURNAL_ENTRIES: usize = 10_000;
const MACHINE_ID_PATH: &str = "/etc/machine-id";
const JOURNAL_ROOT: &str = "/var/log/journal";
const MACHINE_ID_LIMIT: u64 = 64;
const MAX_SIZE_MIB: u32 = 1_048_576;
const MAX_AGE_DAYS: u16 = 3650;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionError {
    SizeOutOfRange,
    AgeOutOfRange,
}

/// Archived-file limit passed to `journalctl --vacuum-*`. Values are bounded at construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retention {
    SizeMiB(u32),
    AgeDays(u16),
}

impl Retention {
    pub fn size_mib(mib: u32) -> Result<Self, RetentionError> {
        if (1..=MAX_SIZE_MIB).contains(&mib) {
            Ok(Self::SizeMiB(mib))
        } else {
            Err(RetentionError::SizeOutOfRange)
        }
    }

    pub fn age_days(days: u16) -> Result<Self, RetentionError> {
        if (1..=MAX_AGE_DAYS).contains(&days) {
            Ok(Self::AgeDays(days))
        } else {
            Err(RetentionError::AgeOutOfRange)
        }
    }

    pub fn arg(self) -> String {
        match self {
            Self::SizeMiB(mib) => format!("--vacuum-size={mib}M"),
            Self::AgeDays(days) => format!("--vacuum-time={days}days"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Usage {
    /// journalctl's own value for archived plus active journals, e.g. `286.8M`.
    Reported(String),
    Unknown(String),
}

const DISK_USAGE_PREFIX: &str = "Archived and active journals take up ";
const DISK_USAGE_SUFFIX: &str = " in the file system.";

/// Extracts journalctl's size token. Anything that is not `<number>[unit]` is rejected.
pub fn parse_disk_usage(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    let value = line
        .strip_prefix(DISK_USAGE_PREFIX)?
        .strip_suffix(DISK_USAGE_SUFFIX)?;
    is_size_token(value).then(|| value.to_owned())
}

fn is_size_token(value: &str) -> bool {
    let number_end = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(number_end);
    let number_ok = number.starts_with(|c: char| c.is_ascii_digit())
        && !number.ends_with('.')
        && number.matches('.').count() <= 1;
    number_ok && matches!(unit, "" | "B" | "K" | "M" | "G" | "T" | "P" | "E")
}

/// Reads `journalctl --disk-usage` through the owner runner's bounded query (unprivileged, no sudo).
pub fn disk_usage(runner: &OwnerRunner) -> Usage {
    let cmd = OwnerCommand::new(OwnerProgram::Journalctl, ["--disk-usage"], false);
    match runner.query(&cmd) {
        Ok(output) => match parse_disk_usage(&output) {
            Some(value) => Usage::Reported(value),
            None => Usage::Unknown("journalctl printed an unrecognized disk-usage line".to_owned()),
        },
        Err(run) => Usage::Unknown(
            run.reason
                .unwrap_or_else(|| "journalctl did not report disk usage".to_owned()),
        ),
    }
}

/// The journal directory for this machine: `/var/log/journal/<machine-id>`.
pub fn journal_dir() -> io::Result<PathBuf> {
    journal_dir_from(Path::new(MACHINE_ID_PATH), Path::new(JOURNAL_ROOT))
}

/// Joins the machine id into the journal root. The id is a path component, so it must be 32 lowercase hex digits.
pub fn journal_dir_from(machine_id_file: &Path, journal_root: &Path) -> io::Result<PathBuf> {
    let mut raw = String::new();
    fs::File::open(machine_id_file)?
        .take(MACHINE_ID_LIMIT)
        .read_to_string(&mut raw)?;
    let id = raw.trim_end();
    if !is_machine_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "machine-id is not 32 lowercase hex digits",
        ));
    }
    Ok(journal_root.join(id))
}

fn is_machine_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Inventory {
    pub archived: usize,
    pub active: Vec<String>,
    /// Corrupt or unrecognized entries. Not counted as archived, so never a vacuum target by our count.
    pub other: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Active,
    Archived,
    Other,
}

/// systemd-journald(8): an archived file is renamed to `original@suffix.journal` (or `.journal~`).
/// A plain `name.journal` is active. Anything else is left unclassified.
fn classify(name: &str) -> Kind {
    if name.ends_with(".journal") || name.ends_with(".journal~") {
        if name.contains('@') {
            Kind::Archived
        } else if name.ends_with(".journal") {
            Kind::Active
        } else {
            Kind::Other
        }
    } else {
        Kind::Other
    }
}

/// Counts journal files in `dir` without following symlinks. Refuses directories past the entry bound.
pub fn inventory(dir: &Path) -> io::Result<Inventory> {
    let mut found = Inventory::default();
    for (seen, entry) in fs::read_dir(dir)?.enumerate() {
        if seen >= MAX_JOURNAL_ENTRIES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("journal directory has more than {MAX_JOURNAL_ENTRIES} entries"),
            ));
        }
        let entry = entry?;
        let is_file = entry.file_type()?.is_file();
        let name = entry.file_name();
        let kind = match name.to_str() {
            Some(text) if is_file => classify(text),
            _ => Kind::Other,
        };
        match kind {
            Kind::Archived => found.archived += 1,
            Kind::Active => found.active.push(name.to_string_lossy().into_owned()),
            Kind::Other => found.other += 1,
        }
    }
    found.active.sort();
    Ok(found)
}
