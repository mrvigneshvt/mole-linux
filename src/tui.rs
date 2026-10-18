#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::scan::{EntryKind, Node, NodeStatus};

    use super::{
        App, FilesystemStats, FilesystemType, Navigator, SizeMode, escape_name,
        filesystem_type_from_magic, format_size, percentage, render,
    };

    fn node(name: &str, kind: EntryKind, children: Vec<Node>) -> Node {
        Node {
            name: OsString::from(name),
            kind,
            cleanable: false,
            apparent: 4096,
            allocated: 2048,
            children,
            status: NodeStatus::Complete,
        }
    }

    fn sample_tree() -> Node {
        node(
            "root",
            EntryKind::Directory,
            vec![
                node(
                    "first",
                    EntryKind::Directory,
                    vec![node("nested", EntryKind::File, vec![])],
                ),
                node("second", EntryKind::Directory, vec![]),
            ],
        )
    }

    #[test]
    fn navigation_moves_enters_and_returns_to_parent_selection() {
        let mut navigation = Navigator::new(sample_tree());
        assert_eq!(
            navigation.selected_node().map(|node| node.name.as_os_str()),
            Some(OsStr::new("first"))
        );

        navigation.move_selection(1);
        assert_eq!(
            navigation.selected_node().map(|node| node.name.as_os_str()),
            Some(OsStr::new("second"))
        );
        navigation.move_selection(-1);
        assert!(navigation.enter());
        assert_eq!(navigation.current_node().name, "first");
        assert!(navigation.up());
        assert_eq!(
            navigation.selected_node().map(|node| node.name.as_os_str()),
            Some(OsStr::new("first"))
        );
    }

    #[test]
    fn size_formatting_uses_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KiB");
    }

    #[test]
    fn names_escape_terminal_control_characters() {
        assert_eq!(
            escape_name(OsStr::new("line\nbreak\t\\")),
            "line\\nbreak\\t\\\\"
        );
    }

    #[test]
    fn percentages_are_suppressed_for_partial_parent() {
        assert_eq!(percentage(&NodeStatus::Partial, 25, 100), None);
        assert_eq!(percentage(&NodeStatus::Complete, 25, 100), Some(25));
        assert_eq!(percentage(&NodeStatus::Complete, 25, 0), None);
    }

    #[test]
    fn test_backend_renders_header_and_child_list() {
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        let mut root = sample_tree();
        root.children[0].cleanable = true;
        let app = App::new(
            PathBuf::from("/tmp/example"),
            root,
            Some(FilesystemStats {
                free_bytes: 8 * 1024 * 1024,
                total_bytes: 16 * 1024 * 1024,
                filesystem_type: None,
            }),
            false,
        );

        terminal
            .draw(|frame| render(frame, &app))
            .expect("render test frame");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("/tmp/example"));
        assert!(rendered.contains("Allocated"));
        assert!(rendered.contains("first"));
        assert!(rendered.contains("d* first"));
        assert!(rendered.contains("[##########] 100%"));
    }

    #[test]
    fn filesystem_magic_maps_btrfs_and_other_filesystems() {
        assert_eq!(
            filesystem_type_from_magic(0x9123_683e),
            FilesystemType::Btrfs
        );
        assert_eq!(filesystem_type_from_magic(0xef53), FilesystemType::Other);
    }

    #[test]
    fn btrfs_header_warns_shared_and_snapshot_space_is_not_counted() {
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        let app = App::new(
            PathBuf::from("/tmp/example"),
            sample_tree(),
            Some(FilesystemStats {
                free_bytes: 8 * 1024 * 1024,
                total_bytes: 16 * 1024 * 1024,
                filesystem_type: Some(FilesystemType::Btrfs),
            }),
            false,
        );

        terminal
            .draw(|frame| render(frame, &app))
            .expect("render btrfs warning");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("snapshot/shared extents not counted"));
    }

    #[test]
    fn test_backend_header_uses_current_directory_after_descend() {
        let mut root = sample_tree();
        root.apparent = 9_000;
        root.allocated = 5_000;
        root.children[0].apparent = 1_234;
        root.children[0].allocated = 321;
        root.children[0].status = NodeStatus::Partial;
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        let mut app = App::new(PathBuf::from("/tmp/example"), root, None, false);
        assert!(app.navigator.enter());

        terminal
            .draw(|frame| render(frame, &app))
            .expect("render descended directory");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("/tmp/example/first"));
        assert!(rendered.contains("Allocated: ~321 B | Apparent: ~1.2 KiB"));
        assert!(rendered.contains("| Partial"));
        assert!(!rendered.contains("Allocated: 4.8 KiB | Apparent: 8.7 KiB"));
    }

    #[test]
    fn test_backend_marks_excluded_mount_nodes_as_zero_size() {
        let mut root = sample_tree();
        root.children.push(Node {
            name: OsString::from("mounted"),
            kind: EntryKind::Directory,
            cleanable: false,
            apparent: 0,
            allocated: 0,
            children: Vec::new(),
            status: NodeStatus::Excluded("mount boundary".to_owned()),
        });
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        let app = App::new(PathBuf::from("/tmp"), root, None, false);

        terminal
            .draw(|frame| render(frame, &app))
            .expect("render excluded node");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("× mounted 0 B"));
        assert!(rendered.contains("excluded"));
    }

    #[test]
    fn size_mode_toggles_between_allocated_and_apparent() {
        let mut app = App::new(PathBuf::from("/tmp"), sample_tree(), None, false);
        assert_eq!(app.size_mode, SizeMode::Allocated);
        app.toggle_size_mode();
        assert_eq!(app.size_mode, SizeMode::Apparent);
    }

    #[test]
    fn test_backend_renders_large_files_and_filter_prompt() {
        let mut app = App::new(PathBuf::from("/tmp"), sample_tree(), None, false);
        app.show_large_files = true;
        app.large_files = super::LargeFilesView::from_entries(vec![
            (PathBuf::from("/tmp/large.bin"), Some(8 * 1024)),
            (PathBuf::from("/tmp/unknown.bin"), None),
        ]);
        app.large_files.begin_filter();
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        terminal
            .draw(|frame| render(frame, &app))
            .expect("render large files");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Top 20 large files"));
        assert!(rendered.contains("large.bin"));
        assert!(rendered.contains("unknown.bin"));
        assert!(rendered.contains("Filter:"));
        app.large_files.move_selection(1);
        app.large_files.toggle_selected();
        assert_eq!(app.large_files.selected_count(), 0);
    }
}
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::ffi::OsStr;
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, atomic::AtomicBool as GlobalAtomicBool};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};
use rustix::fs::{statfs, statvfs};

