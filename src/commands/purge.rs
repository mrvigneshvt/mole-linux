// Ported from tw93/Mole lib/clean/project.sh:clean_project_artifacts (GPL-3.0).
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{self, Event, KeyCode};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::core::oplog::{OpLog, SessionSummary};
use crate::core::outcome::{Outcome, OutcomeLedger};
use crate::core::paths::XdgPaths;
use crate::core::sink::{DeleteError, DeleteMode, Identity, SinkContext, delete};
use crate::core::units::bytes_si;
use crate::core::whitelist::Whitelist;
use crate::purge::{self, Activity, Candidate, SizeEstimate};

#[derive(Clone, Debug)]
pub struct ReviewedCandidate {
    pub candidate: Candidate,
    identity: Identity,
}

impl ReviewedCandidate {
    pub fn capture(candidate: Candidate) -> io::Result<Self> {
        let identity = Identity::capture(&candidate.path)?;
        Ok(Self {
            candidate,
            identity,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PurgeReport {
    pub previewed: usize,
    pub trashed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub paths: Vec<PathBuf>,
    pub skipped_paths: Vec<(PathBuf, String)>,
    pub failed_paths: Vec<(PathBuf, String)>,
    pub outcome: Option<Outcome>,
}

impl PurgeReport {
    pub fn summary(&self, dry_run: bool) -> String {
        if dry_run {
            format!(
                "Dry run: {} eligible artifact(s) reviewed; nothing was removed.",
                self.previewed
            )
        } else {
            format!(
                "Trashed {} artifact(s); space is not freed until Trash is emptied.",
                self.trashed
            )
        }
    }
}

/// Applies only the explicitly selected reviewed paths. Recent candidates may be
/// selected after review; uncertain and policy-protected candidates never pass.
pub fn apply_selected(
    candidates: Vec<ReviewedCandidate>,
    selected: &[PathBuf],
    mode: DeleteMode,
    context: &SinkContext,
    oplog: &OpLog,
) -> io::Result<PurgeReport> {
    let mut entries = candidates
        .into_iter()
        .filter(|item| selected.contains(&item.candidate.path))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.candidate.path.cmp(&right.candidate.path));
    entries.dedup_by(|left, right| left.candidate.path == right.candidate.path);
    let mut report = PurgeReport::default();
    if entries.is_empty() {
        report.skipped = selected.len();
        set_outcome(&mut report)?;
        return Ok(report);
    }
    let item_count = u64::try_from(entries.len())
        .map_err(|_| io::Error::other("selected path count exceeds operation log range"))?;
    let mut session = oplog.start_session("purge", mode == DeleteMode::DryRun)?;
    for item in entries {
        let path = &item.candidate.path;
        let protected_reason = if item.candidate.protected {
            Some("protected by purge policy")
        } else if item.candidate.activity == Activity::Uncertain {
            Some("activity could not be verified")
        } else {
            None
        };
        if let Some(reason) = protected_reason {
            report.skipped = report.skipped.saturating_add(1);
            report.skipped_paths.push((path.clone(), reason.to_owned()));
            session.record("trash", "skipped", path)?;
            continue;
        }
        let current_activity = purge::classify_activity_with_budget(
            path,
            std::time::SystemTime::now(),
            std::time::Duration::from_secs(30),
        );
        if current_activity == Activity::Uncertain
            || (item.candidate.activity == Activity::Old && current_activity == Activity::Recent)
        {
            report.skipped = report.skipped.saturating_add(1);
            let reason = if current_activity == Activity::Uncertain {
                "activity could not be verified at final review"
            } else {
                "became recently active after review"
            };
            report.skipped_paths.push((path.clone(), reason.to_owned()));
            session.record("trash", "skipped", path)?;
            continue;
        }
        match delete(path, &item.identity, mode, context) {
            Ok(_) if mode == DeleteMode::DryRun => {
                report.previewed = report.previewed.saturating_add(1);
                session.record("review", "unchanged", path)?;
            }
            Ok(_) => {
                report.trashed = report.trashed.saturating_add(1);
                report.paths.push(path.clone());
                session.record("trash", "applied", path)?;
            }
            Err(
                error @ (DeleteError::IdentityChanged
                | DeleteError::Whitelisted
                | DeleteError::Cancelled
                | DeleteError::InvalidPath(_)
                | DeleteError::CrossFilesystemTrash
                | DeleteError::MountIdUnavailable),
            ) => {
                report.skipped = report.skipped.saturating_add(1);
                report
                    .skipped_paths
                    .push((path.clone(), visible_text(&error.to_string())));
                session.record("trash", "skipped", path)?;
            }
            Err(
                error @ (DeleteError::Partial(_)
                | DeleteError::TrashMetadataCleanup(_)
                | DeleteError::Io(_)),
            ) => {
                report.failed = report.failed.saturating_add(1);
                report
                    .failed_paths
                    .push((path.clone(), visible_text(&error.to_string())));
                session.record("trash", "failed", path)?;
            }
        }
    }
    session.end(&SessionSummary {
        items: item_count,
        size: format!("{item_count} paths"),
    })?;
    set_outcome(&mut report)?;
    Ok(report)
}

fn set_outcome(report: &mut PurgeReport) -> io::Result<()> {
    let count = |value: usize| {
        u64::try_from(value).map_err(|_| io::Error::other("purge outcome count exceeds u64"))
    };
    let mut ledger = OutcomeLedger::default();
    ledger
        .start()
        .map_err(|error| io::Error::other(format!("purge outcome: {error:?}")))?;
    ledger
        .report_from_counts(
            count(report.trashed)?,
            count(report.failed)?,
            count(report.skipped)?,
        )
        .map_err(|error| io::Error::other(format!("purge outcome: {error:?}")))?;
    ledger
        .finish("purge")
        .map_err(|error| io::Error::other(format!("purge outcome: {error:?}")))?;
    report.outcome = [
        Outcome::Failed,
        Outcome::Applied,
        Outcome::Skipped,
        Outcome::Unchanged,
    ]
    .into_iter()
    .find(|outcome| ledger.count(*outcome) > 0);
    Ok(())
}

pub fn run(dry_run: bool, paths: Option<String>, include_empty: bool, yes: bool) -> io::Result<()> {
    let xdg = XdgPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    let config = xdg.config.join("mole-linux/purge_paths");
    let roots = match paths.as_deref() {
        Some(paths) => paths.split(',').map(PathBuf::from).collect(),
        None => purge::load_search_paths(&xdg.home, &config)?,
    };
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")?;
    let mut candidates = Vec::new();
    let mut scan_errors = Vec::new();
    let mut attempted_roots = 0_usize;
    let mut successful_roots = 0_usize;
    for root in roots {
        match fs::symlink_metadata(&root) {
            Ok(_) => attempted_roots += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                attempted_roots += 1;
                scan_errors.push((root, error));
                continue;
            }
        }
        match purge::scan_artifacts_with_context(
            &root,
            8,
            std::time::Duration::from_secs(30),
            &xdg.home,
            &mountinfo,
        ) {
            Ok(found) => {
                successful_roots += 1;
                candidates.extend(found);
            }
            Err(error) => scan_errors.push((root, error)),
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates.dedup_by(|left, right| left.path == right.path);
    if attempted_roots > 0 && successful_roots == 0 && !scan_errors.is_empty() {
        for (root, error) in &scan_errors {
            eprintln!(
                "Unavailable {}: {}",
                visible_path(root, &xdg.home),
                visible_text(&error.to_string())
            );
        }
        return Err(io::Error::other(
            "Purge inventory unavailable; no artifacts were changed",
        ));
    }
    if candidates.is_empty() && scan_errors.is_empty() {
        println!("No project artifacts found.");
        return Ok(());
    }
    let mut reviewed = Vec::new();
    for candidate in candidates {
        let path = candidate.path.clone();
        if let Some(reason) = &candidate.unavailable_reason {
            eprintln!(
                "Unavailable {}: {}",
                visible_path(&path, &xdg.home),
                visible_text(reason)
            );
            match ReviewedCandidate::capture(candidate) {
                Ok(item) => reviewed.push(item),
                Err(error) => scan_errors.push((path, error)),
            }
            continue;
        }
        if include_empty || candidate.size != SizeEstimate::Known(0) {
            match ReviewedCandidate::capture(candidate) {
                Ok(item) => reviewed.push(item),
                Err(error) => scan_errors.push((path, error)),
            }
        }
    }
    if !scan_errors.is_empty() {
        for (root, error) in &scan_errors {
            eprintln!(
                "Unavailable {}: {}",
                visible_path(root, &xdg.home),
                visible_text(&error.to_string())
            );
        }
    }
    if reviewed.is_empty() {
        println!("No eligible project artifacts found.");
        return Ok(());
    }

    let selected = if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        select_interactively(&reviewed)?
    } else {
        reviewed
            .iter()
            .filter(|item| item.candidate.selected_by_default)
            .map(|item| item.candidate.path.clone())
            .collect()
    };
    if dry_run {
        print_preview(&reviewed, &selected, &xdg.home);
    }
    if selected.is_empty() {
        println!("No artifacts selected.");
        return Ok(());
    }
    if !dry_run && !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unattended purge requires --yes after reviewing the selected paths",
            ));
        }
        if !confirm(&selected, &xdg.home)? {
            println!("Purge cancelled.");
            return Ok(());
        }
    }
    let context = SinkContext::new(
        xdg.clone(),
        Whitelist::load(&xdg).map_err(|_| io::Error::other("could not load whitelist"))?,
    );
    let oplog = OpLog::new()?;
    let mode = if dry_run {
        DeleteMode::DryRun
    } else {
        DeleteMode::Trash
    };
    let report = apply_selected(reviewed, &selected, mode, &context, &oplog)?;
    println!("{}", report.summary(dry_run));
    for path in &report.paths {
        println!("  {}", visible_path(path, &xdg.home));
    }
    if report.skipped > 0 || report.failed > 0 {
        println!("Skipped: {}; failed: {}", report.skipped, report.failed);
    }
    for (path, reason) in &report.skipped_paths {
        println!("Skipped {}: {reason}", visible_path(path, &xdg.home));
    }
    for (path, reason) in &report.failed_paths {
        println!("Failed {}: {reason}", visible_path(path, &xdg.home));
    }
    Ok(())
}

