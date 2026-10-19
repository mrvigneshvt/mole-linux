#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::core::sink::DeleteError;
    use crate::scan::{EntryKind, Node, NodeStatus, ScanReport};

    use super::{
        App, FilesystemStats, FilesystemType, Navigator, SizeMode, escape_name,
        filesystem_type_from_magic, format_size, percentage, render,
    };

    fn node(name: &str, kind: EntryKind, children: Vec<Node>) -> Node {
        Node {
            name: OsString::from(name),
            target_path: None,
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
    fn overview_root_header_uses_overview_label() {
        let mut root = node("/", EntryKind::Directory, vec![]);
        root.children.push(Node {
            name: OsString::from("Home"),
            target_path: Some(PathBuf::from("/tmp/home")),
            kind: EntryKind::Directory,
            cleanable: false,
            apparent: 0,
            allocated: 0,
            children: Vec::new(),
            status: NodeStatus::Scanning,
        });
        let app = App::new(PathBuf::from("/"), root, None, true);
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("create test terminal");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("render overview");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Overview"));
        assert!(!rendered.contains("\n/\n"));
        assert!(rendered.contains("d Home ?"));
    }

    #[test]
    fn test_backend_marks_excluded_mount_nodes_as_zero_size() {
        let mut root = sample_tree();
        root.children.push(Node {
            name: OsString::from("mounted"),
            target_path: None,
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
        assert!(rendered.contains("Search:"));
        app.large_files.push_filter("zebra");
        terminal
            .draw(|frame| render(frame, &app))
            .expect("render active search query");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Search: zebra_"));
        app.large_files.move_selection(1);
        app.large_files.toggle_selected();
        assert_eq!(app.large_files.selected_count(), 0);
    }

    #[test]
    fn test_backend_renders_menu_and_search_results() {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        let mut app = super::MenuApp::default();
        terminal
            .draw(|frame| super::render_menu(frame, &app))
            .expect("render main menu");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Clean"));
        assert!(rendered.contains("Analyze"));
        assert!(rendered.contains("Q Quit"));

        app.query.push_str("sta");
        app.searching = true;
        terminal
            .draw(|frame| super::render_menu(frame, &app))
            .expect("render filtered main menu");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Commands — search: sta"));
        assert!(rendered.contains("Monitor system health"));
        assert!(!rendered.contains("Free up disk space"));

        app.clear_search();
        terminal
            .draw(|frame| super::render_menu(frame, &app))
            .expect("render menu after Esc clears search");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Commands"));
        assert!(!rendered.contains("search: sta"));
        assert!(!rendered.contains("Search: sta_"));
    }

    #[test]
    fn confirmed_delete_removes_large_file_rows_and_renders_result_and_failures() {
        let target = PathBuf::from("/tmp/example/selected.bin");
        let failed = PathBuf::from("/tmp/example/protected.bin");
        let mut root = node(
            "root",
            EntryKind::Directory,
            vec![
                node("selected.bin", EntryKind::File, vec![]),
                node("protected.bin", EntryKind::File, vec![]),
            ],
        );
        root.apparent = 3072;
        root.allocated = 3072;
        let mut app = App::new(PathBuf::from("/tmp/example"), root, None, false);
        app.large_files = super::LargeFilesView::from_entries(vec![
            (target.clone(), Some(2048)),
            (failed.clone(), Some(1024)),
        ]);
        let sizes = std::collections::HashMap::from([
            (target.clone(), Some(2048)),
            (failed.clone(), Some(1024)),
        ]);

        let refresh = app.apply_delete_results(
            vec![
                (target.clone(), Ok(target.clone())),
                (failed.clone(), Err(DeleteError::Whitelisted)),
            ],
            &sizes,
        );

        assert!(refresh, "a successful Trash should refresh scan totals");
        assert_eq!(app.large_files.files.entries().len(), 1);
        assert_eq!(app.large_files.files.entries()[0].path, failed);
        let report = app.delete_report.as_ref().expect("delete result");
        assert_eq!(report.moved, 1);
        assert_eq!(report.moved_bytes, Some(2048));
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].1, "protected by the whitelist");

        let mut refreshed_root = node(
            "root",
            EntryKind::Directory,
            vec![node("protected.bin", EntryKind::File, vec![])],
        );
        refreshed_root.apparent = 1024;
        refreshed_root.allocated = 1024;
        app.accept_report(ScanReport {
            root: refreshed_root,
        });
        assert_eq!(app.navigator.current_node().apparent, 1024);

        let mut terminal = Terminal::new(TestBackend::new(120, 20)).expect("test terminal");
        app.show_large_files = true;
        terminal
            .draw(|frame| render(frame, &app))
            .expect("render result");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Moved 1 item(s) to Trash (2.0 KiB apparent)."));
        assert!(rendered.contains("Space is not freed until Trash is emptied."));
        assert!(rendered.contains("/tmp/example/protected.bin"));
        assert!(rendered.contains("protected by the whitelist"));
        assert!(!rendered.contains("/tmp/example/selected.bin"));
    }

    #[test]
    fn main_list_selection_tracks_paths_and_is_disabled_while_scanning_or_at_overview() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let base = temp.path().join("home");
        std::fs::create_dir(&base).expect("home");
        std::fs::write(base.join("candidate.bin"), b"candidate").expect("candidate file");
        let root = node(
            "root",
            EntryKind::Directory,
            vec![node("candidate.bin", EntryKind::File, vec![])],
        );
        let mut app = App::new(base.clone(), root, None, false);

        app.toggle_current_selected();
        assert!(app.main_selected.contains_key(&base.join("candidate.bin")));
        let pending = app
            .prepare_main_delete()
            .expect("capture selected identity")
            .expect("selected path pending");
        assert_eq!(pending.entries[0].0, base.join("candidate.bin"));
        app.scanning = true;
        app.toggle_current_selected();
        assert!(app.main_selected.contains_key(&base.join("candidate.bin")));
        assert!(
            app.prepare_main_delete()
                .expect("scanning disables delete")
                .is_none()
        );

        let mut terminal = Terminal::new(TestBackend::new(100, 16)).expect("test terminal");
        terminal
            .draw(|frame| render(frame, &app))
            .expect("render scanning list");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("[x] f candidate.bin"));
        assert!(!rendered.contains("Space select"));

        let mut overview_root = node("/", EntryKind::Directory, vec![]);
        overview_root.children.push(Node {
            name: OsString::from("Home"),
            target_path: Some(PathBuf::from("/tmp/example")),
            kind: EntryKind::Directory,
            cleanable: false,
            apparent: 4096,
            allocated: 2048,
            children: Vec::new(),
            status: NodeStatus::Complete,
        });
        let mut overview = App::new(PathBuf::from("/"), overview_root, None, false);
        overview.toggle_current_selected();
        assert!(overview.main_selected.is_empty());
        assert!(
            overview
                .prepare_main_delete()
                .expect("overview disables delete")
                .is_none()
        );
    }
}
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::AtomicI32;
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
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use rustix::fs::{statfs, statvfs};

