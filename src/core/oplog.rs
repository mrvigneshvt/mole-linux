use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

const MAX_TAIL_BYTES: u64 = 1024 * 1024;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 200;
pub const OUTCOME_NAMES: [&str; 6] = [
    "applied",
    "unchanged",
    "skipped",
    "unavailable",
    "attention",
    "failed",
];

#[derive(Debug, Serialize)]
struct History {
    logs: LogPaths,
    limit: usize,
    sessions: Vec<HistorySession>,
    deletions: Vec<Deletion>,
}

#[derive(Debug, Serialize)]
struct LogPaths {
    operations: String,
    deletions: String,
}

#[derive(Debug, Serialize)]
struct HistorySession {
    command: String,
    run_id: String,
    attribution: String,
    started_at: String,
    ended_at: String,
    items: u64,
    size: String,
    operation_count: u64,
    failed_tasks: u64,
    malformed_summaries: u64,
    ambiguous: bool,
    actions: Actions,
}

#[derive(Debug, Default, Serialize)]
struct Actions {
    removed: u64,
    trashed: u64,
    skipped: u64,
    failed: u64,
    rebuilt: u64,
    other: u64,
}

#[derive(Debug, Serialize)]
struct Deletion {
    timestamp: String,
    mode: String,
    status: String,
    size_kb: Option<u64>,
    path: String,
}

#[derive(Default)]
struct SessionBuilder {
    command: String,
    run_id: String,
    started_at: String,
    ended_at: String,
    items: u64,
    size: String,
    operation_count: u64,
    failed_tasks: u64,
    malformed_summaries: u64,
    ambiguous: bool,
    marker_sequence: u64,
    actions: Actions,
}

pub fn run(json: bool, limit: Option<usize>) -> io::Result<()> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--limit must be between 1 and 200",
        ));
    }
    let paths = log_paths()?;
    let sessions = load_sessions(&paths.operations)?;
    let deletions = load_deletions(&paths.deletions)?;
    let history = History {
        logs: LogPaths {
            operations: paths.operations.display().to_string(),
            deletions: paths.deletions.display().to_string(),
        },
        limit,
        sessions: sessions.into_iter().rev().take(limit).collect(),
        deletions: deletions.into_iter().rev().take(limit).collect(),
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&history).map_err(io::Error::other)?
        );
    } else {
        render_text(&history);
    }
    Ok(())
}

struct Paths {
    operations: PathBuf,
    deletions: PathBuf,
}

fn log_paths() -> io::Result<Paths> {
    let paths = crate::core::paths::XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    let directory = paths.state.join("mole-linux");
    Ok(Paths {
        operations: directory.join("operations.log"),
        deletions: directory.join("deletions.log"),
    })
}

fn read_tail(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let size = file.metadata()?.len();
    let start = size.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let capacity = usize::try_from(size - start)
        .map_err(|_| io::Error::other("history log tail does not fit memory address space"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_TAIL_BYTES).read_to_end(&mut bytes)?;
    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }
    Ok(bytes)
}

