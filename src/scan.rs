use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc::Sender};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeStatus {
    Complete,
    Partial,
    Unavailable(String),
    Excluded(String),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: OsString,
    pub kind: EntryKind,
    pub apparent: u64,
    pub allocated: u64,
    pub children: Vec<Node>,
    pub status: NodeStatus,
}

#[derive(Debug)]
pub struct ScanReport {
    pub root: Node,
}

#[derive(Serialize)]
struct JsonOutput {
    scan_status: &'static str,
    path: String,
    overview: bool,
    entries: Vec<JsonEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    large_files: Vec<JsonFileEntry>,
    apparent_bytes: u64,
    allocated_bytes: u64,
    total_apparent_bytes: u64,
    total_allocated_bytes: u64,
    total_size: u64,
    total_files: u64,
}

#[derive(Serialize)]
struct JsonEntry {
    scan_status: &'static str,
    name: String,
    name_bytes: Vec<u8>,
    path: String,
    /// Upstream-compatible `size` field; reports allocated bytes.
    size: u64,
    apparent_bytes: u64,
    allocated_bytes: u64,
    is_dir: bool,
}

#[derive(Serialize)]
struct JsonFileEntry {
    name: String,
    name_bytes: Vec<u8>,
    path: String,
    /// Upstream-compatible `size` field; reports allocated bytes.
    size: u64,
    apparent_bytes: u64,
    allocated_bytes: u64,
}

/// Serializes the same scan tree used by the interactive analyzer.
// Ported from tw93/Mole cmd/analyze/json.go:jsonEntriesFromDirEntries (GPL-3.0)
pub fn report_json(report: &ScanReport, path: &Path) -> Result<String, serde_json::Error> {
    let mut files = Vec::new();
    collect_files(&report.root, path, &mut files);
    files.sort_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.path.cmp(&right.path))
    });
    files.truncate(20);
    let total_files = count_files(&report.root);
    let output = JsonOutput {
        scan_status: status_name(&report.root.status),
        path: path.to_string_lossy().into_owned(),
        overview: false,
        entries: report
            .root
            .children
            .iter()
            .map(|node| json_entry(node, &path.join(&node.name)))
            .collect(),
        large_files: files,
        apparent_bytes: report.root.apparent,
        allocated_bytes: report.root.allocated,
        total_apparent_bytes: report
            .root
            .children
            .iter()
            .fold(0_u64, |total, child| total.saturating_add(child.apparent)),
        total_allocated_bytes: report
            .root
            .children
            .iter()
            .fold(0_u64, |total, child| total.saturating_add(child.allocated)),
        total_size: report
            .root
            .children
            .iter()
            .fold(0_u64, |total, child| total.saturating_add(child.allocated)),
        total_files,
    };
    serde_json::to_string_pretty(&output)
}

fn json_entry(node: &Node, path: &Path) -> JsonEntry {
    JsonEntry {
        scan_status: status_name(&node.status),
        name: node.name.to_string_lossy().into_owned(),
        name_bytes: node.name.as_encoded_bytes().to_vec(),
        path: path.to_string_lossy().into_owned(),
        size: node.allocated,
        apparent_bytes: node.apparent,
        allocated_bytes: node.allocated,
        is_dir: node.kind == EntryKind::Directory,
    }
}

fn collect_files(node: &Node, path: &Path, files: &mut Vec<JsonFileEntry>) {
    for child in &node.children {
        let child_path = path.join(&child.name);
        if child.kind == EntryKind::File {
            files.push(JsonFileEntry {
                name: child.name.to_string_lossy().into_owned(),
                name_bytes: child.name.as_encoded_bytes().to_vec(),
                path: child_path.to_string_lossy().into_owned(),
                size: child.allocated,
                apparent_bytes: child.apparent,
                allocated_bytes: child.allocated,
            });
        } else {
            collect_files(child, &child_path, files);
        }
    }
}

fn count_files(node: &Node) -> u64 {
    node.children.iter().fold(0_u64, |total, child| {
        total
            .saturating_add(u64::from(child.kind == EntryKind::File))
            .saturating_add(count_files(child))
    })
}