use crate::scan;
use crate::scan::{EntryKind, Node, NodeStatus, ScanEvent, ScanReport};
use crate::status;

static TERMINAL_ACTIVE: GlobalAtomicBool = GlobalAtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeMode {
    Allocated,
    Apparent,
}

const LARGE_FILE_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargeFileEntry {
    pub path: PathBuf,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct LargeFiles {
    entries: Vec<LargeFileEntry>,
}

impl LargeFiles {
    // Ported from tw93/Mole cmd/analyze/update.go large-file navigation (GPL-3.0).
    pub fn from_tree(root: &Node, base: &Path) -> Self {
        let mut heap = BinaryHeap::<Reverse<(u64, PathBuf)>>::new();
        collect_large_files(root, base, &mut heap);
        let mut entries = heap
            .into_iter()
            .map(|Reverse((size, path))| LargeFileEntry {
                path,
                size: Some(size),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .size
                .cmp(&left.size)
                .then_with(|| left.path.cmp(&right.path))
        });
        Self { entries }
    }

    pub fn from_entries(entries: Vec<(PathBuf, Option<u64>)>) -> Self {
        let mut entries = entries
            .into_iter()
            .map(|(path, size)| LargeFileEntry { path, size })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .size
                .cmp(&left.size)
                .then_with(|| left.path.cmp(&right.path))
        });
        entries.truncate(LARGE_FILE_LIMIT);
        Self { entries }
    }

    pub fn entries(&self) -> &[LargeFileEntry] {
        &self.entries
    }
}