fn select_interactively(candidates: &[ReviewedCandidate]) -> io::Result<Vec<PathBuf>> {
    let mut selected = initial_selection(candidates);
    let mut cursor = 0_usize;
    let mut terminal = ratatui::try_init()?;
    let result = (|| -> io::Result<Option<Vec<PathBuf>>> {
        loop {
            let area = terminal.size()?;
            let can_confirm = area.width >= 40 && area.height >= 12;
            terminal.draw(|frame| {
                render_selection(frame, candidates, &selected, cursor, can_confirm)
            })?;
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Up | KeyCode::Char('k') => cursor = cursor.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        cursor = (cursor + 1).min(candidates.len().saturating_sub(1))
                    }
                    KeyCode::PageUp => cursor = cursor.saturating_sub(10),
                    KeyCode::PageDown => {
                        cursor = (cursor + 10).min(candidates.len().saturating_sub(1))
                    }
                    KeyCode::Char(' ')
                        if !candidates[cursor].candidate.protected
                            && candidates[cursor].candidate.activity != Activity::Uncertain =>
                    {
                        selected[cursor] = !selected[cursor]
                    }
                    KeyCode::Char('a') => {
                        let all = candidates
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| {
                                !item.candidate.protected
                                    && item.candidate.activity != Activity::Uncertain
                            })
                            .all(|(index, _)| selected[index]);
                        for (index, item) in candidates.iter().enumerate() {
                            if !item.candidate.protected
                                && item.candidate.activity != Activity::Uncertain
                            {
                                selected[index] = !all;
                            }
                        }
                    }
                    KeyCode::Enter if can_confirm => {
                        return Ok(Some(
                            candidates
                                .iter()
                                .enumerate()
                                .filter(|(index, _)| selected[*index])
                                .map(|(_, item)| item.candidate.path.clone())
                                .collect(),
                        ));
                    }
                    KeyCode::Enter => {}
                    _ => {}
                }
            }
        }
    })();
    ratatui::try_restore()?;
    Ok(result?.unwrap_or_default())
}