fn status_name(status: &NodeStatus) -> &'static str {
    match status {
        NodeStatus::Complete => "complete",
        NodeStatus::Partial => "partial",
        NodeStatus::Unavailable(_) => "unavailable",
        NodeStatus::Excluded(_) => "excluded",
    }
}

#[derive(Debug)]
pub enum ScanEvent {
    Initial(Node),
    ChildUpdated(Node),
    Complete(ScanReport),
}

struct TreeEntry {
    node: Node,
    children: Vec<usize>,
}

pub fn scan(path: &Path, cancel: &AtomicBool) -> ScanReport {
    scan_with_device_ids(path, cancel, |_, metadata| metadata.dev())
}

/// Scans while allowing the device id lookup to be substituted in tests.
pub fn scan_with_device_ids<F>(path: &Path, cancel: &AtomicBool, device_id: F) -> ScanReport
where
    F: Fn(&Path, &fs::Metadata) -> u64,
{
    scan_with_shared_hardlinks(
        path,
        cancel,
        device_id,
        Arc::new(Mutex::new(HashSet::new())),
        None,
    )
}

fn scan_with_shared_hardlinks<F>(
    path: &Path,
    cancel: &AtomicBool,
    device_id: F,
    hardlinks: Arc<Mutex<HashSet<(u64, u64)>>>,
    root_device: Option<u64>,
) -> ScanReport
where
    F: Fn(&Path, &fs::Metadata) -> u64,
{
    let mut tree = Vec::<TreeEntry>::new();
    let mut pending = vec![(path.to_owned(), None::<usize>, root_device)];

    while let Some((entry_path, parent, root_dev)) = pending.pop() {
        if cancel.load(Ordering::Relaxed) && parent.is_some() {
            if let Some(parent) = parent {
                tree[parent].node.status = NodeStatus::Partial;
            }
            for (_, pending_parent, _) in pending.drain(..) {
                if let Some(pending_parent) = pending_parent {
                    tree[pending_parent].node.status = NodeStatus::Partial;
                }
            }
            break;
        }

        let name = entry_path
            .file_name()
            .map_or_else(|| entry_path.as_os_str().to_owned(), OsString::from);
        let metadata = match fs::symlink_metadata(&entry_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                let node = unavailable_node(name, error.to_string());
                insert_tree_entry(node, parent, &mut tree);
                continue;
            }
        };
        let kind = entry_kind(&metadata);
        let entry_dev = device_id(&entry_path, &metadata);
        let root_dev = root_dev.unwrap_or(entry_dev);
        if is_mount_boundary(root_dev, entry_dev) {
            let node = Node {
                name,
                kind,
                apparent: 0,
                allocated: 0,
                children: Vec::new(),
                status: NodeStatus::Excluded("mount boundary".to_owned()),
            };
            insert_tree_entry(node, parent, &mut tree);
            continue;
        }

        let duplicate_hardlink = if kind == EntryKind::File && metadata.nlink() > 1 {
            match hardlinks.lock() {
                Ok(mut seen) => !seen.insert((metadata.dev(), metadata.ino())),
                Err(_) => true,
            }
        } else {
            false
        };
        let node = Node {
            name,
            kind,
            apparent: if duplicate_hardlink {
                0
            } else {
                metadata.len()
            },
            allocated: if duplicate_hardlink {
                0
            } else {
                metadata.blocks().saturating_mul(512)
            },
            children: Vec::new(),
            status: NodeStatus::Complete,
        };
        let node_index = insert_tree_entry(node, parent, &mut tree);

        if kind != EntryKind::Directory {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            tree[node_index].node.status = NodeStatus::Partial;
            continue;
        }

        let entries = match fs::read_dir(&entry_path) {
            Ok(entries) => entries,
            Err(error) => {
                tree[node_index].node.apparent = 0;
                tree[node_index].node.allocated = 0;
                tree[node_index].node.status = NodeStatus::Unavailable(error.to_string());
                continue;
            }
        };
        let mut paths = Vec::<PathBuf>::new();
        let mut cancelled_during_listing = false;
        for entry in entries {
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(error) => {
                    let error_node = unavailable_node(OsString::new(), error.to_string());
                    insert_tree_entry(error_node, Some(node_index), &mut tree);
                }
            }
            if cancel.load(Ordering::Relaxed) {
                cancelled_during_listing = true;
                break;
            }
        }
        if cancelled_during_listing {
            tree[node_index].node.status = NodeStatus::Partial;
            continue;
        }
        paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
        if cancel.load(Ordering::Relaxed) {
            tree[node_index].node.status = NodeStatus::Partial;
        } else {
            for child_path in paths.into_iter().rev() {
                pending.push((child_path, Some(node_index), Some(root_dev)));
            }
        }
    }

    let mut built = (0..tree.len()).map(|_| None).collect::<Vec<Option<Node>>>();
    for index in (0..tree.len()).rev() {
        let TreeEntry { mut node, children } = std::mem::replace(
            &mut tree[index],
            TreeEntry {
                node: unavailable_node(OsString::new(), String::new()),
                children: Vec::new(),
            },
        );
        for child_index in children {
            if let Some(child) = built[child_index].take() {
                node.apparent = node.apparent.saturating_add(child.apparent);
                node.allocated = node.allocated.saturating_add(child.allocated);
                if matches!(
                    child.status,
                    NodeStatus::Partial | NodeStatus::Unavailable(_)
                ) && node.status == NodeStatus::Complete
                {
                    node.status = NodeStatus::Partial;
                }
                node.children.push(child);
            }
        }
        node.children.sort_by(|left, right| {
            right
                .allocated
                .cmp(&left.allocated)
                .then_with(|| left.name.cmp(&right.name))
        });
        built[index] = Some(node);
    }

    let root = match built.into_iter().next().flatten() {
        Some(root) => root,
        None => unavailable_node(OsString::new(), "scan produced no root".to_owned()),
    };
    ScanReport { root }
}