// Ported from tw93/Mole lib/core/history.sh:history_load_operations (GPL-3.0).
fn load_sessions(path: &Path) -> io::Result<Vec<HistorySession>> {
    let bytes = read_tail(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut sessions: Vec<SessionBuilder> = Vec::new();
    let mut active: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut marker_sequence = 0_u64;
    for line in text.lines() {
        if let Some((identity, started)) = parse_marker(line, " session started at ") {
            let Some((command, run_id)) = parse_identity(identity) else {
                continue;
            };
            marker_sequence = marker_sequence.saturating_add(1);
            let key = (command.clone(), run_id.clone());
            let ambiguous = run_id.is_empty() && active.contains_key(&key);
            if let Some(previous) = active.get(&key).copied() {
                sessions[previous].ended_at = started.to_string();
                if ambiguous {
                    sessions[previous].ambiguous = true;
                }
            }
            let index = sessions.len();
            sessions.push(SessionBuilder {
                command: command.clone(),
                run_id: run_id.clone(),
                started_at: started.to_string(),
                size: "0B".to_string(),
                ambiguous,
                marker_sequence,
                ..SessionBuilder::default()
            });
            active.insert(key, index);
            continue;
        }
        if let Some((identity, rest)) = parse_end_marker(line) {
            let Some((command, run_id)) = parse_identity(identity) else {
                continue;
            };
            let key = (command.clone(), run_id.clone());
            let index = *active.entry(key.clone()).or_insert_with(|| {
                sessions.push(SessionBuilder {
                    command: command.clone(),
                    run_id: run_id.clone(),
                    size: "0B".to_string(),
                    marker_sequence: marker_sequence.saturating_add(1),
                    ..SessionBuilder::default()
                });
                sessions.len() - 1
            });
            let session = &mut sessions[index];
            let (ended, items, size, malformed) = parse_end_summary(rest);
            if session.started_at.is_empty() {
                session.started_at = ended.to_string();
            }
            session.ended_at = ended.to_string();
            session.items = items.unwrap_or(0);
            if let Some(size) = size {
                session.size = size.to_string();
            }
            if malformed {
                session.malformed_summaries = session.malformed_summaries.saturating_add(1);
            }
            active.remove(&key);
            continue;
        }
        if let Some((timestamp, identity, action)) = parse_operation(line) {
            let Some((command, run_id)) = parse_identity(identity) else {
                continue;
            };
            let key = (command.clone(), run_id.clone());
            let index = *active.entry(key).or_insert_with(|| {
                marker_sequence = marker_sequence.saturating_add(1);
                sessions.push(SessionBuilder {
                    command: command.clone(),
                    run_id: run_id.clone(),
                    started_at: timestamp.to_string(),
                    size: "0B".to_string(),
                    marker_sequence,
                    ..SessionBuilder::default()
                });
                sessions.len() - 1
            });
            let session = &mut sessions[index];
            session.operation_count = session.operation_count.saturating_add(1);
            let count = match action {
                "REMOVED" => &mut session.actions.removed,
                "TRASHED" => &mut session.actions.trashed,
                "SKIPPED" => &mut session.actions.skipped,
                "FAILED" => &mut session.actions.failed,
                "REBUILT" => &mut session.actions.rebuilt,
                "TASK_FAILED" => &mut session.failed_tasks,
                _ => &mut session.actions.other,
            };
            *count = count.saturating_add(1);
        }
    }
    sessions.sort_by_key(|session| {
        (
            if session.started_at.is_empty() {
                session.ended_at.clone()
            } else {
                session.started_at.clone()
            },
            session.marker_sequence,
        )
    });
    Ok(sessions
        .into_iter()
        .map(|session| HistorySession {
            command: session.command,
            attribution: if session.ambiguous {
                "ambiguous".to_string()
            } else if session.run_id.is_empty() {
                "command".to_string()
            } else {
                "run".to_string()
            },
            run_id: session.run_id,
            started_at: session.started_at,
            ended_at: session.ended_at,
            items: session.items,
            size: session.size,
            operation_count: session.operation_count,
            failed_tasks: session.failed_tasks,
            malformed_summaries: session.malformed_summaries,
            ambiguous: session.ambiguous,
            actions: session.actions,
        })
        .collect())
}

fn parse_identity(identity: &str) -> Option<(String, String)> {
    match identity.split_once(" run=") {
        Some((command, run_id))
            if !command.is_empty()
                && !run_id.is_empty()
                && run_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) =>
        {
            Some((command.to_string(), run_id.to_string()))
        }
        Some(_) => None,
        None if !identity.is_empty() => Some((identity.to_string(), String::new())),
        None => None,
    }
}

fn parse_marker<'a>(line: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let rest = line.strip_prefix("# ========== ")?;
    let (command, rest) = rest.split_once(marker)?;
    let started = rest.strip_suffix(" ==========")?;
    (!command.is_empty()).then_some((command, started))
}

fn parse_end_marker(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("# ========== ")?;
    let (command, rest) = rest.split_once(" session ended at ")?;
    Some((command, rest.strip_suffix(" ==========")?))
}

fn parse_end_summary(rest: &str) -> (&str, Option<u64>, Option<&str>, bool) {
    let Some((ended, tail)) = rest.split_once(", ") else {
        return (rest, None, None, !valid_timestamp(rest));
    };
    let Some((items, size)) = tail.split_once(" items, ") else {
        return (ended, None, None, true);
    };
    match (
        valid_timestamp(ended),
        items.parse::<u64>(),
        valid_size(size),
    ) {
        (true, Ok(items), true) => (ended, Some(items), Some(size), false),
        _ => (ended, None, None, true),
    }
}