fn initial_selection(candidates: &[ReviewedCandidate]) -> Vec<bool> {
    candidates
        .iter()
        .map(|item| {
            item.candidate.selected_by_default
                && !item.candidate.protected
                && item.candidate.activity != Activity::Uncertain
        })
        .collect()
}

fn render_selection(
    frame: &mut ratatui::Frame<'_>,
    candidates: &[ReviewedCandidate],
    selected: &[bool],
    cursor: usize,
    can_confirm: bool,
) {
    let layout = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(frame.area());
    let rows = candidates
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let row = artifact_row(&item.candidate);
            let mark = if selected[index] { "[x]" } else { "[ ]" };
            let line = format!(
                "{mark} {} · {} · {} · {}",
                visible_text(&row.kind),
                visible_path(&row.path, Path::new("")),
                row.compact_reason,
                row.allocated,
            );
            let style = if index == cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(cursor));
    frame.render_stateful_widget(
        List::new(rows)
            .highlight_style(Style::default().bg(Color::DarkGray))
            .block(
                Block::default()
                    .title("Select Artifacts to Purge")
                    .borders(Borders::ALL),
            ),
        layout[0],
        &mut state,
    );
    let footer = if can_confirm {
        "↑/↓ move · Space toggle · a all · Enter review · q cancel"
    } else {
        "Resize terminal: minimum 40×12"
    };
    frame.render_widget(Paragraph::new(footer), layout[1]);
}