fn collect_large_files(node: &Node, path: &Path, heap: &mut BinaryHeap<Reverse<(u64, PathBuf)>>) {
    let mut pending = vec![(node, path.to_owned())];
    while let Some((parent, parent_path)) = pending.pop() {
        for child in &parent.children {
            let child_path = parent_path.join(&child.name);
            if child.kind == EntryKind::File
                && !matches!(
                    child.status,
                    NodeStatus::Unavailable(_) | NodeStatus::Partial
                )
            {
                let candidate = (child.apparent, child_path.clone());
                if heap.len() < LARGE_FILE_LIMIT {
                    heap.push(Reverse(candidate));
                } else if heap.peek().is_some_and(|smallest| candidate > smallest.0) {
                    heap.pop();
                    heap.push(Reverse(candidate));
                }
            }
            pending.push((child, child_path));
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LargeFilesView {
    files: LargeFiles,
    filter: String,
    filtering: bool,
    selected_index: usize,
    selected: HashSet<PathBuf>,
}

impl LargeFilesView {
    pub fn new(files: LargeFiles) -> Self {
        Self {
            files,
            ..Self::default()
        }
    }
    pub fn from_entries(entries: Vec<(PathBuf, Option<u64>)>) -> Self {
        Self::new(LargeFiles::from_entries(entries))
    }
    pub fn visible_entries(&self) -> Vec<&LargeFileEntry> {
        self.files
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&self.filter.to_lowercase())
            })
            .collect()
    }
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }
    pub fn toggle_selected(&mut self) {
        if let Some(entry) = self.visible_entries().get(self.selected_index).copied()
            && entry.size.is_some()
        {
            let path = entry.path.clone();
            if !self.selected.remove(&path) {
                self.selected.insert(path);
            }
        }
    }
    pub fn begin_filter(&mut self) {
        self.filtering = true;
    }
    pub fn push_filter(&mut self, text: &str) {
        self.filter.push_str(text);
        self.selected_index = 0;
    }
    pub fn apply_filter(&mut self) {
        self.filtering = false;
        self.selected_index = 0;
    }
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.filtering = false;
        self.selected_index = 0;
    }
    pub fn move_selection(&mut self, delta: isize) {
        let count = self.visible_entries().len();
        if count == 0 {
            self.selected_index = 0;
            return;
        }
        self.selected_index = if delta.is_negative() {
            self.selected_index.saturating_sub(delta.unsigned_abs())
        } else {
            self.selected_index
                .saturating_add(usize::try_from(delta).unwrap_or(usize::MAX))
                .min(count - 1)
        };
    }
    pub fn restore(&mut self, previous: Self) {
        *self = previous;
    }
    fn is_filtering(&self) -> bool {
        self.filtering
    }
    fn filter(&self) -> &str {
        &self.filter
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesystemStats {
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub filesystem_type: Option<FilesystemType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemType {
    Btrfs,
    Other,
}

fn filesystem_type_from_magic(magic: u64) -> FilesystemType {
    if magic == 0x9123_683e {
        FilesystemType::Btrfs
    } else {
        FilesystemType::Other
    }
}

impl FilesystemStats {
    fn for_path(path: &Path) -> Option<Self> {
        let stats = statvfs(path).ok()?;
        let filesystem_type = statfs(path)
            .ok()
            .and_then(|stats| u64::try_from(stats.f_type).ok())
            .map(filesystem_type_from_magic);
        Some(Self {
            free_bytes: stats.f_bavail.checked_mul(stats.f_frsize)?,
            total_bytes: stats.f_blocks.checked_mul(stats.f_frsize)?,
            filesystem_type,
        })
    }
}

pub struct Navigator {
    root: Node,
    levels: Vec<usize>,
}

impl Navigator {
    pub fn new(root: Node) -> Self {
        Self {
            root,
            levels: vec![0],
        }
    }

    pub fn current_node(&self) -> &Node {
        let mut node = &self.root;
        for index in self.levels.iter().take(self.levels.len().saturating_sub(1)) {
            let Some(child) = node.children.get(*index) else {
                return &self.root;
            };
            node = child;
        }
        node
    }

    pub fn selected_index(&self) -> usize {
        match self.levels.last() {
            Some(index) => *index,
            None => 0,
        }
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.current_node().children.get(self.selected_index())
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.current_node().children.len();
        if count == 0 {
            if let Some(selected) = self.levels.last_mut() {
                *selected = 0;
            }
            return;
        }
        let maximum = count.saturating_sub(1);
        let current = self.selected_index();
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            match usize::try_from(delta) {
                Ok(step) => current.saturating_add(step).min(maximum),
                Err(_) => maximum,
            }
        };
        if let Some(selected) = self.levels.last_mut() {
            *selected = next;
        }
    }

    pub fn enter(&mut self) -> bool {
        let Some(node) = self.selected_node() else {
            return false;
        };
        if node.kind != EntryKind::Directory
            || matches!(
                &node.status,
                NodeStatus::Unavailable(_) | NodeStatus::Excluded(_)
            )
        {
            return false;
        }
        self.levels.push(0);
        true
    }

    pub fn up(&mut self) -> bool {
        if self.levels.len() <= 1 {
            return false;
        }
        self.levels.pop();
        true
    }

    fn current_path(&self, base: &Path) -> PathBuf {
        let mut path = base.to_owned();
        let mut node = &self.root;
        for index in self.levels.iter().take(self.levels.len().saturating_sub(1)) {
            let Some(child) = node.children.get(*index) else {
                break;
            };
            path.push(&child.name);
            node = child;
        }
        path
    }
}

pub struct App {
    path: PathBuf,
    pub navigator: Navigator,
    pub filesystem: Option<FilesystemStats>,
    pub scanning: bool,
    pub size_mode: SizeMode,
    spinner_index: usize,
    scan_failed: bool,
    large_files: LargeFilesView,
    show_large_files: bool,
}

impl App {
    pub fn new(
        path: PathBuf,
        root: Node,
        filesystem: Option<FilesystemStats>,
        scanning: bool,
    ) -> Self {
        Self {
            path,
            navigator: Navigator::new(root),
            filesystem,
            scanning,
            size_mode: SizeMode::Allocated,
            spinner_index: 0,
            scan_failed: false,
            large_files: LargeFilesView::default(),
            show_large_files: false,
        }
    }

    pub fn toggle_size_mode(&mut self) {
        self.size_mode = match self.size_mode {
            SizeMode::Allocated => SizeMode::Apparent,
            SizeMode::Apparent => SizeMode::Allocated,
        };
    }

    fn accept_report(&mut self, report: ScanReport) {
        self.large_files = LargeFilesView::new(LargeFiles::from_tree(&report.root, &self.path));
        self.navigator = Navigator::new(report.root);
        self.scanning = false;
    }

    fn accept_event(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Initial(root) => self.navigator = Navigator::new(root),
            ScanEvent::ChildUpdated(updated) => {
                if let Some(child) = self
                    .navigator
                    .root
                    .children
                    .iter_mut()
                    .find(|child| child.name == updated.name)
                {
                    *child = updated;
                }
            }
            ScanEvent::Complete(report) => self.accept_report(report),
        }
    }
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut unit_index = 1_usize;
    let mut divisor = 1024_u128;
    while unit_index + 1 < UNITS.len() && u128::from(bytes) >= divisor * 1024 {
        divisor *= 1024;
        unit_index += 1;
    }
    let tenths = u128::from(bytes).saturating_mul(10) / divisor;
    format!("{}.{:01} {}", tenths / 10, tenths % 10, UNITS[unit_index])
}

