use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

const MAX_TAIL_BYTES: u64 = 1024 * 1024;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 200;

#[derive(Debug, Serialize)]
struct History {
    logs: LogPaths,
    limit: usize,
    sessions: Vec<Session>,
    deletions: Vec<Deletion>,
}

#[derive(Debug, Serialize)]
struct LogPaths {
    operations: String,
    deletions: String,
}

#[derive(Debug, Serialize)]
struct Session {
    command: String,
    run_id: String,
    attribution: String,
    started_at: String,
    ended_at: String,
    items: u64,
    size: String,
    operation_count: u64,
    failed_tasks: u64,
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
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"));
    let directory = state.join("mole-linux");
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
fn load_sessions(path: &Path) -> io::Result<Vec<Session>> {
    let bytes = read_tail(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut sessions: Vec<SessionBuilder> = Vec::new();
    let mut active: BTreeMap<(String, String), usize> = BTreeMap::new();
    for line in text.lines() {
        if let Some((identity, started)) = parse_marker(line, " session started at ") {
            let (command, run_id) = parse_identity(identity);
            let index = sessions.len();
            sessions.push(SessionBuilder {
                command: command.clone(),
                run_id: run_id.clone(),
                started_at: started.to_string(),
                size: "0B".to_string(),
                ..SessionBuilder::default()
            });
            active.insert((command, run_id), index);
            continue;
        }
        if let Some((identity, rest)) = parse_end_marker(line) {
            let (command, run_id) = parse_identity(identity);
            let index = *active
                .entry((command.clone(), run_id.clone()))
                .or_insert_with(|| {
                    sessions.push(SessionBuilder {
                        command: command.clone(),
                        run_id: run_id.clone(),
                        size: "0B".to_string(),
                        ..SessionBuilder::default()
                    });
                    sessions.len() - 1
                });
            let session = &mut sessions[index];
            let (ended, items, size) = parse_end_summary(rest);
            session.ended_at = ended.to_string();
            session.items = items.unwrap_or(session.operation_count);
            if let Some(size) = size {
                session.size = size.to_string();
            }
            continue;
        }
        if let Some((timestamp, identity, action)) = parse_operation(line) {
            let (command, run_id) = parse_identity(identity);
            let index = *active
                .entry((command.clone(), run_id.clone()))
                .or_insert_with(|| {
                    sessions.push(SessionBuilder {
                        command: command.clone(),
                        run_id: run_id.clone(),
                        started_at: timestamp.to_string(),
                        size: "0B".to_string(),
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
    Ok(sessions
        .into_iter()
        .map(|session| Session {
            command: session.command,
            attribution: if session.run_id.is_empty() {
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
            actions: session.actions,
        })
        .collect())
}

fn parse_identity(identity: &str) -> (String, String) {
    match identity.split_once(" run=") {
        Some((command, run_id))
            if !command.is_empty()
                && !run_id.is_empty()
                && run_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) =>
        {
            (command.to_string(), run_id.to_string())
        }
        _ => (identity.to_string(), String::new()),
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

fn parse_end_summary(rest: &str) -> (&str, Option<u64>, Option<&str>) {
    let Some((ended, tail)) = rest.split_once(", ") else {
        return (rest, None, None);
    };
    let Some((items, size)) = tail.split_once(" items, ") else {
        return (rest, None, None);
    };
    (ended, items.parse().ok(), Some(size))
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

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
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
                "not ended"
            } else {
                &session.ended_at
            }
        );
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
        history.logs.operations, history.logs.deletions
    );
}

fn visible(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TAIL_BYTES, append_deletion_to, append_operation_to, end_session_to, load_deletions,
        read_tail, start_session_to,
    };

    #[test]
    fn large_log_tail_is_bounded_before_parsing() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("large.log");
        let mut file = std::fs::File::create(&path).expect("create log");
        for _ in 0..3 {
            file.write_all(&vec![b'x'; 512 * 1024]).expect("write log");
            file.write_all(b"\n").expect("write delimiter");
        }
        let tail = read_tail(&path).expect("bounded read");
        assert!(tail.len() <= usize::try_from(MAX_TAIL_BYTES).expect("bound fits usize"));
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

    use std::io::Write;
}
