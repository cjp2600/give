use crate::{
    ActivateRow, CollapseRow, Commit, CopyDiff, CopySelection, ExpandRow, FirstRow, LastRow,
    NextChange, NextFile, OpenRepository, PageDown, PageUp, PreviousFile, Refresh, SelectAllText,
    SelectDown, SelectLeft, SelectRight, SelectUp, SwitchPane, ToggleDeletions, ToggleTerminal,
};
use give::{
    git::{ChangedFile, Diff, LineKind, Repository, Snapshot},
    preview,
    preview_cache::{Key, PreviewCache, Slot, Syntax},
    scrollbar,
    text_selection::{self, Point as TextPoint, Selection as TextSelection},
    tree::{self, RowKind, TreeRow},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable, TitleBar,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const BACKGROUND: u32 = 0x1b2632;
const PANEL: u32 = 0x192330;
const SURFACE: u32 = 0x213143;
const BORDER: u32 = 0x2a394b;
const TEXT: u32 = 0xcbd5e1;
const MUTED: u32 = 0x7a8da5;
const ACCENT: u32 = 0x87b1d7;
const GREEN: u32 = 0x9ac9ad;
const RED: u32 = 0xdb929d;
const LINE_HEIGHT: f32 = 26.;
const TREE_HEIGHT: f32 = 27.;

pub struct Give {
    repo: Option<Repository>,
    accessibility: crate::accessibility::Text,
    accessible_diff: Option<Arc<Diff>>,
    accessible_selection: Option<TextSelection>,
    terminal: Option<Entity<crate::terminal_view::TerminalView>>,
    terminals: HashMap<PathBuf, Entity<crate::terminal_view::TerminalView>>,
    terminal_visible: bool,
    terminal_width: f32,
    terminal_dragging: bool,
    snapshot: Snapshot,
    tree: Vec<TreeRow>,
    tree_cursor: Option<usize>,
    tree_request: u64,
    tree_task: Option<Task<()>>,
    collapsed: HashSet<PathBuf>,
    focus_left: bool,
    commit_open: bool,
    scrollbar_bounds: Rc<Cell<Bounds<Pixels>>>,
    scrollbar_drag: Option<f32>,
    changes: Arc<Vec<usize>>,
    show_deletions: bool,
    preview_cursor: usize,
    text_selection: Option<TextSelection>,
    preview_selecting: bool,
    selection_pointer: Point<Pixels>,
    text_layouts: HashMap<usize, TextLayout>,
    change_index: Option<usize>,
    syntax: Syntax,
    cache: PreviewCache,
    preview_target: Option<(Key, bool)>,
    prefetching: HashSet<Key>,
    current_slot: Option<Arc<Slot>>,
    syntax_pending: HashSet<(Key, bool, usize)>,
    selected: Option<PathBuf>,
    diff: Arc<Diff>,
    filter: String,
    filter_input: Entity<InputState>,
    commit_input: Entity<InputState>,
    focus: FocusHandle,
    files_scroll: UniformListScrollHandle,
    diff_scroll: UniformListScrollHandle,
    notice: String,
    error: bool,
    busy: bool,
    opening: bool,
    picking: bool,
    refreshing: bool,
    preview_loading: bool,
    epoch: u64,
    snapshot_request: u64,
    preview_request: u64,
    _subscriptions: Vec<Subscription>,
}

impl Give {
    pub fn new(initial: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter changed files…"));
        let commit_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Describe your changes…"));
        let subscriptions = vec![
            cx.subscribe_in(&filter_input, window, |this, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter = input.read(cx).value().to_string().to_lowercase();
                    this.rebuild_tree(true, cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&commit_input, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut this = Self {
            repo: None,
            accessibility: crate::accessibility::Text::default(),
            accessible_diff: None,
            accessible_selection: None,
            terminal: None,
            terminals: HashMap::new(),
            terminal_visible: false,
            terminal_width: 440.,
            terminal_dragging: false,
            snapshot: Snapshot::default(),
            tree: Vec::new(),
            tree_cursor: None,
            tree_request: 0,
            tree_task: None,
            collapsed: HashSet::new(),
            focus_left: true,
            commit_open: false,
            scrollbar_bounds: Rc::new(Cell::new(Bounds::default())),
            scrollbar_drag: None,
            changes: Arc::default(),
            show_deletions: false,
            preview_cursor: 0,
            text_selection: None,
            preview_selecting: false,
            selection_pointer: point(px(0.), px(0.)),
            text_layouts: HashMap::new(),
            change_index: None,
            syntax: Arc::default(),
            cache: PreviewCache::default(),
            preview_target: None,
            prefetching: HashSet::new(),
            current_slot: None,
            syntax_pending: HashSet::new(),
            selected: None,
            diff: Arc::default(),
            filter: String::new(),
            filter_input,
            commit_input,
            focus,
            files_scroll: UniformListScrollHandle::new(),
            diff_scroll: UniformListScrollHandle::new(),
            notice: "Ready when you are".into(),
            error: false,
            busy: false,
            opening: false,
            picking: false,
            refreshing: false,
            preview_loading: false,
            epoch: 0,
            snapshot_request: 0,
            preview_request: 0,
            _subscriptions: subscriptions,
        };
        cx.background_executor()
            .spawn(async { preview::warm_up() })
            .detach();
        if let Some(path) = initial {
            this.open(path, window, cx);
        }
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if view.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn rebuild_tree(&mut self, debounce: bool, cx: &mut Context<Self>) {
        self.tree_request += 1;
        let request = self.tree_request;
        let epoch = self.epoch;
        self.tree_task = Some(cx.spawn(async move |view, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(75))
                    .await;
            }
            let state = view
                .update(cx, |this, _| {
                    if request != this.tree_request || epoch != this.epoch {
                        return None;
                    }
                    Some((
                        this.snapshot.files.clone(),
                        this.collapsed.clone(),
                        this.filter.clone(),
                    ))
                })
                .ok()
                .flatten();
            let Some((files, mut collapsed, filter)) = state else {
                return;
            };
            let (rows, collapsed) = cx
                .background_executor()
                .spawn(async move {
                    tree::prune_collapsed(&mut collapsed, &files);
                    let files: Vec<_> = files
                        .into_iter()
                        .filter(|file| {
                            filter.is_empty()
                                || file.path.to_string_lossy().to_lowercase().contains(&filter)
                        })
                        .collect();
                    let expanded = HashSet::new();
                    let rows = tree::build_tree(
                        &files,
                        if filter.is_empty() {
                            &collapsed
                        } else {
                            &expanded
                        },
                    );
                    (rows, collapsed)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if request != this.tree_request || epoch != this.epoch {
                    return;
                }
                this.apply_tree(rows, collapsed);
                this.load_diff(true, cx);
                cx.notify();
            });
        }));
    }

    fn apply_tree(&mut self, rows: Vec<TreeRow>, collapsed: HashSet<PathBuf>) {
        let previous = self
            .tree_cursor
            .and_then(|i| self.tree.get(i))
            .and_then(|row| row.path().map(|p| (p.to_owned(), row.untracked)));
        self.tree = rows;
        self.collapsed = collapsed;
        self.tree_cursor = previous
            .and_then(|(path, untracked)| {
                self.tree
                    .iter()
                    .position(|row| {
                        row.path() == Some(path.as_path()) && row.untracked == untracked
                    })
                    .or_else(|| {
                        self.tree
                            .iter()
                            .position(|row| row.path() == Some(path.as_path()))
                    })
            })
            .or_else(|| self.tree.iter().position(TreeRow::navigable));
        let selected = self.tree_cursor.and_then(|i| match &self.tree[i].kind {
            RowKind::File(file) => Some(file.path.clone()),
            _ => None,
        });
        if self.selected != selected {
            self.selected = selected;
            self.reset_preview_position();
        }
    }

    fn reset_preview_position(&mut self) {
        self.diff = Arc::default();
        self.syntax = Arc::default();
        self.preview_target = None;
        self.current_slot = None;
        self.changes = Arc::default();
        self.diff_scroll = UniformListScrollHandle::new();
        self.preview_cursor = 0;
        self.text_selection = None;
        self.preview_selecting = false;
        self.text_layouts.clear();
        self.change_index = None;
    }

    fn open_picker(&mut self, _: &OpenRepository, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.opening || self.picking {
            return;
        }
        self.picking = true;
        let result = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = result.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.picking = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.open(path, window, cx);
                        }
                    }
                    Ok(Err(error)) => this.show_error(error, cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.epoch += 1;
        let epoch = self.epoch;
        self.opening = true;
        self.refreshing = false;
        self.preview_request += 1;
        self.notice = "Opening repository…".into();
        self.error = false;
        let task = cx.background_executor().spawn(async move {
            let repo = Repository::open(&path)?;
            let snapshot = repo.snapshot()?;
            anyhow::Ok((repo, snapshot))
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                if this.epoch != epoch {
                    return;
                }
                this.opening = false;
                match result {
                    Ok((repo, snapshot)) => {
                        window.set_window_title(&format!(
                            "{} — give",
                            repo.root.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        this.repo = Some(repo);
                        this.activate_project_terminal(window, cx);
                        this.cache = PreviewCache::default();
                        this.prefetching.clear();
                        this.syntax_pending.clear();
                        this.current_slot = None;
                        this.preview_target = None;
                        this.snapshot = snapshot;
                        this.selected = None;
                        this.tree.clear();
                        this.tree_cursor = None;
                        this.collapsed.clear();
                        this.focus_left = true;
                        this.filter.clear();
                        this.filter_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.commit_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.rebuild_tree(false, cx);
                        this.notice = "Watching local changes".into();
                    }
                    Err(error) => this.show_error(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.opening || self.refreshing {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.refreshing = true;
        self.snapshot_request += 1;
        let request = self.snapshot_request;
        let epoch = self.epoch;
        let task = cx
            .background_executor()
            .spawn(async move { repo.snapshot() });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if epoch != this.epoch || request != this.snapshot_request {
                    return;
                }
                this.refreshing = false;
                match result {
                    Ok(snapshot) => {
                        let changed_tree = this.snapshot.files != snapshot.files;
                        this.cache.retain_versions(&snapshot.versions);
                        this.snapshot = snapshot;
                        if changed_tree {
                            this.rebuild_tree(false, cx);
                        }
                        this.load_diff(true, cx);
                    }
                    Err(error) => this.show_error(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_diff(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let file = self
            .snapshot
            .files
            .iter()
            .find(|file| Some(&file.path) == self.selected.as_ref())
            .cloned();
        let (Some(repo), Some(file)) = (self.repo.clone(), file) else {
            self.preview_request += 1;
            self.preview_target = None;
            self.current_slot = None;
            self.diff = Arc::default();
            self.syntax = Arc::default();
            self.preview_loading = false;
            self.prefetch_neighbors(cx);
            return;
        };
        let key = Key {
            path: file.path.clone(),
            version: self.snapshot.versions[&file.path],
        };
        let show = self.show_deletions;
        if self.preview_target.as_ref() == Some(&(key.clone(), show)) {
            self.prefetch_neighbors(cx);
            return;
        }
        self.preview_request += 1;
        let request = self.preview_request;
        let epoch = self.epoch;
        self.preview_target = Some((key.clone(), show));
        let slot = self.cache.slot(key);
        let anchor = self
            .diff
            .lines
            .get(self.preview_cursor)
            .and_then(|line| line.new);
        if let Some(Err(error)) = slot.ready() {
            self.preview_error(&slot.key, error.clone(), cx);
            return;
        }
        if let Some(Ok(_)) = slot.ready() {
            self.show_slot(&slot, anchor);
            if slot.styles(show).is_some() {
                self.prefetch_neighbors(cx);
                cx.notify();
                return;
            }
        } else if !quiet {
            self.preview_loading = true;
        }
        let ready = slot.ready().is_some();
        let read_slot = slot.clone();
        let read_task = (!ready).then(|| {
            cx.background_executor()
                .spawn(async move { read_slot.load(&repo, &file) })
        });
        cx.spawn(async move |view, cx| {
            if let Some(task) = read_task {
                let result = task.await;
                let current = view
                    .update(cx, |this, cx| {
                        if epoch != this.epoch || request != this.preview_request {
                            return false;
                        }
                        this.preview_loading = false;
                        if let Err(error) = result {
                            this.preview_error(&slot.key, error, cx);
                            return false;
                        }
                        this.show_slot(&slot, anchor);
                        this.cache.trim();
                        this.prefetch_neighbors(cx);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
            }
            // Plain content is already on screen. Syntax never delays first paint.
            let syntax_slot = slot.clone();
            cx.background_executor()
                .spawn(async move {
                    syntax_slot.highlight_range(show, 0..48);
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if epoch != this.epoch || request != this.preview_request {
                    return;
                }
                this.syntax = slot.styles(this.show_deletions).unwrap_or_default();
                this.cache.trim();
                cx.notify();
            });
        })
        .detach();
        self.prefetch_neighbors(cx);
    }

    fn preview_error(&mut self, key: &Key, error: String, cx: &mut Context<Self>) {
        self.diff = Arc::new(Diff {
            notice: Some(error),
            ..Diff::default()
        });
        self.syntax = Arc::default();
        self.changes = Arc::default();
        self.current_slot = None;
        self.preview_loading = false;
        self.preview_target = None;
        self.cache.remove(&key.path);
        cx.notify();
    }

    fn show_slot(&mut self, slot: &Arc<Slot>, anchor: Option<usize>) {
        let Some(Ok(pair)) = slot.ready() else {
            return;
        };
        self.current_slot = Some(slot.clone());
        let next = pair.mode(self.show_deletions);
        if !Arc::ptr_eq(&next, &self.diff) {
            self.text_selection = None;
        }
        self.diff = next;
        self.changes = Arc::new(scrollbar::changed_lines(&self.diff.lines));
        self.syntax = slot.styles(self.show_deletions).unwrap_or_default();
        self.preview_loading = false;
        self.preview_cursor = anchor
            .and_then(|number| {
                self.diff
                    .lines
                    .iter()
                    .position(|line| line.new == Some(number))
            })
            .unwrap_or_else(|| {
                self.preview_cursor
                    .min(self.diff.lines.len().saturating_sub(1))
            });
        self.change_index = None;
    }

    fn ensure_syntax(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let Some(slot) = self.current_slot.clone() else {
            return;
        };
        let show = self.show_deletions;
        for chunk in range.start / 48..range.end.min(self.diff.lines.len()).div_ceil(48) {
            if self.syntax_pending.len() >= 2 {
                break;
            }
            let range = chunk * 48..(chunk + 1) * 48;
            let key = (slot.key.clone(), show, chunk);
            if slot.range_ready(show, range.clone()) || self.syntax_pending.contains(&key) {
                continue;
            }
            self.syntax_pending.insert(key.clone());
            let slot = slot.clone();
            let work = slot.clone();
            let epoch = self.epoch;
            let task = cx.background_executor().spawn(async move {
                work.highlight_range(show, range);
            });
            cx.spawn(async move |view, cx| {
                task.await;
                let _ = view.update(cx, |this, cx| {
                    if epoch != this.epoch {
                        return;
                    }
                    this.syntax_pending.remove(&key);
                    if this
                        .preview_target
                        .as_ref()
                        .is_some_and(|(key, _)| *key == slot.key)
                    {
                        this.syntax = slot.styles(this.show_deletions).unwrap_or_default();
                    }
                    this.cache.trim();
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn prefetch_neighbors(&mut self, cx: &mut Context<Self>) {
        if self.prefetching.len() >= 2 || self.opening {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let cursor = self.tree_cursor.unwrap_or(0).min(self.tree.len());
        let as_file = |row: &TreeRow| match &row.kind {
            RowKind::File(file) => Some(file.clone()),
            _ => None,
        };
        // Collect at most two paths; walking a large tree must not clone every file.
        let candidates: Vec<_> = if self.selected.is_some() {
            self.tree
                .iter()
                .skip(cursor + 1)
                .find_map(as_file)
                .into_iter()
                .chain(self.tree[..cursor].iter().rev().find_map(as_file))
                .collect()
        } else {
            self.tree
                .iter()
                .skip(cursor)
                .filter_map(as_file)
                .take(2)
                .collect()
        };
        for candidate in candidates {
            if self.prefetching.len() >= 2 {
                break;
            }
            let path = &candidate.path;
            let Some(file) = self
                .snapshot
                .files
                .iter()
                .find(|file| &file.path == path)
                .cloned()
            else {
                continue;
            };
            let Some(version) = self.snapshot.versions.get(&file.path).copied() else {
                continue;
            };
            let key = Key {
                path: file.path.clone(),
                version,
            };
            if self.prefetching.contains(&key) {
                continue;
            }
            let slot = self.cache.slot(key.clone());
            if slot.range_ready(false, 0..48) {
                continue;
            }
            self.prefetching.insert(key.clone());
            let epoch = self.epoch;
            let repo = repo.clone();
            let task = cx.background_executor().spawn(async move {
                if slot.load(&repo, &file).is_ok() {
                    slot.highlight_range(false, 0..48);
                }
            });
            cx.spawn(async move |view, cx| {
                task.await;
                let _ = view.update(cx, |this, _| {
                    if this.epoch == epoch {
                        this.prefetching.remove(&key);
                        this.cache.trim();
                    }
                });
            })
            .detach();
        }
    }

    fn select_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree.get(index).is_some_and(TreeRow::navigable) {
            return;
        }
        self.tree_cursor = Some(index);
        let selected = match &self.tree[index].kind {
            RowKind::File(file) => Some(file.path.clone()),
            _ => None,
        };
        if self.selected != selected {
            self.selected = selected;
            self.reset_preview_position();
            self.load_diff(false, cx);
        }
        self.focus_left = true;
        self.focus.focus(window);
        cx.notify();
    }

    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tree_cursor else {
            return;
        };
        if !self.focus_left {
            return;
        }
        if let RowKind::Directory { path, expanded } = &self.tree[index].kind {
            let path = path.clone();
            if *expanded {
                self.collapsed.insert(path);
            } else {
                self.collapsed.remove(&path);
            }
            self.rebuild_tree(false, cx);
            self.focus.focus(window);
            cx.notify();
        }
    }

    fn tree_arrow(&mut self, expand: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_left {
            return;
        }
        let Some(index) = self.tree_cursor else {
            return;
        };
        if let RowKind::Directory { expanded, .. } = &self.tree[index].kind
            && *expanded != expand
        {
            self.activate_row(window, cx);
            return;
        }
        if expand {
            if matches!(self.tree[index].kind, RowKind::File(_)) {
                self.focus_left = false;
                cx.notify();
            } else {
                self.navigate(1, window, cx);
            }
        } else {
            let depth = self.tree[index].depth;
            if let Some(parent) = (0..index).rev().find(|i| {
                self.tree[*i].depth < depth
                    && matches!(self.tree[*i].kind, RowKind::Directory { .. })
            }) {
                self.select_row(parent, window, cx);
                self.files_scroll
                    .scroll_to_item(parent, ScrollStrategy::Top);
            }
        }
    }

    fn navigate(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_left {
            if self.diff.lines.is_empty() {
                return;
            }
            self.preview_cursor = self
                .preview_cursor
                .saturating_add_signed(offset)
                .min(self.diff.lines.len() - 1);
            self.diff_scroll
                .scroll_to_item(self.preview_cursor, ScrollStrategy::Top);
            cx.notify();
            return;
        }
        let rows: Vec<_> = self
            .tree
            .iter()
            .enumerate()
            .filter(|(_, row)| row.navigable())
            .map(|(i, _)| i)
            .collect();
        if rows.is_empty() {
            return;
        }
        let current = rows
            .iter()
            .position(|i| Some(*i) == self.tree_cursor)
            .unwrap_or(0);
        let index = rows[current.saturating_add_signed(offset).min(rows.len() - 1)];
        self.files_scroll.scroll_to_item(index, ScrollStrategy::Top);
        self.select_row(index, window, cx);
    }

    fn page(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let handle = if self.focus_left {
            &self.files_scroll
        } else {
            &self.diff_scroll
        };
        let height = handle.0.borrow().base_handle.bounds().size.height;
        let rows = (f32::from(height)
            / if self.focus_left {
                TREE_HEIGHT
            } else {
                LINE_HEIGHT
            })
        .max(1.) as isize;
        self.navigate(direction * rows, window, cx);
    }

    fn toggle_deletions(&mut self, cx: &mut Context<Self>) {
        self.text_selection = None;
        self.text_layouts.clear();
        self.show_deletions = !self.show_deletions;
        self.load_diff(false, cx);
        cx.notify();
    }

    fn next_change(&mut self, cx: &mut Context<Self>) {
        let blocks = preview::change_blocks(&self.diff.lines);
        if blocks.is_empty() {
            self.notice = "No changed lines in this file".into();
        } else {
            let index = self.change_index.map_or(0, |i| (i + 1) % blocks.len());
            self.change_index = Some(index);
            self.preview_cursor = blocks[index];
            self.diff_scroll
                .scroll_to_item_strict(self.preview_cursor, ScrollStrategy::Center);
            self.notice = format!("Change {}/{} · Space next", index + 1, blocks.len());
        }
        cx.notify();
    }

    fn mutate(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.opening {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let file = if all {
            None
        } else {
            self.snapshot
                .files
                .iter()
                .find(|file| Some(&file.path) == self.selected.as_ref())
                .cloned()
        };
        if !all && file.is_none() {
            return;
        }
        let unstage = !all && file.as_ref().is_some_and(|f| f.staged() && !f.unstaged());
        self.start_operation(
            if unstage {
                "Unstaging changes…"
            } else {
                "Staging changes…"
            },
            cx,
        );
        let task = cx.background_executor().spawn(async move {
            if unstage {
                repo.unstage(file.as_ref())?;
            } else {
                repo.stage(file.as_ref())?;
            }
            repo.snapshot()
        });
        self.finish_operation(task, "Index updated", false, window, cx);
    }

    fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.opening {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let message = self.commit_input.read(cx).value().to_string();
        if message.trim().is_empty() {
            self.commit_open = true;
            cx.notify();
            self.commit_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if !self.snapshot.files.iter().any(ChangedFile::staged) {
            return;
        }
        self.start_operation("Committing staged changes…", cx);
        let task = cx.background_executor().spawn(async move {
            repo.commit(&message)?;
            repo.snapshot()
        });
        self.finish_operation(task, "Commit created", true, window, cx);
    }

    fn start_operation(&mut self, message: &str, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = false;
        self.notice = message.into();
        // Invalidate reads started before the index mutation.
        self.snapshot_request += 1;
        self.preview_request += 1;
        self.preview_target = None;
        self.tree_request += 1;
        self.tree_task = None;
        self.refreshing = false;
        cx.notify();
    }

    fn finish_operation(
        &mut self,
        task: Task<anyhow::Result<Snapshot>>,
        message: &'static str,
        clear_message: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(snapshot) => {
                        this.snapshot = snapshot;
                        this.notice = message.into();
                        if clear_message {
                            this.commit_input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        this.rebuild_tree(false, cx);
                    }
                    Err(error) => {
                        this.show_error(error, cx);
                        this.refresh(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_error(&mut self, error: impl std::fmt::Display, cx: &mut Context<Self>) {
        self.notice = error.to_string();
        self.error = true;
        cx.notify();
    }

    fn copy_selection(&mut self, _: &CopySelection, _: &mut Window, cx: &mut Context<Self>) {
        let text = if self.focus_left {
            self.selected
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            self.text_selection
                .map(|selection| selection.text(&self.diff.lines))
                .unwrap_or_default()
        };
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.notice = "Selection copied".into();
            cx.notify();
        }
    }

    fn select_all_text(&mut self, _: &SelectAllText, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.diff.lines.last() else {
            return;
        };
        self.focus_left = false;
        self.text_selection = Some(TextSelection {
            anchor: TextPoint { row: 0, byte: 0 },
            head: TextPoint {
                row: self.diff.lines.len() - 1,
                byte: last.text.len(),
            },
        });
        self.focus.focus(window);
        cx.notify();
    }

    fn selection_point(&self, row: usize, position: Point<Pixels>) -> TextPoint {
        let Some(line) = self.diff.lines.get(row) else {
            return TextPoint::default();
        };
        let byte = self
            .text_layouts
            .get(&row)
            .map(|layout| {
                let p = point(position.x, layout.bounds().center().y);
                layout
                    .index_for_position(p)
                    .unwrap_or_else(|nearest| nearest)
            })
            .unwrap_or_else(|| {
                if position.x < self.diff_scroll.0.borrow().base_handle.bounds().center().x {
                    0
                } else {
                    line.text.len()
                }
            });
        TextPoint {
            row,
            byte: text_selection::boundary(&line.text, byte),
        }
    }

    fn start_selection(
        &mut self,
        row: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let point = self.selection_point(row, event.position);
        self.preview_cursor = row;
        self.focus_left = false;
        self.focus.focus(window);
        self.selection_pointer = event.position;
        if event.modifiers.shift {
            let selection = self
                .text_selection
                .get_or_insert_with(|| TextSelection::new(point));
            selection.head = point;
        } else if event.click_count == 2 {
            let range = text_selection::word(&self.diff.lines[row].text, point.byte);
            self.text_selection = Some(TextSelection {
                anchor: TextPoint {
                    row,
                    byte: range.start,
                },
                head: TextPoint {
                    row,
                    byte: range.end,
                },
            });
        } else if event.click_count >= 3 {
            self.text_selection = Some(TextSelection {
                anchor: TextPoint { row, byte: 0 },
                head: TextPoint {
                    row,
                    byte: self.diff.lines[row].text.len(),
                },
            });
        } else {
            self.text_selection = Some(TextSelection::new(point));
        }
        self.preview_selecting = true;
        cx.stop_propagation();
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(32))
                    .await;
                let active = view
                    .update_in(cx, |this, _, cx| {
                        if !this.preview_selecting {
                            return false;
                        }
                        let bounds = this.diff_scroll.0.borrow().base_handle.bounds();
                        if this.selection_pointer.y < bounds.top()
                            || this.selection_pointer.y > bounds.bottom()
                        {
                            this.drag_selection(this.selection_pointer, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !active {
                    break;
                }
            }
        })
        .detach();
    }

    fn drag_selection(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.diff.lines.is_empty() {
            return;
        }
        let state = self.diff_scroll.0.borrow();
        let bounds = state.base_handle.bounds();
        let mut offset = state.base_handle.offset();
        if position.y < bounds.top() {
            offset.y += px(LINE_HEIGHT);
        } else if position.y > bounds.bottom() {
            offset.y -= px(LINE_HEIGHT);
        }
        state.base_handle.set_offset(offset);
        let y = position.y.clamp(bounds.top(), bounds.bottom() - px(1.));
        let row = (f32::from(y - bounds.top() - state.base_handle.offset().y) / LINE_HEIGHT)
            .floor()
            .max(0.) as usize;
        drop(state);
        let point = self.selection_point(row.min(self.diff.lines.len() - 1), position);
        if let Some(selection) = &mut self.text_selection {
            selection.head = point;
        }
        self.preview_cursor = point.row;
        cx.notify();
    }

    fn extend_selection(&mut self, vertical: isize, horizontal: isize, cx: &mut Context<Self>) {
        if self.focus_left || self.diff.lines.is_empty() {
            return;
        }
        let mut selection = self.text_selection.unwrap_or_else(|| {
            TextSelection::new(TextPoint {
                row: self.preview_cursor,
                byte: 0,
            })
        });
        selection.head =
            text_selection::step(selection.head, &self.diff.lines, vertical, horizontal);
        self.preview_cursor = selection.head.row;
        self.text_selection = Some(selection);
        self.diff_scroll
            .scroll_to_item(self.preview_cursor, ScrollStrategy::Top);
        cx.notify();
    }

    fn copy_diff(&mut self, _: &CopyDiff, _: &mut Window, cx: &mut Context<Self>) {
        if !self.diff.raw.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.diff.raw.clone()));
            self.notice = "Diff copied to clipboard".into();
            self.error = false;
            cx.notify();
        }
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal_visible = !self.terminal_visible;
        self.activate_project_terminal(window, cx);
        if !self.terminal_visible {
            self.focus.focus(window);
        }
        cx.notify();
    }

    fn activate_project_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.terminal.take() {
            previous.update(cx, |view, cx| view.set_visible(false, window, cx));
        }
        let path = self
            .repo
            .as_ref()
            .map(|repo| repo.root.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if self.terminal_visible && !self.terminals.contains_key(&path) {
            let terminal =
                cx.new(|cx| crate::terminal_view::TerminalView::new(path.clone(), window, cx));
            self.terminals.insert(path.clone(), terminal);
        }
        self.terminal = self.terminals.get(&path).cloned();
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |view, cx| {
                view.set_visible(self.terminal_visible, window, cx)
            });
        }
    }

    fn terminal_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self
            .terminal_width
            .min((f32::from(window.viewport_size().width) - 600.).max(280.));
        div()
            .flex()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .child(
                div()
                    .id("terminal-resize")
                    .w(px(4.))
                    .h_full()
                    .cursor_col_resize()
                    .bg(rgb(BORDER))
                    .hover(|el| el.bg(rgb(ACCENT)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.terminal_dragging = true;
                            cx.stop_propagation();
                        }),
                    ),
            )
            .when_some(self.terminal.clone(), |el, terminal| el.child(terminal))
    }

    fn drag_ui(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_dragging {
            self.terminal_width = (f32::from(window.viewport_size().width - event.position.x))
                .clamp(
                    280.,
                    (f32::from(window.viewport_size().width) - 600.).max(280.),
                );
            cx.notify();
        } else if self.preview_selecting && event.dragging() {
            self.selection_pointer = event.position;
            self.drag_selection(event.position, cx);
        } else {
            self.drag_overview(event, window, cx);
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .repo
            .as_ref()
            .and_then(|repo| repo.root.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "give".into());
        TitleBar::new()
            .bg(rgb(0x151e28))
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(12.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .when(self.repo.is_some(), |el| {
                        el.child(div().text_color(rgb(BORDER)).child("/"))
                            .child(ui_icon("branch", 13., MUTED))
                            .child(
                                div()
                                    .text_color(rgb(MUTED))
                                    .child(self.snapshot.branch.clone()),
                            )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pr_3()
                    .child(
                        div()
                            .id("toggle-terminal")
                            .w(px(30.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .bg(rgb(if self.terminal_visible {
                                SURFACE
                            } else {
                                0x151e28
                            }))
                            .hover(|el| el.bg(rgb(SURFACE)))
                            .tooltip(|window, cx| {
                                Tooltip::new("Toggle right terminal · ⌘B").build(window, cx)
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_terminal(&ToggleTerminal, window, cx)
                            }))
                            .child(ui_icon(
                                "panel-right",
                                17.,
                                if self.terminal_visible { ACCENT } else { MUTED },
                            )),
                    )
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .xsmall()
                            .label("Refresh")
                            .disabled(self.repo.is_none() || self.busy || self.opening)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("open")
                            .ghost()
                            .xsmall()
                            .label("Open project")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_picker(&OpenRepository, window, cx)
                            })),
                    ),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let staged = self
            .snapshot
            .files
            .iter()
            .filter(|file| file.staged())
            .count();
        let working = self
            .snapshot
            .files
            .iter()
            .filter(|file| file.unstaged())
            .count();
        let can_commit = staged > 0
            && !self.busy
            && !self.opening
            && !self.commit_input.read(cx).value().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .w(px(270.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child("CHANGES")
                            .child(
                                div()
                                    .text_color(rgb(ACCENT))
                                    .child(self.snapshot.files.len().to_string()),
                            ),
                    )
                    .child(
                        Button::new("all")
                            .ghost()
                            .xsmall()
                            .label("Stage all")
                            .disabled(self.busy || self.opening || working == 0)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.mutate(true, window, cx)),
                            ),
                    ),
            )
            .child(
                div().px_2().pb_2().child(
                    Input::new(&self.filter_input)
                        .small()
                        .appearance(false)
                        .prefix(ui_icon("search", 13., MUTED)),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(!self.tree.is_empty(), |el| {
                        el.child(
                            uniform_list(
                                "files",
                                self.tree.len(),
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|index| {
                                            let row = &this.tree[index];
                                            let selected = this.tree_cursor == Some(index);
                                            let navigable = row.navigable();
                                            let (icon, color, badge) = match &row.kind {
                                                RowKind::Directory { expanded, .. } => (
                                                    if *expanded {
                                                        "folder-open"
                                                    } else {
                                                        "folder"
                                                    },
                                                    0x8aa6bc,
                                                    String::new(),
                                                ),
                                                RowKind::File(file) => {
                                                    let ext = file
                                                        .path
                                                        .extension()
                                                        .unwrap_or_default()
                                                        .to_string_lossy();
                                                    let icon = match ext.as_ref() {
                                                        "rs" | "go" | "ts" | "js" | "py" => "code",
                                                        "md" => "markdown",
                                                        "toml" | "json" | "yaml" | "yml" => {
                                                            "config"
                                                        }
                                                        _ => "file",
                                                    };
                                                    let color = match ext.as_ref() {
                                                        "rs" => 0xc79679,
                                                        "go" => 0x86b3c5,
                                                        "md" => 0x86afd1,
                                                        "toml" | "json" | "yaml" | "yml" => {
                                                            0xa79bc7
                                                        }
                                                        _ => MUTED,
                                                    };
                                                    let badge = if file.untracked() {
                                                        "U".into()
                                                    } else if file.index == 'D'
                                                        || file.worktree == 'D'
                                                    {
                                                        "D".into()
                                                    } else if file.staged() && !file.unstaged() {
                                                        "●".into()
                                                    } else {
                                                        "M".into()
                                                    };
                                                    (icon, color, badge)
                                                }
                                                _ => ("file", MUTED, String::new()),
                                            };
                                            div()
                                                .id(index)
                                                .h(px(TREE_HEIGHT))
                                                .w_full()
                                                .pl_3()
                                                .pr_3()
                                                .flex()
                                                .items_center()
                                                .bg(rgb(if selected { 0x2b4057 } else { PANEL }))
                                                .when(navigable, |el| {
                                                    el.cursor_pointer()
                                                        .hover(|el| el.bg(rgb(SURFACE)))
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.select_row(index, window, cx);
                                                                if matches!(
                                                                    this.tree[index].kind,
                                                                    RowKind::Directory { .. }
                                                                ) {
                                                                    this.activate_row(window, cx);
                                                                }
                                                            },
                                                        ))
                                                })
                                                .when(matches!(row.kind, RowKind::Divider), |el| {
                                                    el.child(
                                                        div().w_full().h(px(1.)).bg(rgb(BORDER)),
                                                    )
                                                })
                                                .when(!matches!(row.kind, RowKind::Divider), |el| {
                                                    el.when(navigable, |el| {
                                                        el.child(div().h_full().flex().children(
                                                            (0..row.depth).map(|_| {
                                                                div()
                                                                    .w(px(16.))
                                                                    .h_full()
                                                                    .border_l_1()
                                                                    .border_color(rgb(0x304052))
                                                            }),
                                                        ))
                                                        .child(
                                                            div().w(px(14.)).flex_shrink_0().when(
                                                                matches!(
                                                                    row.kind,
                                                                    RowKind::Directory { .. }
                                                                ),
                                                                |el| {
                                                                    el.child(ui_icon(
                                                                        if matches!(
                                                                            row.kind,
                                                                            RowKind::Directory {
                                                                                expanded: true,
                                                                                ..
                                                                            }
                                                                        ) {
                                                                            "chevron-down"
                                                                        } else {
                                                                            "chevron-right"
                                                                        },
                                                                        10.,
                                                                        MUTED,
                                                                    ))
                                                                },
                                                            ),
                                                        )
                                                        .child(ui_icon(icon, 15., color))
                                                    })
                                                    .child(
                                                        div()
                                                            .ml_2()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .truncate()
                                                            .text_size(px(if navigable {
                                                                13.
                                                            } else {
                                                                10.
                                                            }))
                                                            .text_color(rgb(if navigable {
                                                                TEXT
                                                            } else {
                                                                MUTED
                                                            }))
                                                            .child(row.label.clone()),
                                                    )
                                                    .when(!badge.is_empty(), |el| {
                                                        el.child(
                                                            div()
                                                                .ml_2()
                                                                .text_size(px(10.))
                                                                .text_color(rgb(if row.untracked {
                                                                    GREEN
                                                                } else {
                                                                    0xc5b28a
                                                                }))
                                                                .child(badge),
                                                        )
                                                    })
                                                })
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(self.files_scroll.clone())
                            .h_full(),
                        )
                    })
                    .when(self.tree.is_empty(), |el| {
                        el.child(div().p_4().text_color(rgb(MUTED)).child("No changed files"))
                    }),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .p_2()
                    .child(
                        Button::new("commit-toggle")
                            .ghost()
                            .small()
                            .w_full()
                            .label(format!("Commit · {staged} staged"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.commit_open = !this.commit_open;
                                cx.notify();
                            })),
                    )
                    .when(self.commit_open, |el| {
                        el.child(
                            div()
                                .pt_2()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(Input::new(&self.commit_input).small().disabled(self.busy))
                                .child(
                                    Button::new("commit")
                                        .primary()
                                        .small()
                                        .label(if self.busy {
                                            "Working…"
                                        } else {
                                            "Commit staged changes"
                                        })
                                        .disabled(!can_commit)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.commit(&Commit, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
    }

    fn preview_bottom_padding(&self) -> usize {
        if self.change_index.is_none() {
            return 0;
        }
        let height = self.diff_scroll.0.borrow().base_handle.bounds().size.height;
        (f32::from(height) / LINE_HEIGHT / 2.).max(1.) as usize
    }

    fn drag_overview(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(grab) = self.scrollbar_drag else {
            return;
        };
        let bounds = self.scrollbar_bounds.get();
        let y = f32::from(event.position.y - bounds.origin.y);
        let height = f32::from(bounds.size.height);
        let state = self.diff_scroll.0.borrow();
        let visible = f32::from(state.base_handle.bounds().size.height) / LINE_HEIGHT;
        let total = self.diff.lines.len() + self.preview_bottom_padding();
        if let Some((_, size)) = scrollbar::thumb_with_minimum(total, visible, 0., height, 12.) {
            let percent = ((y - grab) / (height - size).max(1.)).clamp(0., 1.);
            let offset = percent * (total as f32 - visible).max(0.) * LINE_HEIGHT;
            let x = state.base_handle.offset().x;
            state.base_handle.set_offset(point(x, -px(offset)));
        }
        cx.notify();
    }

    fn overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds_cell = self.scrollbar_bounds.clone();
        let scroll = self.diff_scroll.clone();
        let changes = self.changes.clone();
        let total = self.diff.lines.len() + self.preview_bottom_padding();
        div()
            .id("change-overview")
            .w(px(14.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(0x17222e))
            .cursor_pointer()
            .border_l_1()
            .border_color(rgb(BORDER))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let bounds = this.scrollbar_bounds.get();
                    let y = f32::from(event.position.y - bounds.origin.y);
                    let height = f32::from(bounds.size.height);
                    let state = this.diff_scroll.0.borrow();
                    let visible = f32::from(state.base_handle.bounds().size.height) / LINE_HEIGHT;
                    let offset = -f32::from(state.base_handle.offset().y) / LINE_HEIGHT;
                    let total = this.diff.lines.len() + this.preview_bottom_padding();
                    let thumb = scrollbar::thumb_with_minimum(total, visible, offset, height, 12.);
                    drop(state);
                    if let Some(line) = scrollbar::nearest_change(&this.changes, total, height, y) {
                        this.preview_cursor = line;
                        this.change_index = preview::change_blocks(&this.diff.lines)
                            .iter()
                            .rposition(|start| *start <= line);
                        this.diff_scroll
                            .scroll_to_item_strict(line, ScrollStrategy::Center);
                        this.focus_left = false;
                        this.focus.focus(window);
                    } else if let Some((start, size)) = thumb
                        && y >= start
                        && y <= start + size
                    {
                        this.scrollbar_drag = Some(y - start);
                    } else if total > 0 {
                        let line = ((y / height.max(1.)).clamp(0., 1.) * total as f32) as usize;
                        this.diff_scroll.scroll_to_item_strict(
                            line.min(this.diff.lines.len().saturating_sub(1)),
                            ScrollStrategy::Center,
                        );
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.scrollbar_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.scrollbar_drag = None),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        bounds_cell.set(bounds);
                    },
                    move |bounds, _, window, _| {
                        let height = f32::from(bounds.size.height);
                        let state = scroll.0.borrow();
                        let visible =
                            f32::from(state.base_handle.bounds().size.height) / LINE_HEIGHT;
                        let offset = -f32::from(state.base_handle.offset().y) / LINE_HEIGHT;
                        if let Some((start, size)) =
                            scrollbar::thumb_with_minimum(total, visible, offset, height, 12.)
                        {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left() + px(3.), bounds.top() + px(start)),
                                    gpui::size(px(7.), px(size)),
                                ),
                                rgb(0x435a73),
                            ));
                        }
                        // Merge marks that map to the same pixels; a large changed block
                        // becomes one strip rather than thousands of paint operations.
                        let mut run: Option<(f32, f32)> = None;
                        let paint = |start: f32, end: f32, window: &mut Window| {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left() + px(2.), bounds.top() + px(start)),
                                    gpui::size(
                                        px(9.),
                                        px((end - start + 2.).min(height - start).max(1.)),
                                    ),
                                ),
                                rgb(GREEN),
                            ));
                        };
                        for line in changes.iter() {
                            let y = scrollbar::marker_position(*line, total, height);
                            match run {
                                Some((start, end)) if y <= end + 2. => run = Some((start, y)),
                                Some((start, end)) => {
                                    paint(start, end, window);
                                    run = Some((y, y));
                                }
                                None => run = Some((y, y)),
                            }
                        }
                        if let Some((start, end)) = run {
                            paint(start, end, window);
                        }
                    },
                )
                .size_full(),
            )
    }

    fn preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self
            .selected
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = self
            .selected
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Review".into());
        let unstage = self
            .snapshot
            .files
            .iter()
            .find(|f| Some(&f.path) == self.selected.as_ref())
            .is_some_and(|f| f.staged() && !f.unstaged());
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(BACKGROUND))
            .child(
                div()
                    .h(px(35.))
                    .flex_shrink_0()
                    .flex()
                    .bg(rgb(PANEL))
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .px_4()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .bg(rgb(BACKGROUND))
                            .border_r_1()
                            .border_color(rgb(BORDER))
                            .border_b_2()
                            .text_size(px(13.))
                            .child(ui_icon("code", 14., ACCENT))
                            .child(name),
                    ),
            )
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .bg(rgb(0x1e2c3b))
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(path),
                    )
                    .when(self.selected.is_some(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(GREEN))
                                        .child(format!("+{}", self.diff.additions)),
                                )
                                .child(
                                    div()
                                        .mr_2()
                                        .text_size(px(11.))
                                        .text_color(rgb(RED))
                                        .child(format!("−{}", self.diff.deletions)),
                                )
                                .child(
                                    Button::new("deletions")
                                        .ghost()
                                        .xsmall()
                                        .label(if self.show_deletions {
                                            if self.terminal_visible {
                                                "− D"
                                            } else {
                                                "Hide deletions  D"
                                            }
                                        } else {
                                            if self.terminal_visible {
                                                "+ D"
                                            } else {
                                                "Show deletions  D"
                                            }
                                        })
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.toggle_deletions(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("next-change")
                                        .ghost()
                                        .xsmall()
                                        .label("Next ↓")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.next_change(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("stage-file")
                                        .ghost()
                                        .xsmall()
                                        .label(if unstage { "Unstage" } else { "Stage" })
                                        .disabled(self.busy || self.opening)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.mutate(false, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .when(!self.preview_loading && self.selected.is_none(), |el| {
                el.child(empty_state(
                    "Select a changed file",
                    "Browse the tree to review the full file and its changes.",
                    false,
                ))
            })
            .when(self.selected.is_some(), |el| {
                el.when_some(self.diff.notice.clone(), |el, notice| {
                    el.child(
                        div()
                            .px_4()
                            .py_2()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(notice),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .child(
                            uniform_list(
                                "diff",
                                self.diff.lines.len() + self.preview_bottom_padding(),
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    this.ensure_syntax(range.clone(), cx);
                                    this.text_layouts.retain(|row, _| range.contains(row));
                                    range
                                        .map(|index| {
                                            let Some(line) = this.diff.lines.get(index) else {
                                                return div()
                                                    .id(index)
                                                    .h(px(LINE_HEIGHT))
                                                    .min_w_full();
                                            };
                                            let bg = match line.kind {
                                                LineKind::Added => 0x223d34,
                                                LineKind::Removed => 0x452c38,
                                                _ if !this.focus_left
                                                    && this.preview_cursor == index =>
                                                {
                                                    0x223347
                                                }
                                                _ => BACKGROUND,
                                            };
                                            let selected =
                                                this.text_selection.and_then(|selection| {
                                                    selection.range(index, &line.text)
                                                });
                                            let fallback = vec![preview::Highlight {
                                                range: 0..line.text.len(),
                                                color: TEXT,
                                            }];
                                            let syntax = this
                                                .syntax
                                                .get(index)
                                                .and_then(|row| row.get())
                                                .unwrap_or(&fallback);
                                            let mut spans = Vec::new();
                                            for span in syntax {
                                                let mut edges =
                                                    vec![span.range.start, span.range.end];
                                                if let Some(selection) = &selected {
                                                    edges.extend([
                                                        selection.start.clamp(
                                                            span.range.start,
                                                            span.range.end,
                                                        ),
                                                        selection.end.clamp(
                                                            span.range.start,
                                                            span.range.end,
                                                        ),
                                                    ]);
                                                }
                                                edges.sort_unstable();
                                                edges.dedup();
                                                for edge in edges.windows(2) {
                                                    let range = edge[0]..edge[1];
                                                    spans.push((
                                                        range.clone(),
                                                        HighlightStyle {
                                                            color: Some(rgb(span.color).into()),
                                                            background_color: selected
                                                                .as_ref()
                                                                .filter(|selection| {
                                                                    selection.contains(&range.start)
                                                                })
                                                                .map(|_| rgb(0x3c5c7e).into()),
                                                            ..Default::default()
                                                        },
                                                    ));
                                                }
                                            }
                                            let code = StyledText::new(line.text.clone())
                                                .with_highlights(spans);
                                            this.text_layouts.insert(index, code.layout().clone());
                                            div()
                                                .id(index)
                                                .h(px(LINE_HEIGHT))
                                                .min_w_full()
                                                .flex()
                                                .items_center()
                                                .bg(rgb(bg))
                                                .font_family("Menlo")
                                                .text_size(px(15.))
                                                .line_height(px(LINE_HEIGHT))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, event, window, cx| {
                                                        this.start_selection(
                                                            index, event, window, cx,
                                                        )
                                                    }),
                                                )
                                                .when(this.show_deletions, |el| {
                                                    el.child(
                                                        div()
                                                            .w(px(54.))
                                                            .flex_shrink_0()
                                                            .pr_2()
                                                            .text_right()
                                                            .text_color(rgb(MUTED))
                                                            .child(
                                                                line.old
                                                                    .map(|n| n.to_string())
                                                                    .unwrap_or_default(),
                                                            ),
                                                    )
                                                })
                                                .child(
                                                    div()
                                                        .w(px(62.))
                                                        .flex_shrink_0()
                                                        .pr_4()
                                                        .text_right()
                                                        .text_color(rgb(
                                                            if !this.focus_left
                                                                && this.preview_cursor == index
                                                            {
                                                                ACCENT
                                                            } else {
                                                                MUTED
                                                            },
                                                        ))
                                                        .child(
                                                            line.new
                                                                .map(|n| n.to_string())
                                                                .unwrap_or_default(),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .w(px(16.))
                                                        .flex_shrink_0()
                                                        .text_color(rgb(match line.kind {
                                                            LineKind::Added => GREEN,
                                                            LineKind::Removed => RED,
                                                            _ => MUTED,
                                                        }))
                                                        .child(match line.kind {
                                                            LineKind::Added => "+",
                                                            LineKind::Removed => "−",
                                                            _ => "",
                                                        }),
                                                )
                                                .child(div().whitespace_nowrap().pr_6().child(code))
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .with_horizontal_sizing_behavior(
                                ListHorizontalSizingBehavior::Unconstrained,
                            )
                            .track_scroll(self.diff_scroll.clone())
                            .flex_1()
                            .min_w_0()
                            .min_h_0(),
                        )
                        .child(self.overview(cx)),
                )
            })
    }

    fn publish_selection(&mut self, window: &Window) {
        let changed = self
            .accessible_diff
            .as_ref()
            .is_none_or(|diff| !Arc::ptr_eq(diff, &self.diff));
        if changed {
            self.accessibility.set_value(
                self.diff
                    .lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            self.accessible_diff = Some(self.diff.clone());
        }
        if changed || self.accessible_selection != self.text_selection {
            let (text, range) = if let Some(selection) = self.text_selection {
                let (start, _) = selection.ordered();
                let prefix = self
                    .diff
                    .lines
                    .iter()
                    .take(start.row)
                    .map(|line| line.text.encode_utf16().count() + 1)
                    .sum::<usize>()
                    + self.diff.lines.get(start.row).map_or(0, |line| {
                        line.text[..text_selection::boundary(&line.text, start.byte)]
                            .encode_utf16()
                            .count()
                    });
                let text = selection.text(&self.diff.lines);
                let end = prefix + text.encode_utf16().count();
                (text, prefix..end)
            } else {
                (String::new(), 0..0)
            };
            self.accessibility.set_selection(text, range);
            self.accessible_selection = self.text_selection;
        }
        let bounds = self.diff_scroll.0.borrow().base_handle.bounds();
        self.accessibility.publish(
            0,
            "Source preview",
            !self.focus_left && self.focus.is_focused(window),
            self.selected.is_some(),
            bounds,
            window,
        );
    }

    fn welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .bg(rgb(BACKGROUND))
            .child(brand_icon(128.))
            .child(
                div()
                    .text_size(px(36.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Make good changes."),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(rgb(MUTED))
                    .child("A quiet workspace for reviewing what you and your agents build."),
            )
            .child(
                Button::new("welcome-open")
                    .primary()
                    .label(if self.opening {
                        "Opening…"
                    } else {
                        "Open a Git repository"
                    })
                    .disabled(self.opening)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_picker(&OpenRepository, window, cx)
                    })),
            )
            .child(
                div()
                    .mt_3()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("Review a diff  ·  Stage with intention  ·  Commit when ready"),
            )
    }
}

impl Render for Give {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.publish_selection(window);
        div()
            .key_context("Give")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open_picker))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::copy_diff))
            .on_action(cx.listener(Self::copy_selection))
            .on_action(cx.listener(Self::select_all_text))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.extend_selection(0, -1, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.extend_selection(0, 1, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.extend_selection(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.extend_selection(1, 0, cx)))
            .on_mouse_move(cx.listener(Self::drag_ui))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.terminal_dragging = false;
                    this.preview_selecting = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.terminal_dragging = false;
                    this.preview_selecting = false;
                }),
            )
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &NextFile, window, cx| this.navigate(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousFile, window, cx| this.navigate(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &SwitchPane, window, cx| {
                this.focus_left = !this.focus_left;
                this.focus.focus(window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleDeletions, _, cx| this.toggle_deletions(cx)))
            .on_action(cx.listener(|this, _: &NextChange, _, cx| this.next_change(cx)))
            .on_action(
                cx.listener(|this, _: &ActivateRow, window, cx| this.activate_row(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &CollapseRow, window, cx| this.tree_arrow(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ExpandRow, window, cx| this.tree_arrow(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &PageDown, window, cx| this.page(1, window, cx)))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| this.page(-1, window, cx)))
            .on_action(
                cx.listener(|this, _: &FirstRow, window, cx| this.navigate(isize::MIN, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &LastRow, window, cx| this.navigate(isize::MAX, window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .font_family(".SystemUIFont")
            .text_size(px(12.))
            .child(self.toolbar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(self.repo.is_some(), |el| {
                        el.child(self.sidebar(cx)).child(self.preview(cx))
                    })
                    .when(self.repo.is_none(), |el| el.child(self.welcome(cx)))
                    .when(self.terminal_visible, |el| {
                        el.child(self.terminal_panel(window, cx))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .h(px(25.))
                    .px_4()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(if self.error { RED } else { MUTED }))
                            .child(self.notice.clone()),
                    )
                    .child("Tab focus · ↑↓ / J K move · D deletions · Space / Ctrl+N next change"),
            )
    }
}

fn brand_icon(side: f32) -> impl IntoElement {
    img(ImageSource::Resource(Resource::Embedded(
        "give-master.png".into(),
    )))
    .size(px(side))
    .object_fit(ObjectFit::Contain)
}

fn empty_state(title: &'static str, detail: &'static str, icon: bool) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .when(icon, |el| el.child(brand_icon(72.)))
        .child(div().text_size(px(18.)).text_color(rgb(TEXT)).child(title))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(detail),
        )
}

fn ui_icon(name: &'static str, side: f32, color: u32) -> impl IntoElement {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(side))
        .flex_shrink_0()
        .text_color(rgb(color))
}