pub fn escape_name(name: &OsStr) -> String {
    let mut escaped = String::new();
    for character in name.to_string_lossy().chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\\' => escaped.push_str("\\\\"),
            control if control.is_control() => {
                escaped.push_str(&format!("\\u{{{:x}}}", u32::from(control)));
            }
            visible => escaped.push(visible),
        }
    }
    escaped
}

pub fn percentage(parent_status: &NodeStatus, child: u64, parent: u64) -> Option<u8> {
    if parent == 0
        || matches!(
            parent_status,
            NodeStatus::Partial | NodeStatus::Unavailable(_)
        )
    {
        return None;
    }
    let percent = u128::from(child).saturating_mul(100) / u128::from(parent);
    u8::try_from(percent.min(100)).ok()
}

fn size_for_node(node: &Node, mode: SizeMode) -> String {
    match &node.status {
        NodeStatus::Unavailable(_) => "?".to_owned(),
        NodeStatus::Excluded(_) => "0 B".to_owned(),
        NodeStatus::Partial => format!("~{}", format_size(size_for_mode(node, mode))),
        NodeStatus::Complete => format_size(size_for_mode(node, mode)),
    }
}

fn size_for_mode(node: &Node, mode: SizeMode) -> u64 {
    match mode {
        SizeMode::Allocated => node.allocated,
        SizeMode::Apparent => node.apparent,
    }
}