use crate::commands::analyze_delete::trash_confirmed;
use crate::core::oplog::OpLog;
use crate::core::paths::XdgPaths;
use crate::core::sink::{Identity, SinkContext};
use crate::core::whitelist::Whitelist;
use crate::scan;
use crate::scan::{EntryKind, Node, NodeStatus, ScanEvent, ScanReport};
use crate::status;

static TERMINAL_ACTIVE: GlobalAtomicBool = GlobalAtomicBool::new(false);
static MENU_SIGNAL: AtomicI32 = AtomicI32::new(0);

// Ported from tw93/Mole mole:interactive_main_menu and show_main_menu (GPL-3.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    Clean,
    Uninstall,
    Optimize,
    Analyze,
    Status,
}

const MENU_CHOICES: [(MenuChoice, &str, &str); 5] = [
    (MenuChoice::Clean, "Clean", "Free up disk space"),
    (MenuChoice::Uninstall, "Uninstall", "Remove apps completely"),
    (
        MenuChoice::Optimize,
        "Optimize",
        "Refresh caches and services",
    ),
    (MenuChoice::Analyze, "Analyze", "Explore disk usage"),
    (MenuChoice::Status, "Status", "Monitor system health"),
];

#[derive(Default)]
struct MenuApp {
    selected: usize,
    query: String,
    searching: bool,
}

impl MenuApp {
    fn clear_search(&mut self) {
        self.query.clear();
        self.searching = false;
        self.selected = 0;
    }

    fn visible(&self) -> Vec<usize> {
        MENU_CHOICES
            .iter()
            .enumerate()
            .filter_map(|(index, (_, name, description))| {
                let query = self.query.to_lowercase();
                (name.to_lowercase().contains(&query)
                    || description.to_lowercase().contains(&query))
                .then_some(index)
            })
            .collect()
    }

    fn move_selection(&mut self, delta: isize) {
        let visible = self.visible();
        if visible.is_empty() {
            self.selected = 0;
            return;
        }
        let current = visible
            .iter()
            .position(|index| *index == self.selected)
            .unwrap_or(0);
        let next = if delta < 0 {
            current.saturating_sub(1)
        } else {
            current
                .saturating_add(1)
                .min(visible.len().saturating_sub(1))
        };
        self.selected = visible[next];
    }
}

