use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

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
    let mut tree = Vec::<TreeEntry>::new();
    let mut pending = vec![(path.to_owned(), None::<usize>, None::<u64>)];
    let mut hardlinks = HashSet::new();

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

        let duplicate_hardlink = kind == EntryKind::File
            && metadata.nlink() > 1
            && !hardlinks.insert((metadata.dev(), metadata.ino()));
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