fn percent_bar(percent: Option<u8>) -> String {
    let Some(percent) = percent else {
        return String::new();
    };
    let filled = usize::from(percent.min(100)) * 10 / 100;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(10 - filled))
}

fn kind_marker(node: &Node) -> &'static str {
    if matches!(&node.status, NodeStatus::Excluded(_)) {
        return "×";
    }
    match node.kind {
        EntryKind::Directory if node.cleanable => "d*",
        EntryKind::Directory => "d",
        EntryKind::File => "f",
        EntryKind::Symlink => "l",
        EntryKind::Other => "?",
    }
}

fn status_label(status: &NodeStatus) -> String {
    match status {
        NodeStatus::Complete => "Complete".to_owned(),
        NodeStatus::Partial => "Partial".to_owned(),
        NodeStatus::Unavailable(reason) => {
            format!("Unavailable: {}", escape_name(OsStr::new(reason)))
        }
        NodeStatus::Excluded(reason) => format!("Excluded: {}", escape_name(OsStr::new(reason))),
    }
}

fn render(frame: &mut Frame<'_>, app: &App) {
    let btrfs = app
        .filesystem
        .is_some_and(|stats| stats.filesystem_type == Some(FilesystemType::Btrfs));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if btrfs { 4 } else { 3 }),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let current = app.navigator.current_node();
    let fs_line = match app.filesystem {
        Some(stats) => format!(
            "Filesystem free: {} / {}",
            format_size(stats.free_bytes),
            format_size(stats.total_bytes)
        ),
        None => "Filesystem free: ? / ?".to_owned(),
    };
    let scan_label = if app.scanning {
        let spinner = ["|", "/", "-", "\\"][app.spinner_index % 4];
        format!("Scanning {spinner}")
    } else if app.scan_failed {
        "Scan failed".to_owned()
    } else {
        status_label(&current.status)
    };
    let mut header_lines = vec![
        Line::from(escape_name(
            app.navigator.current_path(&app.path).as_os_str(),
        )),
        Line::from(format!(
            "Allocated: {} | Apparent: {}",
            size_for_node(current, SizeMode::Allocated),
            size_for_node(current, SizeMode::Apparent)
        )),
        Line::from(format!("{fs_line} | {scan_label}")),
    ];
    if btrfs {
        header_lines.push(Line::from("snapshot/shared extents not counted"));
    }
    let header = Paragraph::new(header_lines);
    frame.render_widget(header, chunks[0]);

    if app.show_large_files {
        let visible = app.large_files.visible_entries();
        let items = visible
            .iter()
            .map(|entry| {
                let selected = if app.large_files.selected.contains(&entry.path) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let size = entry.size.map_or_else(|| "?".to_owned(), format_size);
                ListItem::new(format!(
                    "{selected} {size} {}",
                    escape_name(entry.path.as_os_str())
                ))
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Top 20 large files"),
            )
            .highlight_symbol("> ")
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        let mut state = ListState::default();
        if !visible.is_empty() {
            state.select(Some(
                app.large_files
                    .selected_index
                    .min(visible.len().saturating_sub(1)),
            ));
        }
        frame.render_stateful_widget(list, chunks[1], &mut state);
        let footer = if app.large_files.is_filtering() {
            format!(
                "Filter: {}_  Enter apply  Esc clear",
                app.large_files.filter()
            )
        } else {
            format!(
                "↑/↓ move  / filter  Space select  r refresh  Esc back{}",
                if app.large_files.selected_count() > 0 {
                    format!("  {} selected", app.large_files.selected_count())
                } else {
                    String::new()
                }
            )
        };
        frame.render_widget(Paragraph::new(footer), chunks[2]);
        return;
    }

    let denominator = size_for_mode(current, app.size_mode);
    let items = current
        .children
        .iter()
        .map(|node| {
            let percent = if matches!(
                &node.status,
                NodeStatus::Partial | NodeStatus::Unavailable(_)
            ) {
                None
            } else {
                percentage(
                    &current.status,
                    size_for_mode(node, app.size_mode),
                    denominator,
                )
            };
            let bar = percent_bar(percent);
            let percent = percent.map_or_else(String::new, |value| format!(" {value}%"));
            ListItem::new(Line::from(format!(
                "{} {} {} {}{}{}",
                kind_marker(node),
                escape_name(&node.name),
                size_for_node(node, app.size_mode),
                bar,
                percent,
                if matches!(&node.status, NodeStatus::Excluded(_)) {
                    " excluded"
                } else {
                    ""
                }
            )))
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Entries (d* regenerable hint)"),
        )
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    if !current.children.is_empty() {
        state.select(Some(
            app.navigator
                .selected_index()
                .min(current.children.len().saturating_sub(1)),
        ));
    }
    frame.render_stateful_widget(list, chunks[1], &mut state);
    frame.render_widget(
        Paragraph::new("↑/↓ or j/k move  Enter/l descend  h/Backspace up  t large files  r refresh  a size  q quit"),
        chunks[2],
    );
}

pub fn run(
    path: &Path,
    mut reports: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
) -> io::Result<()> {
    install_panic_restore_hook();
    let mut terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(
        path.to_owned(),
        Node {
            name: path.as_os_str().to_owned(),
            kind: EntryKind::Directory,
            cleanable: false,
            apparent: 0,
            allocated: 0,
            children: Vec::new(),
            status: NodeStatus::Partial,
        },
        FilesystemStats::for_path(path),
        true,
    );
    let mut active_cancel = Arc::clone(&cancel);
    let mut refresh_worker: Option<std::thread::JoinHandle<()>> = None;

    loop {
        match reports.try_recv() {
            Ok(event) => app.accept_event(event),
            Err(TryRecvError::Disconnected) if app.scanning => {
                app.scanning = false;
                app.scan_failed = true;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
        app.spinner_index = app.spinner_index.saturating_add(1);
        terminal.draw(|frame| render(frame, &app))?;
        if event::poll(Duration::from_millis(100))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Esc
                    if app.show_large_files
                        && (app.large_files.is_filtering()
                            || !app.large_files.filter().is_empty()) =>
                {
                    app.large_files.clear_filter()
                }
                KeyCode::Esc if app.show_large_files => app.show_large_files = false,
                KeyCode::Esc => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Up | KeyCode::Char('k')
                    if app.show_large_files && !app.large_files.is_filtering() =>
                {
                    app.large_files.move_selection(-1)
                }
                KeyCode::Down | KeyCode::Char('j')
                    if app.show_large_files && !app.large_files.is_filtering() =>
                {
                    app.large_files.move_selection(1)
                }
                KeyCode::Char(' ') if app.show_large_files && !app.large_files.is_filtering() => {
                    app.large_files.toggle_selected()
                }
                KeyCode::Enter if app.show_large_files && app.large_files.is_filtering() => {
                    app.large_files.apply_filter()
                }
                KeyCode::Char(character)
                    if app.show_large_files && app.large_files.is_filtering() =>
                {
                    app.large_files.push_filter(&character.to_string())
                }
                KeyCode::Backspace if app.show_large_files && app.large_files.is_filtering() => {
                    app.large_files.filter.pop();
                }
                KeyCode::Char('h') | KeyCode::Backspace if app.show_large_files => {
                    app.show_large_files = false;
                    app.navigator.up();
                }
                KeyCode::Up | KeyCode::Char('k') => app.navigator.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => app.navigator.move_selection(1),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    app.navigator.enter();
                }
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                    app.navigator.up();
                }
                KeyCode::Char('a') => app.toggle_size_mode(),
                KeyCode::Char('t') if !app.scanning => app.show_large_files = !app.show_large_files,
                KeyCode::Char('/') if app.show_large_files => app.large_files.begin_filter(),
                KeyCode::Char('r') => {
                    active_cancel.store(true, Ordering::Relaxed);
                    if let Some(worker) = refresh_worker.take() {
                        worker
                            .join()
                            .map_err(|_| io::Error::other("refresh scanner worker panicked"))?;
                    }
                    let next_cancel = Arc::new(AtomicBool::new(false));
                    let worker_cancel = Arc::clone(&next_cancel);
                    let worker_path = path.to_owned();
                    let (sender, receiver) = std::sync::mpsc::channel();
                    refresh_worker = Some(
                        std::thread::Builder::new()
                            .name("mole-refresh".to_owned())
                            .spawn(move || {
                                scan::scan_progressive(&worker_path, &worker_cancel, sender)
                            })?,
                    );
                    active_cancel = next_cancel;
                    reports = receiver;
                    app.scanning = true;
                    app.scan_failed = false;
                }
                _ => {}
            }
        }
    }
    active_cancel.store(true, Ordering::Relaxed);
    if let Some(worker) = refresh_worker {
        worker
            .join()
            .map_err(|_| io::Error::other("refresh scanner worker panicked"))?;
    }
    drop(terminal);
    terminal_guard.restore()
}