fn render_menu(frame: &mut Frame<'_>, app: &MenuApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new("Mole Linux")
            .block(Block::default().borders(Borders::ALL).title("Main menu")),
        chunks[0],
    );
    let visible = app.visible();
    let items = visible
        .iter()
        .map(|index| {
            let (_, name, description) = MENU_CHOICES[*index];
            ListItem::new(format!("{name}  {description}"))
        })
        .collect::<Vec<_>>();
    let selected = visible.iter().position(|index| *index == app.selected);
    let mut state = ListState::default();
    state.select(selected);
    let title = if app.searching {
        format!("Commands — search: {}", app.query)
    } else {
        "Commands".to_owned()
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, chunks[1], &mut state);
    let footer = if !app.searching {
        "↑/↓ move  / search  Enter select  Q Quit"
    } else {
        "↑/↓ move  Enter select  Esc clear  Q Quit"
    };
    let footer = if !app.searching {
        footer.to_owned()
    } else {
        format!("Search: {}_  |  {footer}", app.query)
    };
    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

pub fn run_menu() -> io::Result<Option<MenuChoice>> {
    install_panic_restore_hook();
    let _signals = MenuSignalGuard::install()?;
    let _terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = MenuApp::default();
    loop {
        if MENU_SIGNAL.swap(0, Ordering::Relaxed) != 0 {
            return Ok(None);
        }
        terminal.draw(|frame| render_menu(frame, &app))?;
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(None),
            Err(error) => return Err(error),
        }
        if MENU_SIGNAL.swap(0, Ordering::Relaxed) != 0 {
            return Ok(None);
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            KeyCode::Char('q') if !app.searching && app.query.is_empty() => return Ok(None),
            KeyCode::Esc if app.searching => {
                app.clear_search();
            }
            KeyCode::Esc if app.query.is_empty() => return Ok(None),
            KeyCode::Esc => {
                app.clear_search();
            }
            KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
            KeyCode::Char('/') => app.searching = true,
            KeyCode::Backspace if app.searching && !app.query.is_empty() => {
                app.query.pop();
                app.selected = app.visible().first().copied().unwrap_or_default();
            }
            KeyCode::Enter => {
                let visible = app.visible();
                if visible.is_empty() {
                    continue;
                }
                return Ok(visible
                    .iter()
                    .find(|index| **index == app.selected)
                    .and_then(|index| MENU_CHOICES.get(*index))
                    .map(|(choice, _, _)| *choice));
            }
            KeyCode::Char(character) if app.searching && !character.is_control() => {
                app.query.push(character);
                app.selected = app.visible().first().copied().unwrap_or_default();
            }
            _ => {}
        }
    }
}

type SignalHandler = usize;

struct MenuSignalGuard {
    previous_int: SignalHandler,
    previous_term: SignalHandler,
}

extern "C" fn record_menu_signal(signal: i32) {
    MENU_SIGNAL.store(signal, Ordering::Relaxed);
}

unsafe extern "C" {
    fn signal(signal: i32, handler: SignalHandler) -> SignalHandler;
}

impl MenuSignalGuard {
    fn install() -> io::Result<Self> {
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;
        MENU_SIGNAL.store(0, Ordering::Relaxed);
        // The handler only stores an atomic signal number; terminal I/O stays in the event loop.
        let handler = record_menu_signal as *const () as SignalHandler;
        let previous_int = unsafe { signal(SIGINT, handler) };
        if previous_int == SignalHandler::MAX {
            return Err(io::Error::last_os_error());
        }
        let previous_term = unsafe { signal(SIGTERM, handler) };
        if previous_term == SignalHandler::MAX {
            unsafe { signal(SIGINT, previous_int) };
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            previous_int,
            previous_term,
        })
    }
}

impl Drop for MenuSignalGuard {
    fn drop(&mut self) {
        unsafe {
            signal(2, self.previous_int);
            signal(15, self.previous_term);
        }
        MENU_SIGNAL.store(0, Ordering::Relaxed);
    }
}

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
            let child_path = child
                .target_path
                .clone()
                .unwrap_or_else(|| parent_path.join(&child.name));
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

    fn selected_sizes(&self) -> HashMap<PathBuf, Option<u64>> {
        self.selected
            .iter()
            .filter_map(|path| {
                self.files
                    .entries
                    .iter()
                    .find(|entry| &entry.path == path)
                    .map(|entry| (path.clone(), entry.size))
            })
            .collect()
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
            if let Some(target_path) = &child.target_path {
                path = target_path.clone();
            }
            node = child;
        }
        path
    }

    fn at_root(&self) -> bool {
        self.levels.len() == 1
    }
}