fn valid_size(value: &str) -> bool {
    let Some(number) = ["KB", "MB", "GB", "TB", "PB", "B"]
        .iter()
        .find_map(|unit| value.strip_suffix(unit))
    else {
        return false;
    };
    number
        .parse::<f64>()
        .is_ok_and(|size| size.is_finite() && size >= 0.0)
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && [4, 7].iter().all(|index| bytes[*index] == b'-')
        && bytes[10] == b' '
        && [13, 16].iter().all(|index| bytes[*index] == b':')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [4, 7, 10, 13, 16].contains(&index) || byte.is_ascii_digit())
}

fn parse_operation(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.strip_prefix('[')?;
    let (timestamp, rest) = rest.split_once("] [")?;
    let (command, rest) = rest.split_once("] ")?;
    let (action, _) = rest.split_once(' ')?;
    Some((timestamp, command, action))
}

// Ported from tw93/Mole lib/core/history.sh:history_load_deletions (GPL-3.0).
fn load_deletions(path: &Path) -> io::Result<Vec<Deletion>> {
    let bytes = read_tail(path)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            let timestamp = decode_field(fields.next()?)?;
            let mode = decode_field(fields.next()?)?;
            let size = decode_field(fields.next()?)?;
            let status = decode_field(fields.next()?)?;
            let path = decode_field(fields.next()?)?;
            if timestamp.is_empty() || mode.is_empty() || status.is_empty() {
                return None;
            }
            Some(Deletion {
                timestamp,
                mode,
                status,
                size_kb: size.parse().ok(),
                path,
            })
        })
        .collect())
}

fn decode_field(field: &str) -> Option<String> {
    let mut out = Vec::with_capacity(field.len());
    let bytes = field.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() && bytes[index + 1] == b'x' {
            let hex = std::str::from_utf8(&bytes[index + 2..index + 4]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 4;
        } else if bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'\\') {
            out.push(b'\\');
            index += 2;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Appends one TSV deletion record, escaping controls so fields cannot add rows or columns.
pub fn append_deletion(
    timestamp: &str,
    mode: &str,
    size_kb: Option<u64>,
    status: &str,
    path: &str,
) -> io::Result<()> {
    let paths = log_paths()?;
    append_deletion_to(&paths.deletions, timestamp, mode, size_kb, status, path)
}

#[derive(Clone, Debug)]
pub struct OpLog {
    operations: PathBuf,
    deletions: PathBuf,
}

#[derive(Debug)]
pub struct SessionSummary {
    pub items: u64,
    pub size: String,
}

#[derive(Debug)]
pub struct Session {
    operations: PathBuf,
    command: String,
    run_id: String,
    ended: bool,
}

impl OpLog {
    pub fn new() -> io::Result<Self> {
        let paths = log_paths()?;
        Ok(Self {
            operations: paths.operations,
            deletions: paths.deletions,
        })
    }

    pub fn with_state_home(state_home: &Path) -> Self {
        let directory = state_home.join("mole-linux");
        Self {
            operations: directory.join("operations.log"),
            deletions: directory.join("deletions.log"),
        }
    }

    pub fn start_session(&self, command: &str, dry_run: bool) -> io::Result<Session> {
        let command = if dry_run {
            format!("{command}-dry-run")
        } else {
            command.to_string()
        };
        let run_id = run_id()?;
        let identity = format!("{command} run={run_id}");
        append_line(
            &self.operations,
            &format!(
                "# ========== {} session started at {} ==========",
                escape_field(&identity),
                timestamp()?
            ),
        )?;
        Ok(Session {
            operations: self.operations.clone(),
            command,
            run_id,
            ended: false,
        })
    }

    pub fn append_deletion(
        &self,
        timestamp: &str,
        mode: &str,
        size_kb: Option<u64>,
        status: &str,
        path: &str,
    ) -> io::Result<()> {
        append_deletion_to(&self.deletions, timestamp, mode, size_kb, status, path)
    }
}

impl Session {
    pub fn record(&mut self, action: &str, outcome: &str, path: &str) -> io::Result<()> {
        if !OUTCOME_NAMES.contains(&outcome) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid operation outcome: {outcome}"),
            ));
        }
        if self.ended {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot record operation after session end",
            ));
        }
        let identity = format!("{} run={}", self.command, self.run_id);
        append_line(
            &self.operations,
            &format!(
                "[{}] [{}] {} {} {}",
                timestamp()?,
                escape_field(&identity),
                escape_field(action),
                escape_field(outcome),
                escape_field(path)
            ),
        )
    }

    pub fn end(&mut self, summary: &SessionSummary) -> io::Result<()> {
        if self.ended {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "session already ended",
            ));
        }
        let identity = format!("{} run={}", self.command, self.run_id);
        append_line(
            &self.operations,
            &format!(
                "# ========== {} session ended at {}, {} items, {} ==========",
                escape_field(&identity),
                timestamp()?,
                summary.items,
                escape_field(&summary.size)
            ),
        )?;
        self.ended = true;
        Ok(())
    }
}

