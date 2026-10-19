//! The giv file trie: tracked/untracked sections, directories first, compact paths.
//! The catalog variant shows every project file in one tree.
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
    /// The row is a changed file or a directory containing one.
    pub changed: bool,
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
    changed: bool,
}

impl Node {
    fn leaf(&self) -> bool {
        self.file.is_some() && self.children.is_empty()
    }

    fn insert(&mut self, file: &ChangedFile) {
        let changed = !file.clean();
        let mut node = self;
        node.changed |= changed;
        for component in file.path.components() {
            node = node
                .children
                .entry(component.as_os_str().into())
                .or_default();
            node.changed |= changed;
        }
        node.file = Some(file.clone());
    }

    fn append(
        &self,
        parent: &Path,
        depth: usize,
        untracked: bool,
        expanded: &dyn Fn(&Path) -> bool,
        rows: &mut Vec<TreeRow>,
    ) {
        let mut children: Vec<_> = self.children.iter().collect();
        children.sort_by(|(a, x), (b, y)| x.leaf().cmp(&y.leaf()).then(a.cmp(b)));
        for (name, node) in children {
            if node.leaf() {
                let file = node.file.clone().unwrap();
                rows.push(TreeRow {
                    untracked: untracked || file.untracked(),
                    changed: node.changed,
                    kind: RowKind::File(file),
                    label: name.to_string_lossy().into_owned(),
                    depth,
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
            let open = expanded(&path);
            rows.push(TreeRow {
                kind: RowKind::Directory {
                    path: path.clone(),
                    expanded: open,
                },
                label,
                depth,
                untracked,
                changed: node.changed,
            });
            if open {
                tip.append(&path, depth + 1, untracked, expanded, rows);
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
            root.insert(file);
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
                changed: false,
            });
        }
        rows.push(TreeRow {
            kind: RowKind::Section,
            label: if untracked { "UNTRACKED" } else { "TRACKED" }.into(),
            depth: 0,
            untracked,
            changed: false,
        });
        root.append(
            Path::new(""),
            0,
            untracked,
            &|path| !collapsed.contains(path),
            &mut rows,
        );
    }
    rows
}

/// All project files as one directory tree, without sections. Directories open only
/// when listed in `expanded`, or all of them with `expand_all` (while filtering).
pub fn build_catalog(
    files: &[ChangedFile],
    expanded: &HashSet<PathBuf>,
    expand_all: bool,
) -> Vec<TreeRow> {
    let mut root = Node::default();
    for file in files {
        root.insert(file);
    }
    let mut rows = Vec::new();
    root.append(
        Path::new(""),
        0,
        false,
        &|path| expand_all || expanded.contains(path),
        &mut rows,
    );
    rows
}

/// The directories holding changed files, used to open the catalog on its changes.
pub fn changed_directories(files: &[ChangedFile]) -> HashSet<PathBuf> {
    files
        .iter()
        .filter(|file| !file.clean())
        .flat_map(|file| file.path.ancestors().skip(1))
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .collect()
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