pub struct App {
    path: PathBuf,
    overview: bool,
    pub navigator: Navigator,
    pub filesystem: Option<FilesystemStats>,
    pub scanning: bool,
    pub size_mode: SizeMode,
    spinner_index: usize,
    scan_failed: bool,
    large_files: LargeFilesView,
    show_large_files: bool,
    main_selected: HashMap<PathBuf, Option<u64>>,
    pending_delete: Option<PendingDelete>,
    confirmation_scroll: u16,
    delete_report: Option<DeleteReport>,
    report_scroll: u16,
    delete_notice: Option<String>,
}

struct PendingDelete {
    entries: Vec<(PathBuf, Identity)>,
    sizes: HashMap<PathBuf, Option<u64>>,
}

struct DeleteReport {
    moved: usize,
    moved_bytes: Option<u64>,
    failures: Vec<(PathBuf, String)>,
}

impl App {
    pub fn new(
        path: PathBuf,
        root: Node,
        filesystem: Option<FilesystemStats>,
        scanning: bool,
    ) -> Self {
        let overview = root.children.iter().any(|node| node.target_path.is_some());
        Self {
            path,
            overview,
            navigator: Navigator::new(root),
            filesystem,
            scanning,
            size_mode: SizeMode::Allocated,
            spinner_index: 0,
            scan_failed: false,
            large_files: LargeFilesView::default(),
            show_large_files: false,
            main_selected: HashMap::new(),
            pending_delete: None,
            confirmation_scroll: 0,
            delete_report: None,
            report_scroll: 0,
            delete_notice: None,
        }
    }

    pub fn toggle_size_mode(&mut self) {
        self.size_mode = match self.size_mode {
            SizeMode::Allocated => SizeMode::Apparent,
            SizeMode::Apparent => SizeMode::Allocated,
        };
    }

    fn can_select_current(&self) -> bool {
        if self.scanning || (self.overview && self.navigator.at_root()) {
            return false;
        }
        self.navigator.selected_node().is_some_and(|node| {
            matches!(node.status, NodeStatus::Complete) && !matches!(node.kind, EntryKind::Other)
        })
    }

    fn current_selection_path(&self) -> Option<PathBuf> {
        if !self.can_select_current() {
            return None;
        }
        let node = self.navigator.selected_node()?;
        Some(
            node.target_path
                .clone()
                .unwrap_or_else(|| self.navigator.current_path(&self.path).join(&node.name)),
        )
    }

    fn toggle_current_selected(&mut self) {
        let Some(path) = self.current_selection_path() else {
            return;
        };
        if self.main_selected.remove(&path).is_none() {
            let size = self
                .navigator
                .selected_node()
                .filter(|node| matches!(node.status, NodeStatus::Complete))
                .map(|node| node.apparent);
            self.main_selected.insert(path, size);
        }
    }

    fn prepare_main_delete(&self) -> io::Result<Option<PendingDelete>> {
        if self.scanning || (self.overview && self.navigator.at_root()) {
            return Ok(None);
        }
        let mut candidates = self
            .main_selected
            .iter()
            .map(|(path, size)| (path.clone(), *size))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        prepare_delete(candidates)
    }

    fn prepare_large_delete(&self) -> io::Result<Option<PendingDelete>> {
        prepare_delete(self.large_files.selected_sizes().into_iter().collect())
    }

    fn apply_delete_results(
        &mut self,
        results: Vec<(PathBuf, Result<PathBuf, crate::core::sink::DeleteError>)>,
        sizes: &HashMap<PathBuf, Option<u64>>,
    ) -> bool {
        let mut moved = 0_usize;
        let mut moved_bytes = Some(0_u64);
        let mut moved_paths = HashSet::new();
        let mut failures = Vec::new();
        for (path, result) in results {
            match result {
                Ok(_) => {
                    moved = moved.saturating_add(1);
                    moved_paths.insert(path.clone());
                    moved_bytes = match (moved_bytes, sizes.get(&path).copied().flatten()) {
                        (Some(total), Some(size)) => total.checked_add(size),
                        _ => None,
                    };
                }
                Err(error) => failures.push((path, delete_error_reason(&error))),
            }
        }
        self.large_files
            .files
            .entries
            .retain(|entry| !moved_paths.contains(&entry.path));
        self.large_files
            .selected
            .retain(|path| !moved_paths.contains(path));
        self.large_files.selected_index = self
            .large_files
            .selected_index
            .min(self.large_files.files.entries.len().saturating_sub(1));
        self.main_selected
            .retain(|path, _| !moved_paths.contains(path));
        self.delete_report = Some(DeleteReport {
            moved,
            moved_bytes,
            failures,
        });
        self.report_scroll = 0;
        moved > 0
    }