fn run_id() -> io::Result<String> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?;
    Ok(format!("{}-{}", std::process::id(), elapsed.as_nanos()))
}

fn timestamp() -> io::Result<String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    Ok(format_utc_timestamp(seconds))
}

fn format_utc_timestamp(seconds: u64) -> String {
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let z = i128::from(days) + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        day_seconds / 3_600,
        day_seconds % 3_600 / 60,
        day_seconds % 60
    )
}

fn append_deletion_to(
    log_path: &Path,
    timestamp: &str,
    mode: &str,
    size_kb: Option<u64>,
    status: &str,
    path: &str,
) -> io::Result<()> {
    append_line(
        log_path,
        &[
            escape_field(timestamp),
            escape_field(mode),
            size_kb.map_or_else(|| "unknown".to_string(), |size| size.to_string()),
            escape_field(status),
            escape_field(path),
        ]
        .join("\t"),
    )
}

/// Starts a session using the upstream operation-log marker format.
pub fn start_session(command: &str, timestamp: &str) -> io::Result<()> {
    let paths = log_paths()?;
    start_session_to(&paths.operations, command, timestamp)
}

/// Records one action in a session using the upstream operation-log format.
pub fn record_operation(
    timestamp: &str,
    command: &str,
    action: &str,
    detail: &str,
) -> io::Result<()> {
    let paths = log_paths()?;
    append_operation_to(&paths.operations, timestamp, command, action, detail)
}

fn append_operation_to(
    log_path: &Path,
    timestamp: &str,
    command: &str,
    action: &str,
    detail: &str,
) -> io::Result<()> {
    append_line(
        log_path,
        &format!(
            "[{}] [{}] {} {}",
            escape_field(timestamp),
            escape_field(command),
            escape_field(action),
            escape_field(detail)
        ),
    )
}

/// Ends a session with its summary using the upstream operation-log format.
pub fn end_session(command: &str, timestamp: &str, items: u64, size: &str) -> io::Result<()> {
    let paths = log_paths()?;
    end_session_to(&paths.operations, command, timestamp, items, size)
}

fn start_session_to(log_path: &Path, command: &str, timestamp: &str) -> io::Result<()> {
    append_line(
        log_path,
        &format!(
            "# ========== {} session started at {} ==========",
            escape_field(command),
            escape_field(timestamp)
        ),
    )
}

fn end_session_to(
    log_path: &Path,
    command: &str,
    timestamp: &str,
    items: u64,
    size: &str,
) -> io::Result<()> {
    append_line(
        log_path,
        &format!(
            "# ========== {} session ended at {}, {} items, {} ==========",
            escape_field(command),
            escape_field(timestamp),
            items,
            escape_field(size)
        ),
    )
}

fn escape_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let characters: Vec<char> = value.chars().collect();
    for (index, character) in characters.iter().copied().enumerate() {
        if character.is_control() {
            for byte in character.to_string().bytes() {
                escaped.push_str(&format!("\\x{byte:02x}"));
            }
        } else if character == '\\'
            && characters.get(index + 1) == Some(&'x')
            && characters
                .get(index + 2..index + 4)
                .is_some_and(|digits| digits.iter().all(char::is_ascii_hexdigit))
        {
            escaped.push_str("\\\\");
        } else {
            escaped.push(character);
        }
    }
    escaped
}

/// Uses `O_APPEND` and one write call so cooperating writers append whole records.
fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut record = Vec::with_capacity(line.len().saturating_add(1));
    record.extend_from_slice(line.as_bytes());
    record.push(b'\n');
    let written = file.write(&record)?;
    if written != record.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "operation log append was not written atomically",
        ));
    }
    Ok(())
}

