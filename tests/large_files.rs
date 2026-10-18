use std::ffi::OsString;
use std::path::PathBuf;

use mole_linux::scan::{EntryKind, Node, NodeStatus};
use mole_linux::tui::{LargeFiles, LargeFilesView};

fn file(name: &str, size: u64) -> Node {
    Node {
        name: OsString::from(name),
        kind: EntryKind::File,
        cleanable: false,
        apparent: size,
        allocated: size,
        children: Vec::new(),
        status: NodeStatus::Complete,
    }
}

#[test]
fn large_files_view_keeps_top_twenty_and_unknown_files_unselectable() {
    let mut root = Node {
        name: OsString::from("root"),
        kind: EntryKind::Directory,
        cleanable: false,
        apparent: 0,
        allocated: 0,
        children: (0_u64..25)
            .map(|size| file(&format!("file-{size}"), size))
            .collect(),
        status: NodeStatus::Complete,
    };
    root.children.push(Node {
        name: OsString::from("unknown"),
        kind: EntryKind::File,
        cleanable: false,
        apparent: 0,
        allocated: 0,
        children: Vec::new(),
        status: NodeStatus::Unavailable("permission denied".to_owned()),
    });

    let files = LargeFiles::from_tree(&root, &PathBuf::from("/tmp"));
    assert_eq!(files.entries().len(), 20);
    assert!(
        !files
            .entries()
            .iter()
            .any(|entry| entry.path.ends_with("unknown"))
    );
    assert_eq!(files.entries()[0].size, Some(24));

    let mut view = LargeFilesView::new(files);
    view.toggle_selected();
    assert_eq!(view.selected_count(), 1);
}

#[test]
fn filter_multiselect_and_history_restore_view_state() {
    let mut view = LargeFilesView::from_entries(vec![
        (PathBuf::from("/tmp/alpha.bin"), Some(100)),
        (PathBuf::from("/tmp/beta.bin"), Some(200)),
    ]);
    view.begin_filter();
    view.push_filter("beta");
    view.apply_filter();
    assert_eq!(view.visible_entries().len(), 1);
    view.toggle_selected();
    assert_eq!(view.selected_count(), 1);

    let saved = view.clone();
    view.clear_filter();
    view.move_selection(1);
    view.restore(saved);
    assert_eq!(
        view.visible_entries()[0].path,
        PathBuf::from("/tmp/beta.bin")
    );
    assert_eq!(view.selected_count(), 1);
}