    fn accept_report(&mut self, report: ScanReport) {
        self.overview = report
            .root
            .children
            .iter()
            .any(|node| node.target_path.is_some());
        self.large_files = LargeFilesView::new(LargeFiles::from_tree(&report.root, &self.path));
        self.navigator = Navigator::new(report.root);
        self.scanning = false;
    }

    fn accept_event(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Initial(root) => {
                self.overview = root.children.iter().any(|node| node.target_path.is_some());
                self.navigator = Navigator::new(root);
            }
            ScanEvent::ChildUpdated(updated) => {
                let root = &mut self.navigator.root;
                if let Some(child) = root
                    .children
                    .iter_mut()
                    .find(|child| child.name == updated.name)
                {
                    *child = updated;
                    root.apparent = root
                        .children
                        .iter()
                        .fold(0_u64, |sum, child| sum.saturating_add(child.apparent));
                    root.allocated = root
                        .children
                        .iter()
                        .fold(0_u64, |sum, child| sum.saturating_add(child.allocated));
                }
            }
            ScanEvent::Complete(report) => self.accept_report(report),
        }
    }
}

fn prepare_delete(candidates: Vec<(PathBuf, Option<u64>)>) -> io::Result<Option<PendingDelete>> {
    let mut candidates = candidates;
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    let mut entries = Vec::with_capacity(candidates.len());
    let mut sizes = HashMap::with_capacity(candidates.len());
    for (path, size) in candidates {
        let identity = Identity::capture(&path)?;
        sizes.insert(path.clone(), size);
        entries.push((path, identity));
    }
    if entries.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PendingDelete { entries, sizes }))
    }
}

fn delete_error_reason(error: &crate::core::sink::DeleteError) -> String {
    use crate::core::sink::DeleteError;
    match error {
        DeleteError::IdentityChanged => "target identity changed after preview".to_owned(),
        DeleteError::Whitelisted => "protected by the whitelist".to_owned(),
        DeleteError::Cancelled => "cancelled before processing".to_owned(),
        DeleteError::InvalidPath(_) => "path rejected by deletion policy".to_owned(),
        DeleteError::CrossFilesystemTrash => "Trash is on another filesystem".to_owned(),
        DeleteError::MountIdUnavailable => "mount identity unavailable".to_owned(),
        DeleteError::Partial(error)
        | DeleteError::TrashMetadataCleanup(error)
        | DeleteError::Io(error) => escape_name(OsStr::new(&error.to_string())),
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
            NodeStatus::Scanning | NodeStatus::Partial | NodeStatus::Unavailable(_)
        )
    {
        return None;
    }
    let percent = u128::from(child).saturating_mul(100) / u128::from(parent);
    u8::try_from(percent.min(100)).ok()
}