/// Publishes the immediate root listing before scanning directories in a bounded worker pool.
// Ported from tw93/Mole cmd/analyze/live_scan.go:readLiveScanInitialEntries (GPL-3.0)
pub fn scan_progressive(path: &Path, cancel: &AtomicBool, events: Sender<ScanEvent>) {
    let name = path
        .file_name()
        .map_or_else(|| path.as_os_str().to_owned(), OsString::from);
    let root_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            let root = unavailable_node(name, error.to_string());
            if !publish_event(&events, cancel, ScanEvent::Initial(root.clone())) {
                return;
            }
            publish_event(&events, cancel, ScanEvent::Complete(ScanReport { root }));
            return;
        }
    };
    if !root_metadata.is_dir() {
        let mut root = scan(path, cancel).root;
        root.name = name;
        if !publish_event(&events, cancel, ScanEvent::Initial(root.clone())) {
            return;
        }
        publish_event(&events, cancel, ScanEvent::Complete(ScanReport { root }));
        return;
    }

    let root_dev = root_metadata.dev();
    let mut root = Node {
        name,
        kind: EntryKind::Directory,
        apparent: root_metadata.len(),
        allocated: root_metadata.blocks().saturating_mul(512),
        children: Vec::new(),
        status: NodeStatus::Partial,
    };
    let mut targets = Vec::new();
    let hardlinks = Arc::new(Mutex::new(HashSet::new()));
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            root.status = NodeStatus::Unavailable(error.to_string());
            root.apparent = 0;
            root.allocated = 0;
            if !publish_event(&events, cancel, ScanEvent::Initial(root.clone())) {
                return;
            }
            publish_event(&events, cancel, ScanEvent::Complete(ScanReport { root }));
            return;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => root
                .children
                .push(unavailable_node(OsString::new(), error.to_string())),
        }
    }
    paths.sort_by(|left: &PathBuf, right: &PathBuf| left.file_name().cmp(&right.file_name()));
    for child_path in paths {
        let child_name = child_path
            .file_name()
            .map_or_else(|| child_path.as_os_str().to_owned(), OsString::from);
        match fs::symlink_metadata(&child_path) {
            Ok(metadata) => {
                let kind = entry_kind(&metadata);
                if is_mount_boundary(root_dev, metadata.dev()) {
                    root.children.push(Node {
                        name: child_name,
                        kind,
                        apparent: 0,
                        allocated: 0,
                        children: Vec::new(),
                        status: NodeStatus::Excluded("mount boundary".to_owned()),
                    });
                } else {
                    let duplicate = kind == EntryKind::File
                        && metadata.nlink() > 1
                        && hardlinks
                            .lock()
                            .map(|mut seen| !seen.insert((metadata.dev(), metadata.ino())))
                            .unwrap_or(true);
                    let node = Node {
                        name: child_name,
                        kind,
                        apparent: if duplicate { 0 } else { metadata.len() },
                        allocated: if duplicate {
                            0
                        } else {
                            metadata.blocks().saturating_mul(512)
                        },
                        children: Vec::new(),
                        status: if kind == EntryKind::Directory {
                            NodeStatus::Partial
                        } else {
                            NodeStatus::Complete
                        },
                    };
                    if kind == EntryKind::Directory {
                        targets.push(child_path);
                    }
                    root.apparent = root.apparent.saturating_add(node.apparent);
                    root.allocated = root.allocated.saturating_add(node.allocated);
                    root.children.push(node);
                }
            }
            Err(error) => root
                .children
                .push(unavailable_node(child_name, error.to_string())),
        }
    }
    sort_children(&mut root.children);
    if !publish_event(&events, cancel, ScanEvent::Initial(root.clone())) {
        return;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (updates_tx, updates_rx) = std::sync::mpsc::channel();
    let worker_count = targets.len().clamp(1, 12);
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let targets = &targets;
            let updates_tx = updates_tx.clone();
            let hardlinks = Arc::clone(&hardlinks);
            let next = &next;
            scope.spawn(move || {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(child_path) = targets.get(index) else {
                        break;
                    };
                    let report = scan_with_shared_hardlinks(
                        child_path,
                        cancel,
                        |_, metadata| metadata.dev(),
                        Arc::clone(&hardlinks),
                        Some(root_dev),
                    );
                    if updates_tx.send(report.root).is_err() {
                        cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            });
        }
        drop(updates_tx);
        for updated in updates_rx {
            let Some(current) = root
                .children
                .iter_mut()
                .find(|child| child.name == updated.name)
            else {
                continue;
            };
            *current = updated;
            if !publish_event(&events, cancel, ScanEvent::ChildUpdated(current.clone())) {
                break;
            }
        }
    });
    root.apparent = root
        .children
        .iter()
        .fold(root_metadata.len(), |sum, child| {
            sum.saturating_add(child.apparent)
        });
    root.allocated = root
        .children
        .iter()
        .fold(root_metadata.blocks().saturating_mul(512), |sum, child| {
            sum.saturating_add(child.allocated)
        });
    if cancel.load(Ordering::Relaxed)
        || root.children.iter().any(|child| {
            matches!(
                child.status,
                NodeStatus::Partial | NodeStatus::Unavailable(_)
            )
        })
    {
        root.status = NodeStatus::Partial;
    } else {
        root.status = NodeStatus::Complete;
    }
    sort_children(&mut root.children);
    publish_event(&events, cancel, ScanEvent::Complete(ScanReport { root }));
}

fn publish_event(events: &Sender<ScanEvent>, cancel: &AtomicBool, event: ScanEvent) -> bool {
    if events.send(event).is_err() {
        cancel.store(true, Ordering::Relaxed);
        false
    } else {
        true
    }
}

fn sort_children(children: &mut [Node]) {
    children.sort_by(|left, right| {
        right
            .allocated
            .cmp(&left.allocated)
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn insert_tree_entry(node: Node, parent: Option<usize>, tree: &mut Vec<TreeEntry>) -> usize {
    let index = tree.len();
    tree.push(TreeEntry {
        node,
        children: Vec::new(),
    });
    if let Some(parent) = parent {
        tree[parent].children.push(index);
    }
    index
}

fn unavailable_node(name: OsString, reason: String) -> Node {
    Node {
        name,
        kind: EntryKind::Other,
        apparent: 0,
        allocated: 0,
        children: Vec::new(),
        status: NodeStatus::Unavailable(reason),
    }
}

fn entry_kind(metadata: &fs::Metadata) -> EntryKind {
    if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

pub fn is_mount_boundary(root_dev: u64, entry_dev: u64) -> bool {
    root_dev != entry_dev
}