#[derive(Clone, Debug)]
struct ArtifactRow {
    path: PathBuf,
    kind: String,
    allocated: String,
    compact_reason: &'static str,
    selection_reason: String,
}

fn artifact_row(candidate: &Candidate) -> ArtifactRow {
    let kind = candidate
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_owned());
    let selection_reason = if let Some(reason) = &candidate.unavailable_reason {
        format!(
            "not selected by default (unavailable: {})",
            visible_text(reason)
        )
    } else if candidate.authored_content {
        "not selected by default (authored content)".to_owned()
    } else if candidate.activity == Activity::Uncertain {
        "not selected by default (uncertain activity)".to_owned()
    } else if candidate.protected {
        "not selected by default (protected)".to_owned()
    } else if candidate.activity == Activity::Recent {
        "not selected by default (recent activity)".to_owned()
    } else if candidate.selected_by_default {
        "selected by default (old activity)".to_owned()
    } else {
        "not selected by default (not eligible)".to_owned()
    };
    let compact_reason = if candidate.unavailable_reason.is_some() {
        "unavailable"
    } else if candidate.authored_content {
        "authored content"
    } else if candidate.activity == Activity::Uncertain {
        "uncertain activity"
    } else if candidate.protected {
        "protected"
    } else if candidate.activity == Activity::Recent {
        "recent"
    } else if candidate.selected_by_default {
        "selected by default"
    } else {
        "not selected"
    };
    let allocated = match &candidate.size {
        SizeEstimate::Known(bytes) => format!("{} allocated", bytes_si(*bytes)),
        SizeEstimate::Unknown(reason) => {
            format!("Unknown allocated size ({})", visible_text(reason))
        }
    };
    ArtifactRow {
        path: candidate.path.clone(),
        kind,
        allocated,
        compact_reason,
        selection_reason,
    }
}

fn print_preview(candidates: &[ReviewedCandidate], selected: &[PathBuf], home: &Path) {
    let mut projects = BTreeMap::<PathBuf, Vec<(bool, ArtifactRow)>>::new();
    let mut selected_allocated = 0_u64;
    let mut unknown_sizes = 0_usize;
    let mut overflowed = false;
    for item in candidates {
        let row = artifact_row(&item.candidate);
        let is_selected = selected.contains(&row.path);
        if is_selected {
            match &item.candidate.size {
                SizeEstimate::Known(bytes) => match selected_allocated.checked_add(*bytes) {
                    Some(total) => selected_allocated = total,
                    None => overflowed = true,
                },
                SizeEstimate::Unknown(_) => unknown_sizes = unknown_sizes.saturating_add(1),
            }
        }
        let project_root = purge::find_project_root_for_artifact(&row.path, home)
            .or_else(|| row.path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| row.path.clone());
        projects
            .entry(project_root)
            .or_default()
            .push((is_selected, row));
    }

    println!("Purge preview:");
    for (project_root, rows) in projects {
        println!("Project: {}", visible_path(&project_root, home));
        for (is_selected, row) in rows {
            let mark = if is_selected { "x" } else { " " };
            println!(
                "  [{mark}] {} ({}) | {} | {}",
                visible_text(&row.kind),
                visible_path(&row.path, home),
                row.allocated,
                row.selection_reason
            );
        }
    }
    if overflowed {
        println!(
            "Selected total: {} artifact(s); allocated total Unknown (byte total overflowed).",
            selected.len()
        );
    } else if unknown_sizes == 0 {
        println!(
            "Selected total: {} artifact(s), {} allocated.",
            selected.len(),
            bytes_si(selected_allocated)
        );
    } else {
        println!(
            "Selected total: {} artifact(s), {} allocated known; {} artifact(s) have Unknown size.",
            selected.len(),
            bytes_si(selected_allocated),
            unknown_sizes
        );
    }
    println!("Trash does not free space until emptied.");
}

fn confirm(paths: &[PathBuf], home: &Path) -> io::Result<bool> {
    println!("Selected paths:");
    for path in paths {
        println!("  {}", visible_path(path, home));
    }
    print!("Remove {} artifacts to Trash? [y/N] ", paths.len());
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes"))
}

fn visible_path(path: &Path, home: &Path) -> String {
    let display = path.display().to_string();
    let shown = if !home.as_os_str().is_empty() && path.starts_with(home) {
        match path.strip_prefix(home) {
            Ok(relative) if relative.as_os_str().is_empty() => "~".to_owned(),
            Ok(relative) => format!("~/{}", relative.display()),
            Err(_) => display,
        }
    } else {
        display
    };
    visible_text(&shown)
}