fn size_for_node(node: &Node, mode: SizeMode) -> String {
    match &node.status {
        NodeStatus::Scanning => "?".to_owned(),
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
        NodeStatus::Scanning => "Scanning".to_owned(),
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
    let heading = if app.overview && app.navigator.at_root() {
        "Overview".to_owned()
    } else {
        escape_name(app.navigator.current_path(&app.path).as_os_str())
    };
    let mut header_lines = vec![
        Line::from(heading),
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
        // Ported from tw93/Mole lib/ui/menu_paginated.sh:paginated_multi_select search prompt (GPL-3.0).
        let footer = if app.large_files.is_filtering() {
            format!(
                "Search: {}_  Enter apply  Esc clear",
                app.large_files.filter()
            )
        } else if app.scanning {
            "Scanning; selection and Trash are disabled".to_owned()
        } else {
            format!(
                "↑/↓ move  / filter  Space select  d Trash  r refresh  Esc back{}{}",
                if app.large_files.selected_count() > 0 {
                    format!("  {} selected", app.large_files.selected_count())
                } else {
                    String::new()
                },
                app.delete_notice
                    .as_ref()
                    .map_or_else(String::new, |notice| format!("  {notice}"))
            )
        };
        frame.render_widget(Paragraph::new(footer), chunks[2]);
        if let Some(pending) = &app.pending_delete {
            render_delete_confirmation(frame, pending, app.confirmation_scroll);
        }
        if let Some(report) = &app.delete_report {
            render_delete_result(frame, report, app.report_scroll);
        }
        return;
    }

    let denominator = size_for_mode(current, app.size_mode);
    let items = current
        .children
        .iter()
        .map(|node| {
            let child_path = node
                .target_path
                .clone()
                .unwrap_or_else(|| app.navigator.current_path(&app.path).join(&node.name));
            let selectable = !app.scanning
                && !(app.overview && app.navigator.at_root())
                && matches!(&node.status, NodeStatus::Complete)
                && !matches!(node.kind, EntryKind::Other);
            let selection = if app.main_selected.contains_key(&child_path) {
                "[x] "
            } else if selectable {
                "[ ] "
            } else {
                "    "
            };
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
                "{selection}{} {} {} {}{}{}",
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
    let can_select = !app.scanning && !(app.overview && app.navigator.at_root());
    let footer = if can_select {
        format!(
            "↑/↓ or j/k move  Space select  d Trash  Enter/l descend  h/Backspace up  t large files  r refresh  a size  q quit  {} selected{}",
            app.main_selected.len(),
            app.delete_notice
                .as_ref()
                .map_or_else(String::new, |notice| format!("  {notice}"))
        )
    } else {
        "↑/↓ or j/k move  Enter/l descend  h/Backspace up  t large files  r refresh  a size  q quit"
            .to_owned()
    };
    frame.render_widget(Paragraph::new(footer), chunks[2]);
    if let Some(pending) = &app.pending_delete {
        render_delete_confirmation(frame, pending, app.confirmation_scroll);
    }
    if let Some(report) = &app.delete_report {
        render_delete_result(frame, report, app.report_scroll);
    }
}

fn render_delete_confirmation(frame: &mut Frame<'_>, pending: &PendingDelete, scroll: u16) {
    let area = centered_rect(80, 70, frame.area());
    let paths = pending
        .entries
        .iter()
        .map(|(path, _)| escape_name(path.as_os_str()))
        .collect::<Vec<_>>();
    let mut lines = vec![Line::from("Trash these selected paths?")];
    lines.extend(paths.into_iter().map(Line::from));
    lines.push(Line::from("↑/↓ scroll  y confirm  n/Esc cancel"));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm Trash"),
            ),
        area,
    );
}

fn render_delete_result(frame: &mut Frame<'_>, report: &DeleteReport, scroll: u16) {
    let area = centered_rect(90, 80, frame.area());
    let size = report
        .moved_bytes
        .map_or_else(|| "unknown size".to_owned(), format_size);
    let mut lines = vec![Line::from(format!(
        "Moved {} item(s) to Trash ({} apparent). Space is not freed until Trash is emptied.",
        report.moved, size,
    ))];
    lines.extend(report.failures.iter().map(|(path, reason)| {
        Line::from(format!(
            "Failed: {}: {reason}",
            escape_name(path.as_os_str())
        ))
    }));
    lines.push(Line::from("↑/↓ scroll  Enter/Esc dismiss"));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(Block::default().borders(Borders::ALL).title("Trash result")),
        area,
    );
}