fn render_text(history: &History) {
    println!("\nMole History\n\nRecent sessions");
    if history.sessions.is_empty() {
        println!("  No operation history yet.");
    }
    for session in &history.sessions {
        println!(
            "  {:<10} {}, {} items, {}",
            visible(&session.command),
            visible(&session.started_at),
            session.items,
            visible(&session.size)
        );
        println!(
            "             removed {}, trashed {}, skipped {}, failed {}, ended {}",
            session.actions.removed,
            session.actions.trashed,
            session.actions.skipped,
            session.actions.failed,
            if session.ended_at.is_empty() {
                "not ended".to_string()
            } else {
                visible(&session.ended_at)
            }
        );
        if session.failed_tasks > 0 {
            println!(
                "             {} optimize tasks failed",
                session.failed_tasks
            );
        }
        if session.malformed_summaries > 0 {
            println!(
                "             {} malformed session summaries",
                session.malformed_summaries
            );
        }
        if session.ambiguous {
            println!("             legacy run attribution unknown run");
        }
    }
    println!("\nDeletion audit");
    if history.deletions.is_empty() {
        println!("  No deletion audit entries yet.");
    }
    for deletion in &history.deletions {
        println!(
            "  {:<24} {:<9} {:<12} {}  {}",
            visible(&deletion.timestamp),
            visible(&deletion.mode),
            visible(&deletion.status),
            deletion
                .size_kb
                .map_or_else(|| "unknown".to_string(), |size| format!("{size}KB")),
            visible(&deletion.path)
        );
    }
    println!(
        "\nLogs\n  operations: {}\n  deletions:  {}",
        visible(&history.logs.operations),
        visible(&history.logs.deletions)
    );
}