pub fn run_status() -> io::Result<()> {
    install_panic_restore_hook();
    let _terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut tracker = status::StatusTracker::default();
    loop {
        let mut snapshot = status::collect();
        tracker.update(&mut snapshot, std::time::Instant::now());
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);
            frame.render_widget(
                Paragraph::new(status::text_report(&snapshot)).block(
                    Block::default()
                        .title(" System Status ")
                        .borders(Borders::ALL),
                ),
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new(" q / Esc quit  •  refreshes every second "),
                chunks[1],
            );
        })?;

        let last_refresh = std::time::Instant::now();
        loop {
            let elapsed = last_refresh.elapsed();
            if elapsed >= Duration::from_secs(1) {
                break;
            }
            let remaining = Duration::from_secs(1).saturating_sub(elapsed);
            if event::poll(remaining.min(Duration::from_millis(100)))? {
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}

fn install_panic_restore_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            restore_terminal_after_panic();
            previous(panic);
        }));
    });
}

fn restore_terminal_after_panic() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    if let Err(error) = disable_raw_mode() {
        eprintln!("failed to disable raw terminal mode after panic: {error}");
    }
    if let Err(error) = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show) {
        eprintln!("failed to restore terminal screen after panic: {error}");
    }
}

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut guard = Self { active: true };
        TERMINAL_ACTIVE.store(true, Ordering::Relaxed);
        if let Err(error) = execute!(stdout(), EnterAlternateScreen, crossterm::cursor::Hide) {
            if let Err(restore_error) = guard.restore() {
                eprintln!("failed to restore terminal after setup error: {restore_error}");
            }
            return Err(error);
        }
        Ok(guard)
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        if !TERMINAL_ACTIVE.swap(false, Ordering::Relaxed) {
            return Ok(());
        }
        let raw_mode = disable_raw_mode();
        let screen = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        raw_mode.and(screen)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active
            && let Err(error) = self.restore()
        {
            eprintln!("failed to restore terminal: {error}");
        }
    }
}