fn centered_rect(
    width_percent: u16,
    height_percent: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

fn restart_analyzer_scan(
    path: &Path,
    reports: &mut Receiver<ScanEvent>,
    active_cancel: &mut Arc<AtomicBool>,
    refresh_worker: &mut Option<std::thread::JoinHandle<()>>,
    app: &mut App,
) -> io::Result<()> {
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
    *refresh_worker = Some(
        std::thread::Builder::new()
            .name("mole-refresh".to_owned())
            .spawn(move || scan::scan_progressive(&worker_path, &worker_cancel, sender))?,
    );
    *active_cancel = next_cancel;
    *reports = receiver;
    app.scanning = true;
    app.scan_failed = false;
    Ok(())
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
            target_path: None,
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
                KeyCode::Up | KeyCode::Char('k') if app.delete_report.is_some() => {
                    app.report_scroll = app.report_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') if app.delete_report.is_some() => {
                    app.report_scroll = app.report_scroll.saturating_add(1);
                }
                KeyCode::Enter | KeyCode::Esc if app.delete_report.is_some() => {
                    app.delete_report = None;
                }
                _ if app.delete_report.is_some() => {}
                KeyCode::Up | KeyCode::Char('k') if app.pending_delete.is_some() => {
                    app.confirmation_scroll = app.confirmation_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') if app.pending_delete.is_some() => {
                    app.confirmation_scroll = app.confirmation_scroll.saturating_add(1);
                }
                KeyCode::Char('y') if app.pending_delete.is_some() => {
                    let Some(pending) = app.pending_delete.take() else {
                        continue;
                    };
                    let paths = pending
                        .entries
                        .iter()
                        .map(|(path, _)| path.clone())
                        .collect::<Vec<_>>();
                    let xdg = XdgPaths::from_env().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid")
                    })?;
                    let whitelist = Whitelist::load(&xdg).map_err(|error| {
                        io::Error::other(format!("failed to load whitelist: {error:?}"))
                    })?;
                    let context = SinkContext::new(xdg, whitelist);
                    let oplog = OpLog::new()?;
                    let results = trash_confirmed(pending.entries, &paths, &context, &oplog)?;
                    app.delete_notice = None;
                    if app.apply_delete_results(results, &pending.sizes) {
                        restart_analyzer_scan(
                            path,
                            &mut reports,
                            &mut active_cancel,
                            &mut refresh_worker,
                            &mut app,
                        )?;
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc if app.pending_delete.is_some() => {
                    app.pending_delete = None;
                    app.confirmation_scroll = 0;
                }
                _ if app.pending_delete.is_some() => {}
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
                    if !app.scanning {
                        app.large_files.toggle_selected()
                    }
                }
                KeyCode::Char('d')
                    if app.show_large_files && !app.large_files.is_filtering() && !app.scanning =>
                {
                    match app.prepare_large_delete() {
                        Ok(Some(pending)) => {
                            app.pending_delete = Some(pending);
                            app.delete_notice = None;
                        }
                        Ok(None) => {}
                        Err(_) => {
                            app.delete_notice = Some(
                                "a selected path is unavailable; refresh and select again"
                                    .to_owned(),
                            );
                        }
                    }
                }
                KeyCode::Char(' ') if !app.show_large_files => {
                    app.toggle_current_selected();
                }
                KeyCode::Char('d')
                    if !app.show_large_files
                        && !app.scanning
                        && !(app.overview && app.navigator.at_root()) =>
                {
                    match app.prepare_main_delete() {
                        Ok(Some(pending)) => {
                            app.pending_delete = Some(pending);
                            app.delete_notice = None;
                        }
                        Ok(None) => {}
                        Err(_) => {
                            app.delete_notice = Some(
                                "a selected path is unavailable; refresh and select again"
                                    .to_owned(),
                            );
                        }
                    }
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
                    restart_analyzer_scan(
                        path,
                        &mut reports,
                        &mut active_cancel,
                        &mut refresh_worker,
                        &mut app,
                    )?;
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

pub fn run_status(mut collector: status::Collector, interval: Duration) -> io::Result<()> {
    install_panic_restore_hook();
    let _terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    loop {
        let snapshot = collector.collect();
        let collected_at = std::time::Instant::now();
        terminal.draw(|frame| render_status_dashboard(frame, &snapshot, interval))?;
        loop {
            let elapsed = collected_at.elapsed();
            if elapsed >= interval {
                break;
            }
            let remaining = interval.saturating_sub(elapsed);
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

fn render_status_dashboard(frame: &mut Frame<'_>, snapshot: &status::Snapshot, interval: Duration) {
    // Linux dashboard adaptation of tw93/Mole cmd/status/main.go:model.View and view.go card rendering (GPL-3.0).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(frame.area());
    frame.render_widget(Paragraph::new("Mole Linux · System Status"), chunks[0]);
    let first_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);
    let second_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);
    let third_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[3]);

    let mut cpu_lines = vec![Line::from(format!(
        "Total {:>7} {}",
        status_percent(snapshot.cpu.usage_percent),
        status_cpu_bar(snapshot.cpu.usage_percent, 20)
    ))];
    for pair in snapshot.cpu.per_core_ticks.chunks(2) {
        let rendered = pair
            .iter()
            .map(|core| {
                format!(
                    "{} {} {}",
                    core.name,
                    status_percent(core.usage_percent),
                    status_cpu_bar(core.usage_percent, 8)
                )
            })
            .collect::<Vec<_>>();
        cpu_lines.push(Line::from(rendered.join("  ")));
    }
    render_status_panel(frame, first_row[0], "CPU", cpu_lines);

    render_status_panel(
        frame,
        first_row[1],
        "Memory / Swap / PSI",
        vec![
            Line::from(format!(
                "Memory {} / {}",
                status::format_kib_gib(snapshot.memory.used_kb),
                status::format_kib_gib(snapshot.memory.total_kb)
            )),
            Line::from(format!(
                "Swap   {} / {}",
                status::format_kib_gib(snapshot.memory.swap_used_kb),
                status::format_kib_gib(snapshot.memory.swap_total_kb)
            )),
            Line::from(format!(
                "PSI CPU {}  Memory {}  I/O {}",
                status_percent(snapshot.cpu_pressure.some_avg10),
                status_percent(snapshot.memory_pressure.some_avg10),
                status_percent(snapshot.io_pressure.some_avg10)
            )),
        ],
    );

    let mut disk_lines = Vec::new();
    for disk in snapshot.disks.mounts.iter().take(2) {
        disk_lines.push(Line::from(format!(
            "{} {} / {}",
            disk.mountpoint,
            status::format_bytes_gb(disk.used_bytes),
            status::format_bytes_gb(disk.total_bytes)
        )));
    }
    for device in snapshot.disk_io.devices.iter().take(2) {
        disk_lines.push(Line::from(format!(
            "{} r {}  w {}",
            device.name,
            status_byte_rate(device.read_bytes_per_sec),
            status_byte_rate(device.write_bytes_per_sec)
        )));
    }
    if disk_lines.is_empty() {
        disk_lines.push(Line::from("Disk data unavailable"));
    }
    render_status_panel(frame, second_row[0], "Disks / I/O", disk_lines);

    let network_lines = snapshot
        .network
        .interfaces
        .iter()
        .take(4)
        .map(|interface| {
            Line::from(format!(
                "{} ↓ {}  ↑ {}",
                interface.name,
                status_byte_rate(interface.rx_bytes_per_sec),
                status_byte_rate(interface.tx_bytes_per_sec)
            ))
        })
        .collect::<Vec<_>>();
    render_status_panel(
        frame,
        second_row[1],
        "Network",
        if network_lines.is_empty() {
            vec![Line::from("Network data unavailable")]
        } else {
            network_lines
        },
    );

    let process_lines = snapshot
        .processes
        .top
        .iter()
        .take(5)
        .map(|process| {
            Line::from(format!(
                "{:>6} {:>7} {}",
                process.pid,
                status_percent(process.cpu_percent),
                process.command
            ))
        })
        .collect::<Vec<_>>();
    render_status_panel(
        frame,
        third_row[0],
        "Top Processes",
        if process_lines.is_empty() {
            vec![Line::from("Process data unavailable")]
        } else {
            process_lines
        },
    );

    let mut alert_lines = snapshot
        .high_cpu_alerts
        .iter()
        .map(|alert| Line::from(format!("High CPU: {} ({})", alert.command, alert.pid)))
        .collect::<Vec<_>>();
    for battery in &snapshot.batteries {
        alert_lines.push(Line::from(format!(
            "Battery {} {}% {}",
            battery.name,
            battery
                .capacity_percent
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
            battery.status.as_deref().unwrap_or("unknown")
        )));
    }
    for thermal in &snapshot.thermal {
        alert_lines.push(Line::from(format!(
            "Thermal {} {} {}",
            thermal.name,
            thermal.sensor_type.as_deref().unwrap_or("unknown"),
            thermal.millicelsius.map_or_else(
                || "unknown".to_owned(),
                |value| format!(
                    "{}.{:01}°C",
                    value / 1000,
                    (value.unsigned_abs() % 1000) / 100
                )
            )
        )));
    }
    if alert_lines.is_empty() {
        alert_lines.push(Line::from("No active alerts"));
    }
    render_status_panel(frame, third_row[1], "Alerts / Power / Thermal", alert_lines);
    frame.render_widget(
        Paragraph::new(format!(
            " q / Esc quit  ·  refresh every {}",
            status_duration(interval)
        )),
        chunks[4],
    );
}

fn render_status_panel(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    title: &str,
    lines: Vec<Line<'static>>,
) {
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(" {title} "))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn status_percent(value: Option<f64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| format!("{value:.1}%"))
}

fn status_cpu_bar(value: Option<f64>, width: u32) -> String {
    let Some(value) = value else {
        return format!("[{}]", "?".repeat(usize::try_from(width).unwrap_or(20)));
    };
    let value = value.clamp(0.0, 100.0);
    let blocks = (0_u32..width)
        .map(|index| {
            if value >= f64::from(index + 1) * 100.0 / f64::from(width) {
                '█'
            } else {
                '░'
            }
        })
        .collect::<String>();
    format!("[{blocks}]")
}

fn status_byte_rate(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |rate| format!("{rate} B/s"))
}

fn status_duration(value: Duration) -> String {
    if value.as_secs() > 0 {
        format!("{}s", value.as_secs())
    } else {
        format!("{}ms", value.as_millis())
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