fn visible_text(text: &str) -> String {
    text.chars()
        .flat_map(char::escape_default)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn test_backend_shows_recent_unselected_and_named_paths() {
        let temporary = tempfile::tempdir().expect("fixture");
        let artifact = temporary.path().join("work/app/node_modules");
        std::fs::create_dir_all(&artifact).expect("artifact");
        let candidate = ReviewedCandidate::capture(Candidate {
            path: artifact,
            activity: Activity::Recent,
            protected: false,
            size: SizeEstimate::Known(2048),
            cachedir_tag: false,
            selected_by_default: false,
            authored_content: false,
            unavailable_reason: None,
        })
        .expect("capture candidate");
        let mut terminal = Terminal::new(TestBackend::new(100, 8)).expect("test terminal");
        terminal
            .draw(|frame| render_selection(frame, &[candidate], &[false], 0, true))
            .expect("render");
        let output = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(output.contains("Select Artifacts to Purge"));
        assert!(output.contains("[ ] "));
        assert!(output.contains("node_modules"));
        assert!(output.contains("recent"));
        assert!(output.contains("2.0 kB"));
        assert!(output.contains("Space toggle"));
    }

    #[test]
    fn displayed_paths_escape_control_characters() {
        let path = PathBuf::from("/tmp/project/line\nbreak");
        assert_eq!(
            visible_path(&path, Path::new("")),
            "/tmp/project/line\\nbreak"
        );
    }

    #[test]
    fn recent_rows_start_unselected_and_long_lists_scroll_to_focus() {
        let temporary = tempfile::tempdir().expect("fixture");
        let mut candidates = Vec::new();
        for index in 0..80 {
            let artifact = temporary
                .path()
                .join(format!("work/app-{index}/node_modules"));
            std::fs::create_dir_all(&artifact).expect("artifact");
            candidates.push(
                ReviewedCandidate::capture(Candidate {
                    path: artifact,
                    activity: if index == 0 {
                        Activity::Recent
                    } else {
                        Activity::Old
                    },
                    protected: false,
                    size: SizeEstimate::Known(1024),
                    cachedir_tag: false,
                    selected_by_default: index != 0,
                    authored_content: false,
                    unavailable_reason: None,
                })
                .expect("capture candidate"),
            );
        }
        let selected = initial_selection(&candidates);
        assert!(!selected[0]);
        assert!(selected[1]);

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("test terminal");
        terminal
            .draw(|frame| render_selection(frame, &candidates, &selected, 29, true))
            .expect("render focused row");
        let output = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(output.contains("app-29"));
        assert!(!output.contains("app-0"));
    }

    #[test]
    fn test_backend_disables_confirmation_when_terminal_is_too_small() {
        let temporary = tempfile::tempdir().expect("fixture");
        let artifact = temporary.path().join("node_modules");
        std::fs::create_dir_all(&artifact).expect("artifact");
        let candidate = ReviewedCandidate::capture(Candidate {
            path: artifact,
            activity: Activity::Old,
            protected: false,
            size: SizeEstimate::Known(1),
            cachedir_tag: false,
            selected_by_default: true,
            authored_content: false,
            unavailable_reason: None,
        })
        .expect("capture candidate");
        let mut terminal = Terminal::new(TestBackend::new(25, 10)).expect("test terminal");
        terminal
            .draw(|frame| render_selection(frame, &[candidate], &[true], 0, false))
            .expect("render small terminal");
        let output = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(output.contains("Resize terminal"));
    }
}

#[cfg(test)]
mod added_display_regression {
    use super::*;

    #[test]
    fn displayed_home_child_has_a_separator() {
        assert_eq!(
            visible_path(Path::new("/home/test/Workspace"), Path::new("/home/test")),
            "~/Workspace"
        );
    }

    #[test]
    fn artifact_rows_explain_uncertain_activity_and_unknown_allocated_size() {
        let candidate = Candidate {
            path: PathBuf::from("/tmp/project/target"),
            activity: Activity::Uncertain,
            protected: true,
            size: SizeEstimate::Unknown("size calculation exceeded its time budget".to_owned()),
            cachedir_tag: false,
            selected_by_default: false,
            authored_content: false,
            unavailable_reason: None,
        };
        let row = artifact_row(&candidate);
        assert_eq!(row.kind, "target");
        assert!(row.allocated.contains("Unknown allocated size"));
        assert!(row.allocated.contains("time budget"));
        assert!(row.selection_reason.contains("uncertain activity"));
    }
}