fn visible(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            for byte in character.to_string().bytes() {
                escaped.push_str(&format!("\\x{byte:02x}"));
            }
        } else {
            escaped.push(character);
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TAIL_BYTES, OUTCOME_NAMES, OpLog, SessionSummary, append_deletion_to, append_line,
        append_operation_to, end_session_to, escape_field, load_deletions, load_sessions,
        read_tail, start_session_to,
    };

    #[test]
    fn large_log_tail_is_bounded_before_parsing() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("large.log");
        let mut file = std::fs::File::create(&path).expect("create log");
        for _ in 0..120_000 {
            file.write_all(b"operation row padding\n")
                .expect("write log row");
        }
        let tail = read_tail(&path).expect("bounded read");
        assert!(tail.len() <= usize::try_from(MAX_TAIL_BYTES).expect("bound fits usize"));
        assert!(tail.len() > 1_000_000);
        assert_eq!(tail.last(), Some(&b'\n'));
    }

    #[test]
    fn deletion_writer_escapes_controls_and_history_decodes_them() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("deletions.log");
        append_deletion_to(
            &path,
            "2026-10-19T10:00:00+0000",
            "trash",
            Some(3),
            "ok",
            "/tmp/a\tb\nc",
        )
        .expect("append deletion");
        let bytes = std::fs::read(&path).expect("read log");
        assert!(!bytes.contains(&b'\t') || bytes.split(|byte| *byte == b'\t').count() == 5);
        assert_eq!(bytes.split(|byte| *byte == b'\n').count(), 2);
        let entries = load_deletions(&path).expect("load log");
        assert_eq!(entries[0].path, "/tmp/a\tb\nc");
    }

    #[test]
    fn deletion_round_trip_preserves_literal_escape_looking_backslashes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("deletions.log");
        let original = "/tmp/literal\\x41-and-tab\tend";
        append_deletion_to(&path, "stamp", "trash", Some(1), "ok", original)
            .expect("append deletion");
        let entries = load_deletions(&path).expect("load log");
        assert_eq!(entries[0].path, original);
    }

    #[test]
    fn session_writer_emits_parseable_start_actions_and_end_summary() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("operations.log");
        start_session_to(&path, "clean", "2026-10-19 10:00:00").expect("start session");
        append_operation_to(
            &path,
            "2026-10-19 10:00:01",
            "clean",
            "TRASHED",
            "/tmp/a\nforged",
        )
        .expect("record action");
        end_session_to(&path, "clean", "2026-10-19 10:00:02", 1, "2KB").expect("end session");
        let sessions = super::load_sessions(&path).expect("load written sessions");
        assert_eq!(sessions[0].command, "clean");
        assert_eq!(sessions[0].actions.trashed, 1);
        assert_eq!(sessions[0].items, 1);
        assert_eq!(sessions[0].size, "2KB");
    }

    #[test]
    fn public_writer_api_marks_preview_and_preserves_canonical_outcome_names() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let log = OpLog::with_state_home(directory.path());
        let mut session = log.start_session("clean", true).expect("start preview");
        session
            .record("TASK_FAILED", "failed", "/tmp/path\twith-control")
            .expect("record outcome");
        for outcome in OUTCOME_NAMES {
            session
                .record("STATE", outcome, "/tmp/outcome")
                .expect("record canonical outcome");
        }
        session
            .end(&SessionSummary {
                items: 1,
                size: "0B".to_string(),
            })
            .expect("end preview");
        let bytes = std::fs::read(directory.path().join("mole-linux/operations.log"))
            .expect("read operations log");
        let text = String::from_utf8(bytes).expect("UTF-8 log");
        assert!(text.contains("[clean-dry-run run="));
        assert!(text.contains("TASK_FAILED failed /tmp/path\\x09with-control"));
        assert_eq!(text.lines().count(), 9);
        assert!(
            !text
                .as_bytes()
                .iter()
                .any(|byte| *byte < 0x20 && *byte != b'\n')
        );
        for outcome in OUTCOME_NAMES {
            assert!(text.contains(&format!("STATE {outcome} /tmp/outcome")));
        }
        let sessions = super::load_sessions(&directory.path().join("mole-linux/operations.log"))
            .expect("load written session");
        assert_eq!(sessions[0].command, "clean-dry-run");
        assert_eq!(sessions[0].failed_tasks, 1);
    }

    #[test]
    fn escaping_preserves_backslashes_and_removes_all_control_bytes() {
        let value: String = (0_u32..=255)
            .map(|code| char::from_u32(code).expect("valid scalar"))
            .collect();
        let escaped = escape_field(&value);
        assert!(!escaped.chars().any(char::is_control));
        assert!(escaped.contains("\\"));
        assert!(escaped.contains("\\x00"));
        assert!(escaped.contains("\\x7f"));
    }

    #[test]
    fn ten_thousand_unended_runs_are_loaded_from_the_fixed_tail() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("operations.log");
        let mut file = std::fs::File::create(&path).expect("create log");
        for index in 0..10_000_u64 {
            writeln!(
                file,
                "# ========== clean run={index} session started at 2026-10-19 10:00:00 ==========\n[2026-10-19 10:00:01] [clean run={index}] REMOVED /tmp/a-long-path-that-keeps-each-record-large-enough-{index} (1KB)"
            )
            .expect("write start marker");
        }
        drop(file);
        let tail = read_tail(&path).expect("read fixed tail");
        assert!(tail.len() <= usize::try_from(MAX_TAIL_BYTES).expect("bound fits usize"));
        assert!(tail.len() > 1_000_000);
        let sessions = load_sessions(&path).expect("parse bounded tail");
        assert!(sessions.len() < 10_000);
        assert!(sessions.len() > 1_000);
    }

    #[test]
    fn concurrent_records_are_whole_lines() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("operations.log");
        let mut threads = Vec::new();
        for worker in 0..8 {
            let path = path.clone();
            threads.push(std::thread::spawn(move || {
                for index in 0..100 {
                    append_line(&path, &format!("worker-{worker}-record-{index}"))
                        .expect("append whole record");
                }
            }));
        }
        for thread in threads {
            thread.join().expect("writer thread");
        }
        let text = std::fs::read_to_string(path).expect("read concurrent records");
        let rows: Vec<_> = text.lines().collect();
        assert_eq!(rows.len(), 800);
        assert!(rows.iter().all(|row| {
            let Some((worker, record)) = row.split_once("-record-") else {
                return false;
            };
            worker.starts_with("worker-") && record.parse::<u16>().is_ok()
        }));
        assert_eq!(
            OUTCOME_NAMES,
            [
                "applied",
                "unchanged",
                "skipped",
                "unavailable",
                "attention",
                "failed"
            ]
        );
    }

    use std::io::Write;
}
