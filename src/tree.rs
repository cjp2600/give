//! The giv file trie: tracked/untracked sections, directories first, compact paths.
use crate::git::ChangedFile;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Section,
    Divider,
    Directory { path: PathBuf, expanded: bool },
    File(ChangedFile),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub kind: RowKind,
    pub label: String,
    pub depth: usize,
    pub untracked: bool,
}

impl TreeRow {
    pub fn path(&self) -> Option<&Path> {
        match &self.kind {
            RowKind::Directory { path, .. } => Some(path),
            RowKind::File(file) => Some(&file.path),
            _ => None,
        }
    }

    pub fn navigable(&self) -> bool {
        self.path().is_some()
    }
}

#[derive(Default)]
struct Node {
    children: BTreeMap<std::ffi::OsString, Node>,
    file: Option<ChangedFile>,
}

impl Node {
    fn leaf(&self) -> bool {
        self.file.is_some() && self.children.is_empty()
    }

    fn append(
        &self,
        parent: &Path,
        depth: usize,
        untracked: bool,
        collapsed: &HashSet<PathBuf>,
        rows: &mut Vec<TreeRow>,
    ) {
        let mut children: Vec<_> = self.children.iter().collect();
        children.sort_by(|(a, x), (b, y)| x.leaf().cmp(&y.leaf()).then(a.cmp(b)));
        for (name, node) in children {
            if node.leaf() {
                rows.push(TreeRow {
                    kind: RowKind::File(node.file.clone().unwrap()),
                    label: name.to_string_lossy().into_owned(),
                    depth,
                    untracked,
                });
                continue;
            }
            let mut tip = node;
            let mut path = parent.join(name);
            let mut label = name.to_string_lossy().into_owned();
            // Merge only directory-to-directory chains, never the final file.
            while tip.children.len() == 1 {
                let (name, child) = tip.children.first_key_value().unwrap();
                if child.leaf() {
                    break;
                }
                path.push(name);
                label.push('/');
                label.push_str(&name.to_string_lossy());
                tip = child;
            }
            let expanded = !collapsed.contains(&path);
            rows.push(TreeRow {
                kind: RowKind::Directory {
                    path: path.clone(),
                    expanded,
                },
                label,
                depth,
                untracked,
            });
            if expanded {
                tip.append(&path, depth + 1, untracked, collapsed, rows);
            }
        }
    }
}

pub fn build_tree(files: &[ChangedFile], collapsed: &HashSet<PathBuf>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for untracked in [false, true] {
        let mut root = Node::default();
        let mut count = 0;
        for file in files.iter().filter(|f| f.untracked() == untracked) {
            let mut node = &mut root;
            for component in file.path.components() {
                node = node
                    .children
                    .entry(component.as_os_str().into())
                    .or_default();
            }
            node.file = Some(file.clone());
            count += 1;
        }
        if count == 0 {
            continue;
        }
        if !rows.is_empty() {
            rows.push(TreeRow {
                kind: RowKind::Divider,
                label: String::new(),
                depth: 0,
                untracked,
            });
        }
        rows.push(TreeRow {
            kind: RowKind::Section,
            label: if untracked { "UNTRACKED" } else { "TRACKED" }.into(),
            depth: 0,
            untracked,
        });
        root.append(Path::new(""), 0, untracked, collapsed, &mut rows);
    }
    rows
}

pub fn prune_collapsed(collapsed: &mut HashSet<PathBuf>, files: &[ChangedFile]) {
    if collapsed.is_empty() {
        return;
    }
    let valid: HashSet<_> = files
        .iter()
        .flat_map(|file| file.path.ancestors().skip(1))
        .filter(|path| !path.as_os_str().is_empty())
        .collect();
    collapsed.retain(|dir| valid.contains(dir.as_path()));
}
