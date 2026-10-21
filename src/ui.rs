use crate::{
    ActivateRow, ClosePreview, CollapseRow, Commit, CopyDiff, CopyPath, ExitReview, ExpandRow,
    Find, FirstRow, LastRow, NextChange, NextFile, OpenRepository, PageDown, PageUp, PickerAccept,
    PickerBack, PickerCancel, PickerDown, PickerOpenActions, PickerUp, PreviousFile, Refresh,
    ReviewBranch, SelectAllText, SelectDown, SelectLeft, SelectRight, SelectUp, SwitchPane,
    ToggleDeletions, ToggleHistory, ToggleSidebar, ToggleTerminal,
};
use give::{
    branches,
    git::{ChangedFile, Diff, LineKind, Repository, Snapshot},
    path_reference, preview,
    preview_cache::{Key, PreviewCache, Slot, Syntax},
    recent,
    review::{self, Branch, Review},
    scrollbar,
    search::{self, SourceMatch},
    text_selection::{self, Point as TextPoint, Selection as TextSelection},
    tree::{self, RowKind, TreeRow},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme as _, Disableable, Sizable, TitleBar,
    button::{Button, ButtonVariants, DropdownButton},
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    text::{TextView, TextViewStyle},
    tooltip::Tooltip,
};
use std::{
    cell::{Cell, RefCell},
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
// File-name colors by change kind, as in JetBrains IDEs.
const NAME_MODIFIED: u32 = 0x7fb2ea;
const NAME_ADDED: u32 = 0x8fd3a4;
const NAME_DELETED: u32 = 0x75808c;
const NAME_UNTRACKED: u32 = 0xe08e98;
const LINE_HEIGHT: f32 = 26.;

/// What the left tree lists: only changed paths (the default) or every project file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeMode {
    Changes,
    Catalog,
}

impl TreeMode {
    fn label(self) -> &'static str {
        match self {
            Self::Changes => "CHANGES",
            Self::Catalog => "CATALOG",
        }
    }
}
const TREE_HEIGHT: f32 = 27.;

#[path = "history_view.rs"]
mod history_view;

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
    sidebar_visible: bool,
    sidebar_width: f32,
    sidebar_dragging: bool,
    history: Entity<history_view::HistoryPanel>,
    history_visible: bool,
    history_height: f32,
    history_dragging: bool,
    /// What the open context menu's "Copy path" copies, set by the right-clicked row.
    context_copy: Option<ContextCopy>,
    snapshot: Snapshot,
    tree: Vec<TreeRow>,
    tree_cursor: Option<usize>,
    tree_request: u64,
    tree_task: Option<Task<()>>,
    collapsed: HashSet<PathBuf>,
    tree_mode: TreeMode,
    /// Catalog directories start collapsed; these are open.
    catalog_expanded: HashSet<PathBuf>,
    /// The catalog first opens the directories of changed files, once per project.
    catalog_seeded: bool,
    focus_left: bool,
    commit_open: bool,
    scrollbar_bounds: Rc<Cell<Bounds<Pixels>>>,
    scrollbar_drag: Option<f32>,
    changes: Arc<Vec<usize>>,
    /// The longest preview row; the list measures its horizontal scroll width from it.
    widest_line: usize,
    show_deletions: bool,
    preview_cursor: usize,
    text_selection: Option<TextSelection>,
    find: Entity<crate::find_view::FindPanel>,
    find_matches: Arc<Vec<SourceMatch>>,
    find_current: Option<usize>,
    find_request: u64,
    find_diff: Option<Arc<Diff>>,
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
    /// Open preview tabs in display order; `selected` is the active one.
    tabs: Vec<PathBuf>,
    diff: Arc<Diff>,
    /// Markdown and SVG files open rendered; this shows their source with changes.
    show_source: bool,
    /// The decoded image for an image file, keyed by its version.
    image: Option<(Key, ImageState)>,
    /// Markdown text joined from `diff`, rebuilt when the diff changes.
    markdown: RefCell<Option<(Arc<Diff>, SharedString)>>,
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
    /// The branch under review, from the request until Exit review.
    review_branch: Option<String>,
    review: Option<Arc<Review>>,
    /// Loader step while a review fetches and diffs.
    review_step: Option<&'static str>,
    review_error: Option<String>,
    /// `--review` waits for the repository to open.
    pending_review: Option<String>,
    picker_open: bool,
    picker_mode: BranchPickerMode,
    branch_button_bounds: Rc<Cell<Bounds<Pixels>>>,
    branch_panel_bounds: Rc<Cell<Bounds<Pixels>>>,
    branch_row_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    branch_menu: Option<BranchMenu>,
    branch_menu_focus: FocusHandle,
    picker_request: u64,
    picker_note: Option<String>,
    branches: Vec<Branch>,
    branch_remotes: Vec<String>,
    branches_loading: bool,
    branch_input: Entity<InputState>,
    branch_query: String,
    branch_cursor: usize,
    branch_scroll: UniformListScrollHandle,
    /// Recently opened project roots, newest first.
    recent: Vec<PathBuf>,
    _subscriptions: Vec<Subscription>,
}

enum ImageState {
    Loading,
    Ready(crate::image_view::Decoded),
    Failed(SharedString),
}

/// What the preview shows instead of source lines.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rendered {
    Markdown,
    Image,
}

fn markdown_path(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| {
        matches!(
            ext.to_string_lossy().to_lowercase().as_str(),
            "md" | "markdown" | "mdx"
        )
    })
}

/// A "Copy path" menu item: its label and the clipboard text.
#[derive(Clone)]
struct ContextCopy {
    label: SharedString,
    text: String,
}

/// A branch picker row: a listed branch, or the typed name as given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerRow {
    Branch(usize),
    Remote(usize),
    Typed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchPickerMode {
    Review,
    Switch,
    PullFrom,
    Push,
}

#[derive(Clone, Copy)]
enum BranchAction {
    Checkout,
    Review,
    Merge,
    Pull,
    PullFrom,
    Push,
}

struct BranchMenu {
    branch: Branch,
    actions: Vec<BranchAction>,
    cursor: usize,
}

enum BranchOperation {
    Switch(Branch),
    Fetch,
    Pull,
    PullFrom(Branch),
    Push(String),
    Merge(Branch),
    AbortMerge,
}

impl Give {
    pub fn new(
        initial: Option<PathBuf>,
        review: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files…"));
        let commit_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Describe your changes…")
                .auto_grow(3, 6)
        });
        let branch_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Branch name, local or remote…"));
        let find = cx.new(|cx| crate::find_view::FindPanel::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&find, window, |this, _, event, window, cx| {
                match event {
                    crate::find_view::Event::Changed => this.search_preview(cx),
                    crate::find_view::Event::Next => this.next_match(false, cx),
                    crate::find_view::Event::Previous => this.next_match(true, cx),
                    crate::find_view::Event::Closed => {
                        this.find_request += 1;
                        this.find_matches = Arc::default();
                        this.find_diff = None;
                        this.focus_left = false;
                        this.focus.focus(window);
                    }
                }
                cx.notify();
            }),
            cx.subscribe_in(&filter_input, window, |this, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter = input.read(cx).value().to_string().to_lowercase();
                    this.rebuild_tree(true, cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&commit_input, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(
                &branch_input,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        this.branch_query = input.read(cx).value().trim().to_owned();
                        this.branch_cursor = 0;
                        this.branch_menu = None;
                        this.branch_scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.confirm_branch(window, cx),
                    _ => {}
                },
            ),
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
            terminal_width: 528.,
            terminal_dragging: false,
            sidebar_visible: true,
            sidebar_width: 270.,
            sidebar_dragging: false,
            history: cx.new(history_view::HistoryPanel::new),
            history_visible: false,
            history_height: 300.,
            history_dragging: false,
            context_copy: None,
            snapshot: Snapshot::default(),
            tree: Vec::new(),
            tree_cursor: None,
            tree_request: 0,
            tree_task: None,
            collapsed: HashSet::new(),
            tree_mode: TreeMode::Changes,
            catalog_expanded: HashSet::new(),
            catalog_seeded: false,
            focus_left: true,
            commit_open: false,
            scrollbar_bounds: Rc::new(Cell::new(Bounds::default())),
            scrollbar_drag: None,
            changes: Arc::default(),
            widest_line: 0,
            show_deletions: false,
            preview_cursor: 0,
            text_selection: None,
            find,
            find_matches: Arc::default(),
            find_current: None,
            find_request: 0,
            find_diff: None,
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
            tabs: Vec::new(),
            diff: Arc::default(),
            show_source: false,
            image: None,
            markdown: RefCell::new(None),
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
            review_branch: None,
            review: None,
            review_step: None,
            review_error: None,
            pending_review: review,
            picker_open: false,
            picker_mode: BranchPickerMode::Review,
            branch_button_bounds: Rc::new(Cell::new(Bounds::default())),
            branch_panel_bounds: Rc::new(Cell::new(Bounds::default())),
            branch_row_bounds: Rc::default(),
            branch_menu: None,
            branch_menu_focus: cx.focus_handle(),
            picker_request: 0,
            picker_note: None,
            branches: Vec::new(),
            branch_remotes: Vec::new(),
            branches_loading: false,
            branch_input,
            branch_query: String::new(),
            branch_cursor: 0,
            branch_scroll: UniformListScrollHandle::new(),
            recent: recent::store()
                .map(|file| recent::load(&file))
                .unwrap_or_default(),
            _subscriptions: subscriptions,
        };
        crate::update_dock_menu(&this.recent, cx);
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

    fn catalog(&self) -> bool {
        self.tree_mode == TreeMode::Catalog && self.review_branch.is_none()
    }

    fn set_tree_mode(&mut self, mode: TreeMode, cx: &mut Context<Self>) {
        if self.tree_mode == mode || self.review_branch.is_some() {
            return;
        }
        self.tree_mode = mode;
        self.files_scroll.scroll_to_item(0, ScrollStrategy::Top);
        if mode == TreeMode::Catalog && self.snapshot.catalog.is_empty() {
            // The in-flight poll lacks the file list; this refresh replaces it.
            self.snapshot_request += 1;
            self.refreshing = false;
            self.refresh(cx);
        } else {
            self.rebuild_tree(false, cx);
        }
        cx.notify();
    }

    fn rebuild_tree(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let catalog = self.catalog();
        if catalog && !self.catalog_seeded && !self.snapshot.catalog.is_empty() {
            self.catalog_seeded = true;
            self.catalog_expanded
                .extend(tree::changed_directories(&self.snapshot.files));
        }
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
                        if catalog {
                            this.snapshot.catalog.clone()
                        } else {
                            this.snapshot.files.clone()
                        },
                        if catalog {
                            this.catalog_expanded.clone()
                        } else {
                            this.collapsed.clone()
                        },
                        this.filter.clone(),
                    ))
                })
                .ok()
                .flatten();
            let Some((files, mut folders, filter)) = state else {
                return;
            };
            let (rows, folders) = cx
                .background_executor()
                .spawn(async move {
                    // A catalog still loading must not forget its open folders.
                    if !files.is_empty() {
                        tree::prune_collapsed(&mut folders, &files);
                    }
                    let files: Vec<_> = files
                        .into_iter()
                        .filter(|file| {
                            filter.is_empty()
                                || file.path.to_string_lossy().to_lowercase().contains(&filter)
                        })
                        .collect();
                    let rows = if catalog {
                        tree::build_catalog(&files, &folders, !filter.is_empty())
                    } else if filter.is_empty() {
                        tree::build_tree(&files, &folders)
                    } else {
                        tree::build_tree(&files, &HashSet::new())
                    };
                    (rows, folders)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if request != this.tree_request || epoch != this.epoch {
                    return;
                }
                if catalog != this.catalog() {
                    return;
                }
                if catalog {
                    this.catalog_expanded = folders;
                } else {
                    this.collapsed = folders;
                }
                this.apply_tree(rows);
                this.load_diff(true, cx);
                cx.notify();
            });
        }));
    }

    fn apply_tree(&mut self, rows: Vec<TreeRow>) {
        let previous = self
            .tree_cursor
            .and_then(|i| self.tree.get(i))
            .and_then(|row| row.path().map(|p| (p.to_owned(), row.untracked)));
        self.tree = rows;
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
        // Refreshes can close stale tabs, but only user navigation opens a file.
        let listed = if self.catalog() {
            &self.snapshot.catalog
        } else {
            &self.snapshot.files
        };
        let files: HashSet<_> = listed.iter().map(|f| &f.path).collect();
        let active = self.active_tab();
        self.tabs.retain(|path| files.contains(path));
        if self
            .selected
            .as_ref()
            .is_some_and(|path| !files.contains(path))
        {
            self.selected = None;
            self.reset_preview_position();
        }
        if self.selected.is_some() {
            return;
        }
        if let Some(index) = active.filter(|_| !self.tabs.is_empty()) {
            self.selected = Some(self.tabs[index.min(self.tabs.len() - 1)].clone());
        }
    }

    fn active_tab(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.tabs.iter().position(|path| path == selected)
    }

    /// Shows `path` in the preview. An open tab is activated; otherwise the file
    /// replaces the active tab, or opens in a new tab after it with `new_tab`.
    /// Returns whether the shown file changed.
    fn show_file(&mut self, path: PathBuf, new_tab: bool) -> bool {
        if self.selected.as_ref() == Some(&path) {
            return false;
        }
        if !self.tabs.contains(&path) {
            match self.active_tab() {
                Some(index) if new_tab => self.tabs.insert(index + 1, path.clone()),
                Some(index) => self.tabs[index] = path.clone(),
                None => self.tabs.push(path.clone()),
            }
        }
        self.selected = Some(path);
        self.reset_preview_position();
        true
    }

    fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.tabs.get(index).cloned() else {
            return;
        };
        if let Some(row) = self
            .tree
            .iter()
            .position(|row| row.path() == Some(path.as_path()))
        {
            self.tree_cursor = Some(row);
            self.files_scroll.scroll_to_item(row, ScrollStrategy::Top);
        }
        if self.show_file(path, false) {
            self.load_diff(false, cx);
        }
        self.focus_left = false;
        self.focus.focus(window);
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let path = self.tabs.remove(index);
        if self.selected.as_ref() != Some(&path) {
            cx.notify();
            return;
        }
        if !self.tabs.is_empty() {
            self.selected = None;
            self.activate_tab(index.min(self.tabs.len() - 1), window, cx);
            return;
        }
        self.selected = None;
        self.preview_request += 1;
        self.preview_loading = false;
        self.reset_preview_position();
        self.scrollbar_drag = None;
        self.terminal_dragging = false;
        self.focus_left = true;
        self.terminal_visible = true;
        self.activate_project_terminal(window, cx);
        cx.notify();
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
        self.show_source = false;
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

    pub(crate) fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
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
        let catalog = self.tree_mode == TreeMode::Catalog;
        let task = cx.background_executor().spawn(async move {
            let repo = Repository::open(&path)?;
            let snapshot = repo.snapshot_with(catalog)?;
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
                        this.remember_project(repo.root.clone(), cx);
                        this.repo = Some(repo);
                        this.cache = PreviewCache::default();
                        this.prefetching.clear();
                        this.syntax_pending.clear();
                        this.current_slot = None;
                        this.preview_target = None;
                        this.snapshot = snapshot;
                        this.selected = None;
                        this.tabs.clear();
                        this.terminal_visible = true;
                        this.tree.clear();
                        this.tree_cursor = None;
                        this.collapsed.clear();
                        this.catalog_expanded.clear();
                        this.catalog_seeded = false;
                        this.focus_left = true;
                        this.filter.clear();
                        this.filter_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.commit_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.review_branch = None;
                        this.review = None;
                        this.review_step = None;
                        this.review_error = None;
                        this.close_picker(window, cx);
                        this.reset_preview_position();
                        this.activate_project_terminal(window, cx);
                        this.notice = "Watching local changes".into();
                        if let Some(branch) = this.pending_review.take() {
                            this.start_review(branch, window, cx);
                        } else {
                            this.rebuild_tree(false, cx);
                        }
                    }
                    Err(error) => this.show_error(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remember_project(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        recent::remember(&mut self.recent, &root);
        crate::update_dock_menu(&self.recent, cx);
        if let Some(file) = recent::store() {
            let projects = self.recent.clone();
            std::thread::spawn(move || {
                if let Err(error) = recent::save(&file, &projects) {
                    eprintln!("give: could not save recent projects: {error}");
                }
            });
        }
    }

    /// ⌘R: reload the review (fetch included) or the working tree.
    fn manual_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.review_branch.clone() {
            Some(branch) => self.start_review(branch, window, cx),
            None => self.refresh(cx),
        }
    }

    /// Working-tree polling. A review compares fixed refs, so it is never polled.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.opening || self.refreshing || self.review_branch.is_some() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.refreshing = true;
        self.snapshot_request += 1;
        let request = self.snapshot_request;
        let epoch = self.epoch;
        let catalog = self.catalog();
        let task = cx
            .background_executor()
            .spawn(async move { repo.snapshot_with(catalog) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if epoch != this.epoch || request != this.snapshot_request {
                    return;
                }
                this.refreshing = false;
                match result {
                    Ok(snapshot) => {
                        let changed_tree = this.snapshot.files != snapshot.files
                            || this.snapshot.catalog != snapshot.catalog;
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
            .selected
            .as_ref()
            .and_then(|path| self.snapshot.file(path))
            .cloned();
        let (Some(repo), Some(file)) = (self.repo.clone(), file) else {
            self.preview_request += 1;
            self.preview_target = None;
            self.current_slot = None;
            self.diff = Arc::default();
            self.syntax = Arc::default();
            self.preview_loading = false;
            self.set_image(None, cx);
            self.prefetch_neighbors(cx);
            return;
        };
        let key = Key {
            path: file.path.clone(),
            version: self.snapshot.versions[&file.path],
        };
        self.load_image(&repo, &file, &key, cx);
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
        let review = self.review.clone();
        let read_task = (!ready).then(|| {
            cx.background_executor()
                .spawn(async move { load_slot(&read_slot, &repo, &file, review.as_deref()) })
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

    fn load_image(
        &mut self,
        repo: &Repository,
        file: &ChangedFile,
        key: &Key,
        cx: &mut Context<Self>,
    ) {
        if crate::image_view::format(&file.path).is_none() {
            self.set_image(None, cx);
            return;
        }
        if self.image.as_ref().is_some_and(|(shown, _)| shown == key) {
            return;
        }
        self.set_image(Some((key.clone(), ImageState::Loading)), cx);
        let (repo, file, review) = (repo.clone(), file.clone(), self.review.clone());
        let task = cx
            .background_executor()
            .spawn(async move { crate::image_view::load(&repo, &file, review.as_deref()) });
        let key = key.clone();
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                let state = match result {
                    Ok(decoded) => ImageState::Ready(decoded),
                    Err(error) => ImageState::Failed(format!("{error:#}").into()),
                };
                if this.image.as_ref().is_some_and(|(shown, _)| *shown == key) {
                    this.set_image(Some((key, state)), cx);
                    cx.notify();
                } else if let ImageState::Ready(decoded) = state {
                    cx.drop_image(decoded.image, None);
                }
            });
        })
        .detach();
    }

    /// Replaces the shown image, freeing the previous one's GPU texture.
    fn set_image(&mut self, image: Option<(Key, ImageState)>, cx: &mut Context<Self>) {
        if let Some((_, ImageState::Ready(old))) = std::mem::replace(&mut self.image, image) {
            // Deferred so the window that is updating now releases it too.
            cx.defer(move |cx| cx.drop_image(old.image, None));
        }
    }

    fn rendered(&self) -> Option<Rendered> {
        let path = self.selected.as_ref()?;
        let svg = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
        if svg || markdown_path(path) {
            if self.show_source {
                None
            } else if svg {
                Some(Rendered::Image)
            } else {
                Some(Rendered::Markdown)
            }
        } else {
            crate::image_view::format(path).map(|_| Rendered::Image)
        }
    }

    /// Whether the shown file can switch between rendered content and source.
    fn has_source_toggle(&self) -> bool {
        self.selected.as_ref().is_some_and(|path| {
            markdown_path(path)
                || path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        })
    }

    fn toggle_source(&mut self, cx: &mut Context<Self>) {
        self.show_source = !self.show_source;
        self.text_selection = None;
        cx.notify();
    }

    fn markdown_text(&self) -> SharedString {
        let mut cache = self.markdown.borrow_mut();
        if let Some((diff, text)) = cache.as_ref()
            && Arc::ptr_eq(diff, &self.diff)
        {
            return text.clone();
        }
        let text: SharedString = self
            .diff
            .lines
            .iter()
            .filter(|line| line.kind != LineKind::Removed)
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        *cache = Some((self.diff.clone(), text.clone()));
        text
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
        // Tabs are expanded and the font is monospaced, so characters approximate width.
        self.widest_line = self
            .diff
            .lines
            .iter()
            .enumerate()
            .max_by_key(|(_, line)| line.text.chars().count())
            .map_or(0, |(index, _)| index);
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
            let Some(file) = self.snapshot.file(path).cloned() else {
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
            let review = self.review.clone();
            let task = cx.background_executor().spawn(async move {
                if load_slot(&slot, &repo, &file, review.as_deref()).is_ok() {
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
        self.open_row(index, false, window, cx);
    }

    /// Moves the tree cursor to `index`; a file opens in the active tab, or in a new
    /// tab with `new_tab`. Folders keep the open preview.
    fn open_row(
        &mut self,
        index: usize,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tree.get(index).is_some_and(TreeRow::navigable) {
            return;
        }
        self.tree_cursor = Some(index);
        if let RowKind::File(file) = &self.tree[index].kind
            && self.show_file(file.path.clone(), new_tab)
        {
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
        if matches!(self.tree[index].kind, RowKind::File(_)) {
            self.select_row(index, window, cx);
            self.focus_left = false;
            return;
        }
        if let RowKind::Directory { path, expanded } = &self.tree[index].kind {
            let path = path.clone();
            let expand = !*expanded;
            if self.catalog() {
                if expand {
                    self.catalog_expanded.insert(path);
                } else {
                    self.catalog_expanded.remove(&path);
                }
            } else if expand {
                self.collapsed.remove(&path);
            } else {
                self.collapsed.insert(path);
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
                self.select_row(index, window, cx);
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
        if self.has_source_toggle() && !self.show_source {
            self.show_source = true;
        }
        self.text_selection = None;
        self.text_layouts.clear();
        self.show_deletions = !self.show_deletions;
        self.load_diff(false, cx);
        cx.notify();
    }

    fn next_change(&mut self, cx: &mut Context<Self>) {
        if self.has_source_toggle() && !self.show_source {
            self.show_source = true;
        }
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
        if self.busy || self.opening || self.review_branch.is_some() {
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
        let catalog = self.catalog();
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
            repo.snapshot_with(catalog)
        });
        self.finish_operation(task, "Index updated", window, cx);
    }

    fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_changes(false, window, cx);
    }

    fn commit_changes(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.opening || self.review_branch.is_some() {
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
        self.start_operation(
            if push {
                "Committing and pushing…"
            } else {
                "Committing…"
            },
            cx,
        );
        let catalog = self.catalog();
        let task = cx.background_executor().spawn(async move {
            let result = if push {
                branches::commit_and_push(&repo, &message).map(Some)
            } else {
                repo.commit(&message).map(|()| None)
            };
            (result, repo.snapshot_with(catalog))
        });
        cx.spawn_in(window, async move |view, cx| {
            let (result, snapshot) = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.busy = false;
                let mut errors = Vec::new();
                match result {
                    Ok(outcome) => {
                        this.commit_input.update(cx, |input, cx| input.set_value("", window, cx));
                        match outcome {
                            None => this.notice = "Commit created".into(),
                            Some(branches::CommitPushOutcome::Pushed(message)) => {
                                this.notice = format!("Commit created · {message}");
                            }
                            Some(branches::CommitPushOutcome::PushFailed(error)) => {
                                errors.push(format!("Commit created locally, but push failed: {error}\nRetry with ‘Push…’ in the branch menu."));
                            }
                        }
                    }
                    Err(error) => errors.push(format!("{error:#}")),
                }
                match snapshot {
                    Ok(snapshot) => {
                        this.cache.retain_versions(&snapshot.versions);
                        this.snapshot = snapshot;
                        this.rebuild_tree(false, cx);
                    }
                    Err(error) => errors.push(format!("Refresh failed: {error:#}")),
                }
                if !errors.is_empty() {
                    this.show_error(errors.join("\n"), cx);
                }
                cx.notify();
            });
        }).detach();
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(snapshot) => {
                        this.snapshot = snapshot;
                        this.notice = message.into();
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

    /// giv's review mode: fetch, then load the MR refs or the merge-base with main.
    fn start_review(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        let branch = branch.trim().to_owned();
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if branch.is_empty() || self.busy || self.opening {
            return;
        }
        self.close_picker(window, cx);
        self.epoch += 1;
        let epoch = self.epoch;
        self.review_branch = Some(branch.clone());
        self.review = None;
        self.review_error = None;
        self.review_step = Some("Fetching…");
        self.refreshing = false;
        self.preview_request += 1;
        self.tree_request += 1;
        self.tree_task = None;
        self.cache = PreviewCache::default();
        self.prefetching.clear();
        self.syntax_pending.clear();
        self.snapshot = Snapshot {
            branch: branch.clone(),
            ..Snapshot::default()
        };
        self.tree.clear();
        self.tree_cursor = None;
        self.collapsed.clear();
        self.selected = None;
        self.tabs.clear();
        self.terminal_visible = true;
        self.activate_project_terminal(window, cx);
        self.reset_preview_position();
        self.focus_left = true;
        self.notice = format!("Reviewing {branch}…");
        self.error = false;
        let fetch_repo = repo.clone();
        let fetch = cx
            .background_executor()
            .spawn(async move { review::fetch(&fetch_repo) });
        cx.spawn(async move |view, cx| {
            let fetched = fetch.await;
            let proceed = view
                .update(cx, |this, cx| {
                    if this.epoch != epoch {
                        return false;
                    }
                    if let Err(error) = &fetched {
                        this.fail_review(format!("Fetch error: {error:#}"), cx);
                        return false;
                    }
                    this.review_step = Some("Loading latest MR diff…");
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !proceed {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { Review::load(&repo, &branch) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                match result {
                    Ok(review) => {
                        this.review_step = None;
                        this.snapshot = review.snapshot();
                        this.notice = format!(
                            "{} changed file{} in {}",
                            review.files.len(),
                            if review.files.len() == 1 { "" } else { "s" },
                            review.branch
                        );
                        this.review = Some(Arc::new(review));
                        this.rebuild_tree(false, cx);
                    }
                    Err(error) => this.fail_review(format!("Review init error: {error:#}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn fail_review(&mut self, message: String, cx: &mut Context<Self>) {
        self.review_step = None;
        self.review_error = Some(message.clone());
        self.show_error(message, cx);
    }

    fn exit_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review_branch.is_none() {
            return;
        }
        if let Some(root) = self.repo.as_ref().map(|repo| repo.root.clone()) {
            self.open(root, window, cx);
        }
    }

    fn open_branch_picker(
        &mut self,
        _: &ReviewBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_branch_picker(BranchPickerMode::Review, window, cx);
    }

    fn show_branch_picker(
        &mut self,
        mode: BranchPickerMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.busy || self.opening {
            return;
        }
        self.picker_open = true;
        self.picker_mode = mode;
        self.branch_menu = None;
        self.branch_row_bounds.borrow_mut().clear();
        self.picker_note = None;
        self.branches.clear();
        self.branch_remotes.clear();
        self.branch_query.clear();
        self.branch_cursor = 0;
        self.branch_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.branch_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.picker_request += 1;
        let request = self.picker_request;
        self.branches_loading = true;
        // The review picker fetches automatically. Checkout exposes Fetch explicitly.
        cx.spawn(async move |view, cx| {
            for fetch in [false, true] {
                if fetch && mode != BranchPickerMode::Review {
                    break;
                }
                let repo = repo.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let fetched = if fetch {
                            review::fetch(&repo).map_err(|error| format!("{error:#}"))
                        } else {
                            Ok(())
                        };
                        (fetched, review::branches(&repo), branches::remotes(&repo))
                    })
                    .await;
                let open = view
                    .update(cx, |this, cx| {
                        if request != this.picker_request || !this.picker_open {
                            return false;
                        }
                        let (fetched, branches, remotes) = result;
                        match remotes {
                            Ok(remotes) => this.branch_remotes = remotes,
                            Err(error) => this.picker_note = Some(format!("{error:#}")),
                        }
                        if let Err(error) = fetched {
                            this.picker_note = Some(format!("Fetch failed: {error}"));
                        }
                        match branches {
                            Ok(branches) => this.set_branches(branches),
                            Err(error) => this.picker_note = Some(format!("{error:#}")),
                        }
                        if fetch || mode != BranchPickerMode::Review {
                            this.branches_loading = false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Replaces the list and keeps the highlighted branch highlighted.
    fn set_branches(&mut self, branches: Vec<Branch>) {
        let highlighted = match self.picker_rows().get(self.branch_cursor) {
            Some(PickerRow::Branch(index)) => self.branches.get(*index).cloned(),
            _ => None,
        };
        self.branches = branches;
        if let Some(branch) = highlighted {
            let rows = self.picker_rows();
            self.branch_cursor = rows
                .iter()
                .position(|row| matches!(row, PickerRow::Branch(i) if self.branches[*i] == branch))
                .unwrap_or(0);
        }
    }

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.picker_open {
            return;
        }
        self.picker_open = false;
        self.branch_menu = None;
        self.picker_request += 1;
        self.branches_loading = false;
        self.focus.focus(window);
        cx.notify();
    }

    /// Branches matching the query, then the typed name when no branch has it exactly.
    fn picker_rows(&self) -> Vec<PickerRow> {
        let query = self.branch_query.to_lowercase();
        if self.picker_mode == BranchPickerMode::Push {
            return self
                .branch_remotes
                .iter()
                .enumerate()
                .filter(|(_, remote)| remote.to_lowercase().contains(&query))
                .map(|(index, _)| PickerRow::Remote(index))
                .collect();
        }
        let mut rows: Vec<_> = self
            .branches
            .iter()
            .enumerate()
            .filter(|(_, branch)| branch.name.to_lowercase().contains(&query))
            .filter(|(_, branch)| match self.picker_mode {
                BranchPickerMode::PullFrom => branch.remote.is_some(),
                _ => true,
            })
            .map(|(index, _)| PickerRow::Branch(index))
            .collect();
        if self.picker_mode == BranchPickerMode::Review
            && !self.branch_query.is_empty()
            && !self
                .branches
                .iter()
                .any(|branch| branch.name == self.branch_query)
        {
            rows.push(PickerRow::Typed);
        }
        rows
    }

    fn move_picker(&mut self, offset: isize, cx: &mut Context<Self>) {
        let count = self.picker_rows().len();
        if count == 0 {
            return;
        }
        self.branch_cursor = self
            .branch_cursor
            .saturating_add_signed(offset)
            .min(count - 1);
        self.branch_scroll
            .scroll_to_item(self.branch_cursor, ScrollStrategy::Center);
        cx.notify();
    }

    fn confirm_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.picker_open || self.busy || self.opening {
            return;
        }
        let rows = self.picker_rows();
        let row = rows.get(self.branch_cursor.min(rows.len().saturating_sub(1)));
        if self.picker_mode == BranchPickerMode::Switch {
            self.open_branch_actions(window, cx);
            return;
        }
        if self.picker_mode != BranchPickerMode::Review {
            let operation = match row {
                Some(PickerRow::Branch(index)) => {
                    let branch = self.branches[*index].clone();
                    match self.picker_mode {
                        BranchPickerMode::PullFrom => BranchOperation::PullFrom(branch),
                        _ => return,
                    }
                }
                Some(PickerRow::Remote(index)) => {
                    BranchOperation::Push(self.branch_remotes[*index].clone())
                }
                _ => return,
            };
            self.branch_operation(operation, window, cx);
            return;
        }
        let branch = match row {
            Some(PickerRow::Branch(index)) => self.branches[*index].name.clone(),
            Some(PickerRow::Typed) => self.branch_query.clone(),
            Some(PickerRow::Remote(_)) => return,
            None => return,
        };
        self.start_review(branch, window, cx);
    }

    fn open_branch_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker_mode != BranchPickerMode::Switch
            || self.busy
            || self.opening
            || self.review_step.is_some()
        {
            return;
        }
        let Some(PickerRow::Branch(index)) = self.picker_rows().get(self.branch_cursor).copied()
        else {
            return;
        };
        let branch = self.branches[index].clone();
        let mut actions = Vec::new();
        if !branch.current {
            actions.push(BranchAction::Checkout);
        }
        actions.push(BranchAction::Review);
        if self.review_branch.is_none() {
            if branch.current {
                actions.extend([
                    BranchAction::Pull,
                    BranchAction::PullFrom,
                    BranchAction::Push,
                ]);
            } else {
                actions.push(BranchAction::Merge);
                if branch.remote.is_some() {
                    actions.push(BranchAction::PullFrom);
                }
            }
        }
        self.branch_menu = Some(BranchMenu {
            branch,
            actions,
            cursor: 0,
        });
        self.branch_menu_focus.focus(window);
        cx.notify();
    }

    fn close_branch_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_menu = None;
        self.branch_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn move_branch_action(&mut self, offset: isize, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.branch_menu {
            menu.cursor = menu
                .cursor
                .saturating_add_signed(offset)
                .min(menu.actions.len() - 1);
            cx.notify();
        }
    }

    fn run_branch_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.opening || self.review_step.is_some() {
            return;
        }
        let Some(menu) = self.branch_menu.take() else {
            return;
        };
        let branch = menu.branch;
        let operation = match menu.actions[menu.cursor] {
            BranchAction::Checkout => BranchOperation::Switch(branch),
            BranchAction::Review => {
                self.start_review(branch.name, window, cx);
                return;
            }
            BranchAction::Merge => BranchOperation::Merge(branch),
            BranchAction::Pull => BranchOperation::Pull,
            BranchAction::PullFrom if branch.remote.is_some() => BranchOperation::PullFrom(branch),
            BranchAction::PullFrom => {
                self.show_branch_picker(BranchPickerMode::PullFrom, window, cx);
                return;
            }
            BranchAction::Push => {
                self.show_branch_picker(BranchPickerMode::Push, window, cx);
                return;
            }
        };
        self.branch_input
            .update(cx, |input, cx| input.focus(window, cx));
        self.branch_operation(operation, window, cx);
    }

    fn branch_operation(
        &mut self,
        operation: BranchOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy
            || self.opening
            || self.review_step.is_some()
            || (self.review_branch.is_some()
                && !matches!(
                    operation,
                    BranchOperation::Switch(_) | BranchOperation::Fetch
                ))
        {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.branch_menu.take().is_some() {
            self.branch_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        let fetching = matches!(operation, BranchOperation::Fetch);
        let reviewing = self.review_branch.is_some();
        let message = match &operation {
            BranchOperation::Switch(_) => "Switching branch…",
            BranchOperation::Fetch => "Fetching all remotes…",
            BranchOperation::Pull | BranchOperation::PullFrom(_) => "Pulling current branch…",
            BranchOperation::Push(_) => "Pushing current branch…",
            BranchOperation::Merge(_) => "Merging into current branch…",
            BranchOperation::AbortMerge => "Aborting merge…",
        };
        // Cancel a branch-list read before it can overwrite the post-operation list.
        self.picker_request += 1;
        self.branches_loading = fetching;
        self.picker_note = None;
        self.start_operation(message, cx);
        let catalog = self.catalog();
        let task = cx.background_executor().spawn(async move {
            let result = match operation {
                BranchOperation::Switch(branch) => {
                    branches::switch(&repo, &branch).map(|name| format!("Switched to {name}"))
                }
                BranchOperation::Fetch => review::fetch(&repo)
                    .map(|()| "Fetch complete · remote branches updated".to_owned()),
                BranchOperation::Pull => branches::pull(&repo)
                    .map(|()| "Pull complete · current branch is up to date".to_owned()),
                BranchOperation::PullFrom(branch) => {
                    branches::pull_from(&repo, &branch).map(|()| {
                        format!("Pulled from {} · upstream tracking configured", branch.name)
                    })
                }
                BranchOperation::Push(remote) => branches::push(&repo, &remote),
                BranchOperation::Merge(branch) => branches::merge(&repo, &branch),
                BranchOperation::AbortMerge => {
                    branches::abort_merge(&repo).map(|()| "Merge aborted".to_owned())
                }
            };
            // Read back even after failure: a network operation may have updated refs.
            (result, repo.snapshot_with(catalog), review::branches(&repo))
        });
        cx.spawn_in(window, async move |view, cx| {
            let (result, snapshot, branches) = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.branches_loading = false;
                if reviewing && !fetching && result.is_ok() {
                    this.epoch += 1;
                    this.review_branch = None;
                    this.review = None;
                    this.review_error = None;
                    this.review_step = None;
                }
                let mut errors = Vec::new();
                match snapshot {
                    Ok(snapshot) if this.review_branch.is_none() => {
                        this.cache.retain_versions(&snapshot.versions);
                        this.snapshot = snapshot;
                        this.rebuild_tree(false, cx);
                    }
                    Ok(_) => this.load_diff(true, cx),
                    Err(error) => errors.push(format!("Refresh failed: {error:#}")),
                }
                match branches {
                    Ok(branches) => this.set_branches(branches),
                    Err(error) => errors.push(format!("Branch list failed: {error:#}")),
                }
                match result {
                    Ok(message) => {
                        this.notice = message;
                        if !fetching {
                            this.close_picker(window, cx);
                        }
                    }
                    Err(error) => errors.insert(0, format!("{error:#}")),
                }
                if !errors.is_empty() {
                    let error = errors.join("\n");
                    if this.picker_open {
                        this.picker_note = Some(error.clone());
                    }
                    this.show_error(error, cx);
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

    fn open_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() || self.rendered() == Some(Rendered::Image) {
            return;
        }
        if self.rendered() == Some(Rendered::Markdown) {
            self.show_source = true;
        }
        self.focus_left = false;
        self.find.update(cx, |find, cx| find.open(window, cx));
        self.search_preview(cx);
        cx.notify();
    }

    fn search_preview(&mut self, cx: &mut Context<Self>) {
        self.find_request += 1;
        let request = self.find_request;
        let query = self.find.read(cx).query.clone();
        let diff = self.diff.clone();
        self.find_diff = Some(diff.clone());
        self.find_matches = Arc::default();
        self.find_current = None;
        let searching = !query.text.is_empty();
        self.find
            .update(cx, |find, cx| find.set_results(None, 0, searching, cx));
        if !searching {
            return;
        }
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(75))
                .await;
            let current = view
                .update(cx, |this, _| this.find_request == request)
                .unwrap_or(false);
            if !current {
                return;
            }
            let source = diff.clone();
            let matches = cx
                .background_executor()
                .spawn(async move { query.source_matches(&source.lines) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.find_request != request || !Arc::ptr_eq(&this.diff, &diff) {
                    return;
                }
                this.find_current = if matches.is_empty() {
                    None
                } else {
                    Some(
                        matches
                            .iter()
                            .position(|m| m.row >= this.preview_cursor)
                            .unwrap_or(0),
                    )
                };
                this.find_matches = Arc::new(matches);
                this.reveal_match(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn next_match(&mut self, backwards: bool, cx: &mut Context<Self>) {
        self.find_current =
            search::next_index(self.find_current, self.find_matches.len(), backwards);
        self.reveal_match(cx);
        cx.notify();
    }

    fn reveal_match(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self
            .find_current
            .and_then(|index| self.find_matches.get(index))
        {
            self.preview_cursor = m.row;
            self.diff_scroll.scroll_to_item(m.row, ScrollStrategy::Top);
            let state = self.diff_scroll.0.borrow();
            let width = f32::from(state.base_handle.bounds().size.width);
            let left =
                self.diff.lines[m.row].text[..m.range.start].chars().count() as f32 * 9. + 124.;
            let offset = state.base_handle.offset();
            state
                .base_handle
                .set_offset(point(px(-(left - width / 2.).max(0.)), offset.y));
        }
        self.find.update(cx, |find, cx| {
            find.set_results(self.find_current, self.find_matches.len(), false, cx)
        });
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

    fn close_preview(&mut self, _: &ClosePreview, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.active_tab() {
            self.close_tab(index, window, cx);
        }
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_some() && self.selected.is_none() {
            return;
        }
        self.terminal_visible = !self.terminal_visible;
        self.activate_project_terminal(window, cx);
        if !self.terminal_visible {
            self.focus.focus(window);
        }
        cx.notify();
    }

    fn toggle_history(&mut self, _: &ToggleHistory, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        self.history_visible = !self.history_visible;
        self.history_dragging = false;
        self.focus.focus(window);
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.sidebar_dragging = false;
        // Arrow keys would otherwise move through a tree that is not on screen.
        if !self.sidebar_visible && self.selected.is_some() {
            self.focus_left = false;
        }
        self.focus.focus(window);
        cx.notify();
    }

    fn absolute(&self, path: &std::path::Path) -> PathBuf {
        self.repo
            .as_ref()
            .map_or_else(|| path.to_owned(), |repo| repo.root.join(path))
    }

    /// The file or folder's absolute path, shared by right-click and Ctrl-C.
    fn tree_copy(&self, index: usize) -> Option<ContextCopy> {
        let path = match &self.tree.get(index)?.kind {
            RowKind::File(file) => &file.path,
            RowKind::Directory { path, .. } => path,
            _ => return None,
        };
        Some(ContextCopy {
            label: "Copy path".into(),
            text: self.absolute(path).to_string_lossy().into_owned(),
        })
    }

    fn tree_context(&mut self, index: usize) {
        self.context_copy = self.tree_copy(index);
    }

    /// The selection with its lines when the row is inside it, otherwise that line.
    fn preview_copy(&self, row: usize) -> Option<ContextCopy> {
        let path = self.absolute(self.selected.as_ref()?);
        let selection = self
            .text_selection
            .filter(|selection| {
                let (start, end) = selection.ordered();
                (start.row..=end.row).contains(&row)
            })
            .and_then(|selection| path_reference::selection(&path, &self.diff.lines, selection));
        Some(match selection {
            Some(text) => ContextCopy {
                label: "Copy path with selection".into(),
                text,
            },
            None => ContextCopy {
                label: "Copy path with line".into(),
                text: path_reference::line(&path, &self.diff.lines, row),
            },
        })
    }

    fn preview_context(&mut self, row: usize) {
        self.context_copy = self.preview_copy(row);
    }

    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let copy = if self.focus_left {
            self.tree_cursor.and_then(|index| self.tree_copy(index))
        } else {
            let text = self
                .text_selection
                .map(|selection| selection.text(&self.diff.lines))
                .unwrap_or_default();
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.notice = "Selection copied".into();
                self.error = false;
                cx.notify();
                return;
            }
            self.preview_copy(self.preview_cursor)
        };
        if let Some(copy) = copy {
            self.write_path(copy, cx);
        }
    }

    fn write_path(&mut self, copy: ContextCopy, cx: &mut Context<Self>) {
        self.notice = format!("Copied {}", copy.text.lines().next().unwrap_or_default());
        self.error = false;
        cx.write_to_clipboard(ClipboardItem::new_string(copy.text));
        cx.notify();
    }

    /// The tree and preview context menu; an empty menu does not open.
    fn context_menu(
        view: &WeakEntity<Self>,
        menu: PopupMenu,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Some(copy) = view
            .upgrade()
            .and_then(|view| view.read(cx).context_copy.clone())
        else {
            return menu;
        };
        let view = view.clone();
        menu.item(
            PopupMenuItem::new(copy.label.clone()).on_click(move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.write_path(copy.clone(), cx);
                });
            }),
        )
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
        let sidebar = self.repo.is_none() || self.selected.is_some();
        let width = self
            .terminal_width
            .min((f32::from(window.viewport_size().width) - 600.).max(280.));
        div()
            .flex()
            .when(sidebar, |el| el.w(px(width)).flex_shrink_0())
            .when(!sidebar, |el| el.flex_1().min_w_0())
            .h_full()
            .bg(rgb(PANEL))
            .when(sidebar, |el| {
                el.child(
                    div()
                        .id("terminal-resize")
                        .w(px(4.))
                        .h_full()
                        .flex_shrink_0()
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
            })
            // The terminal takes what the handle leaves, so it can never squeeze the handle out.
            .when_some(self.terminal.clone(), |el, terminal| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .child(terminal),
                )
            })
    }

    fn drag_ui(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_dragging {
            self.history_height =
                (f32::from(window.viewport_size().height - event.position.y) - 25.).clamp(
                    150.,
                    (f32::from(window.viewport_size().height) - 180.).max(150.),
                );
            cx.notify();
        } else if self.sidebar_dragging {
            self.sidebar_width = f32::from(event.position.x).clamp(
                180.,
                (f32::from(window.viewport_size().width) * 0.6).max(180.),
            );
            cx.notify();
        } else if self.terminal_dragging && (self.repo.is_none() || self.selected.is_some()) {
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
        let branch_bounds = self.branch_button_bounds.clone();
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
                            .when(self.review_branch.is_some(), |el| {
                                el.child(
                                    div()
                                        .px_1p5()
                                        .rounded(px(3.))
                                        .bg(rgb(0x2b4057))
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(ACCENT))
                                        .child("REVIEW"),
                                )
                            })
                            .child(
                                div()
                                    .id("current-branch")
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|el| el.bg(rgb(SURFACE)))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.picker_open {
                                            this.close_picker(window, cx);
                                        } else {
                                            this.show_branch_picker(
                                                BranchPickerMode::Switch,
                                                window,
                                                cx,
                                            );
                                        }
                                    }))
                                    .text_color(rgb(if self.review_branch.is_some() {
                                        TEXT
                                    } else {
                                        MUTED
                                    }))
                                    .child(ui_icon("branch", 13., MUTED))
                                    .child(
                                        div().max_w(px(300.)).truncate().child(
                                            self.review_branch
                                                .clone()
                                                .unwrap_or_else(|| self.snapshot.branch.clone()),
                                        ),
                                    )
                                    .child(ui_icon("chevron-down", 10., MUTED))
                                    .child(
                                        canvas(
                                            move |bounds, _, _| branch_bounds.set(bounds),
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .inset_0(),
                                    ),
                            )
                            .when_some(self.review.as_ref(), |el, review| {
                                el.child(
                                    div()
                                        .text_color(rgb(MUTED))
                                        .child(format!("vs {}", review.compared_to())),
                                )
                            })
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
                            .id("toggle-sidebar")
                            .w(px(30.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .bg(rgb(if self.sidebar_visible {
                                SURFACE
                            } else {
                                0x151e28
                            }))
                            .hover(|el| el.bg(rgb(SURFACE)))
                            .tooltip(|window, cx| {
                                Tooltip::new("Toggle left sidebar · ⌘1").build(window, cx)
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_sidebar(&ToggleSidebar, window, cx)
                            }))
                            .child(ui_icon(
                                "panel-left",
                                17.,
                                if self.sidebar_visible { ACCENT } else { MUTED },
                            )),
                    )
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
                            .when(self.repo.is_some() && self.selected.is_none(), |el| {
                                el.opacity(0.5)
                            })
                            .hover(|el| el.bg(rgb(SURFACE)))
                            .tooltip({
                                let can_hide = self.repo.is_none() || self.selected.is_some();
                                move |window, cx| {
                                    Tooltip::new(if can_hide {
                                        "Toggle right terminal · ⌘2"
                                    } else {
                                        "Open a file to hide the terminal"
                                    })
                                    .build(window, cx)
                                }
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
                    .when(self.review_branch.is_some(), |el| {
                        el.child(
                            Button::new("exit-review")
                                .ghost()
                                .xsmall()
                                .label("Exit review")
                                .disabled(self.busy || self.opening)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.exit_review(window, cx)),
                                ),
                        )
                    })
                    .child(
                        Button::new("review-branch")
                            .ghost()
                            .xsmall()
                            .icon(gpui_component::Icon::empty().path("icons/branch.svg"))
                            .label("Review branch")
                            .tooltip("Review a branch against main · ⌘⇧R")
                            .disabled(self.repo.is_none() || self.busy || self.opening)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_branch_picker(&ReviewBranch, window, cx)
                            })),
                    )
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .xsmall()
                            .label("Refresh")
                            .disabled(self.repo.is_none() || self.busy || self.opening)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.manual_refresh(window, cx)),
                            ),
                    )
                    .child({
                        let view = cx.entity().downgrade();
                        let recent = self.recent.clone();
                        let current = self.repo.as_ref().map(|repo| repo.root.clone());
                        Button::new("open")
                            .ghost()
                            .xsmall()
                            .label("Open project")
                            .icon(gpui_component::Icon::empty().path("icons/chevron-down.svg"))
                            .disabled(self.busy)
                            .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
                                let mut menu = menu.min_w(px(260.)).max_w(px(420.));
                                let recent = recent
                                    .iter()
                                    .filter(|path| current.as_ref() != Some(*path))
                                    .take(8)
                                    .cloned()
                                    .collect::<Vec<_>>();
                                if !recent.is_empty() {
                                    menu = menu.label("Recent projects");
                                    for path in recent {
                                        let view = view.clone();
                                        let label = path.clone();
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                recent_project_label(&label)
                                            })
                                            .on_click(move |_, window, cx| {
                                                let path = path.clone();
                                                let _ = view.update(cx, |this, cx| {
                                                    this.open(path, window, cx)
                                                });
                                            }),
                                        );
                                    }
                                    menu = menu.separator();
                                }
                                let view = view.clone();
                                menu.item(PopupMenuItem::new("Browse…").on_click(
                                    move |_, window, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            this.open_picker(&OpenRepository, window, cx)
                                        });
                                    },
                                ))
                            })
                    }),
            )
    }

    /// The sidebar title doubles as the Changes/Catalog switch.
    fn tree_mode_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let current = self.tree_mode;
        Button::new("tree-mode")
            .ghost()
            .xsmall()
            .label(current.label())
            .icon(gpui_component::Icon::empty().path("icons/chevron-down.svg"))
            .disabled(self.opening)
            .dropdown_menu_with_anchor(Corner::TopLeft, move |mut menu, _, _| {
                for (mode, label) in [
                    (TreeMode::Changes, "Changes"),
                    (TreeMode::Catalog, "Catalog · all files"),
                ] {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(mode == current).on_click(
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.set_tree_mode(mode, cx));
                        },
                    ));
                }
                menu
            })
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
        let reviewing = self.review_branch.is_some();
        let can_commit = staged > 0
            && !self.busy
            && !self.opening
            && !self.commit_input.read(cx).value().trim().is_empty();
        let view = cx.entity().downgrade();
        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
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
                            .when(reviewing, |el| el.child("BRANCH"))
                            .when(!reviewing, |el| el.child(self.tree_mode_menu(cx)))
                            .child(
                                div()
                                    .text_color(rgb(ACCENT))
                                    .child(self.snapshot.files.len().to_string()),
                            ),
                    )
                    .when(!reviewing, |el| {
                        el.child(
                            Button::new("all")
                                .ghost()
                                .xsmall()
                                .label("Stage all")
                                .disabled(self.busy || self.opening || working == 0)
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.mutate(true, window, cx)
                                    }),
                                ),
                        )
                    }),
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
                    .capture_any_mouse_down(cx.listener(clear_context_copy))
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
                                                    let badge = if this.review_branch.is_some() {
                                                        file.index.to_string()
                                                    } else if file.clean() {
                                                        String::new()
                                                    } else if file.untracked() {
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
                                            let name_color = match &row.kind {
                                                RowKind::File(file) => file_name_color(file),
                                                // Catalog folders holding changes are tinted.
                                                RowKind::Directory { .. }
                                                    if row.changed && this.catalog() =>
                                                {
                                                    NAME_MODIFIED
                                                }
                                                _ if navigable => TEXT,
                                                _ => MUTED,
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
                                                        .on_mouse_down(
                                                            MouseButton::Right,
                                                            cx.listener(move |this, _, _, _| {
                                                                this.tree_context(index)
                                                            }),
                                                        )
                                                        .on_click(cx.listener(
                                                            move |this, event: &ClickEvent, window, cx| {
                                                                this.open_row(
                                                                    index,
                                                                    event.modifiers().secondary(),
                                                                    window,
                                                                    cx,
                                                                );
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
                                                            .text_color(rgb(name_color))
                                                            .child(
                                                                if this.review_branch.is_some()
                                                                    && row.kind == RowKind::Section
                                                                {
                                                                    "BRANCH CHANGES".into()
                                                                } else {
                                                                    row.label.clone()
                                                                },
                                                            ),
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
                        el.child(
                            div()
                                .p_4()
                                .text_color(rgb(if self.review_error.is_some() {
                                    RED
                                } else {
                                    MUTED
                                }))
                                .child(if let Some(step) = self.review_step {
                                    step.to_owned()
                                } else if let Some(error) = &self.review_error {
                                    error.clone()
                                } else if reviewing {
                                    "No changes in this branch".to_owned()
                                } else if self.catalog() {
                                    "No files".to_owned()
                                } else {
                                    "No changed files".to_owned()
                                }),
                        )
                    })
                    .context_menu(move |menu, _, cx| Self::context_menu(&view, menu, cx)),
            )
            .when(reviewing, |el| {
                el.child(
                    div()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(
                            div()
                                .text_color(rgb(TEXT))
                                .child(if self.review_step.is_some() {
                                    "Loading review…".to_owned()
                                } else {
                                    self.review
                                        .as_ref()
                                        .map(|review| review.status())
                                        .unwrap_or_default()
                                }),
                        )
                        .child("Read-only review · Exit review returns to local changes"),
                )
            })
            .when(!reviewing, |el| {
                el.child(
                    div()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .p_1p5()
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
                                    .child(
                                        Input::new(&self.commit_input).small().disabled(self.busy || self.opening),
                                    )
                                    .child(
                                        DropdownButton::new("commit-actions")
                                            .primary()
                                            .small()
                                            .w_full()
                                            .disabled(!can_commit)
                                            .button(
                                                Button::new("commit")
                                                    .flex_1()
                                                    .min_w_0()
                                                    .label(if self.busy { "Working…" } else { "Commit" })
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.commit(&Commit, window, cx)
                                                    })),
                                            )
                                            .dropdown_menu_with_anchor(Corner::BottomRight, {
                                                let view = cx.entity().downgrade();
                                                move |menu, _, _| {
                                                    let view = view.clone();
                                                    menu.item(PopupMenuItem::new("Commit + Push").on_click(move |_, window, cx| {
                                                        let _ = view.update(cx, |this, cx| this.commit_changes(true, window, cx));
                                                    }))
                                                }
                                            }),
                                    ),
                            )
                        }),
                )
            });
        // Each context menu keeps its state under its ancestors' ids.
        div()
            .id("sidebar")
            .flex()
            .w(px(self.sidebar_width))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(PANEL))
            .child(content)
            .child(
                div()
                    .id("sidebar-resize")
                    .w(px(4.))
                    .h_full()
                    .flex_shrink_0()
                    .cursor_col_resize()
                    .bg(rgb(BORDER))
                    .hover(|el| el.bg(rgb(ACCENT)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.sidebar_dragging = true;
                            cx.stop_propagation();
                        }),
                    ),
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

    fn preview(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rendered = self.rendered();
        let path = self
            .selected
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let unstage = self
            .snapshot
            .files
            .iter()
            .find(|f| Some(&f.path) == self.selected.as_ref())
            .is_some_and(|f| f.staged() && !f.unstaged());
        let view = cx.entity().downgrade();
        div()
            .id("preview")
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
                    .when(self.tabs.is_empty(), |el| {
                        el.child(
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
                                .child("Review"),
                        )
                    })
                    .child(
                        div()
                            .id("preview-tabs")
                            .flex()
                            .min_w_0()
                            .overflow_x_scroll()
                            .children(self.tabs.iter().enumerate().map(|(index, path)| {
                                let active = self.selected.as_ref() == Some(path);
                                let name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                div()
                                    .id(("preview-tab", index))
                                    .pl_4()
                                    .pr_2()
                                    .h_full()
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap_2()
                                    .bg(rgb(if active { BACKGROUND } else { PANEL }))
                                    .border_r_1()
                                    .border_color(rgb(BORDER))
                                    .when(active, |el| el.border_b_2())
                                    .when(!active, |el| {
                                        el.cursor_pointer().hover(|el| el.bg(rgb(SURFACE)))
                                    })
                                    .text_size(px(13.))
                                    .text_color(rgb(if active { TEXT } else { MUTED }))
                                    .child(ui_icon(
                                        "code",
                                        14.,
                                        if active { ACCENT } else { MUTED },
                                    ))
                                    .child(name)
                                    .child(
                                        Button::new(("close-preview", index))
                                            .ghost()
                                            .xsmall()
                                            .label("×")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.close_tab(index, window, cx)
                                            })),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.activate_tab(index, window, cx)
                                    }))
                            })),
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
                                .when(self.has_source_toggle(), |el| {
                                    el.child(
                                        Button::new("source")
                                            .ghost()
                                            .xsmall()
                                            .label(if self.show_source {
                                                "Rendered"
                                            } else {
                                                "Source"
                                            })
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.toggle_source(cx)
                                                }),
                                            ),
                                    )
                                })
                                .when(rendered.is_none(), |el| {
                                    el.child(
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
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_deletions(cx)
                                            })),
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
                                })
                                .when(self.review_branch.is_none(), |el| {
                                    el.child(
                                        Button::new("stage-file")
                                            .ghost()
                                            .xsmall()
                                            .label(if unstage { "Unstage" } else { "Stage" })
                                            .disabled(self.busy || self.opening)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.mutate(false, window, cx)
                                            })),
                                    )
                                }),
                        )
                    }),
            )
            .when(self.find.read(cx).visible, |el| el.child(self.find.clone()))
            .when(!self.preview_loading && self.selected.is_none(), |el| {
                let branch = self.review_branch.clone().unwrap_or_default();
                el.child(if let Some(step) = self.review_step {
                    empty_state(format!("Reviewing {branch}"), step, false)
                } else if let Some(error) = &self.review_error {
                    empty_state(format!("Could not review {branch}"), error.clone(), false)
                } else if self.review.as_ref().is_some_and(|r| r.files.is_empty()) {
                    empty_state(
                        format!("{branch} has no changes"),
                        "The branch matches its base.",
                        false,
                    )
                } else {
                    empty_state(
                        "Select a changed file",
                        "Browse the tree to review the full file and its changes.",
                        false,
                    )
                })
            })
            .when(
                self.selected.is_some() && rendered == Some(Rendered::Markdown),
                |el| el.child(self.markdown_view(window, cx)),
            )
            .when(
                self.selected.is_some() && rendered == Some(Rendered::Image),
                |el| el.child(self.image_view()),
            )
            .when(self.selected.is_some() && rendered.is_none(), |el| {
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
                        .capture_any_mouse_down(cx.listener(clear_context_copy))
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
                                            let first_match = this
                                                .find_matches
                                                .partition_point(|m| m.row < index);
                                            let row_matches: Vec<_> = this
                                                .find_matches
                                                .iter()
                                                .enumerate()
                                                .skip(first_match)
                                                .take_while(|(_, m)| m.row == index)
                                                .collect();
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
                                                for (_, m) in &row_matches {
                                                    edges.extend([
                                                        m.range.start.clamp(
                                                            span.range.start,
                                                            span.range.end,
                                                        ),
                                                        m.range.end.clamp(
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
                                                                .map(|_| rgb(0x3c5c7e).into())
                                                                .or_else(|| {
                                                                    row_matches
                                                                        .iter()
                                                                        .find(|(_, m)| {
                                                                            m.range.contains(
                                                                                &range.start,
                                                                            )
                                                                        })
                                                                        .map(|(match_index, _)| {
                                                                            rgb(
                                                                                if Some(
                                                                                    *match_index,
                                                                                ) == this
                                                                                    .find_current
                                                                                {
                                                                                    0x87642c
                                                                                } else {
                                                                                    0x514931
                                                                                },
                                                                            )
                                                                            .into()
                                                                        })
                                                                }),
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
                                                .on_mouse_down(
                                                    MouseButton::Right,
                                                    cx.listener(move |this, _, _, _| {
                                                        this.preview_context(index)
                                                    }),
                                                )
                                                .when(
                                                    this.show_deletions
                                                        || this.review_branch.is_some(),
                                                    |el| {
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
                                                    },
                                                )
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
                            .with_width_from_item(Some(self.widest_line))
                            .track_scroll(self.diff_scroll.clone())
                            .flex_1()
                            .min_w_0()
                            .min_h_0(),
                        )
                        .child(self.overview(cx))
                        .context_menu(move |menu, _, cx| Self::context_menu(&view, menu, cx)),
                )
            })
    }

    fn markdown_view(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = format!(
            "markdown-{}",
            self.selected
                .as_ref()
                .map(|path| path.to_string_lossy())
                .unwrap_or_default()
        );
        let style = TextViewStyle {
            is_dark: true,
            highlight_theme: cx.theme().highlight_theme.clone(),
            ..TextViewStyle::default()
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(14.))
            .text_color(rgb(TEXT))
            .child(
                TextView::markdown(SharedString::from(id), self.markdown_text(), window, cx)
                    .style(style)
                    .selectable(true)
                    .scrollable(true)
                    .flex_1()
                    .min_h_0()
                    .px_8()
                    .py_6(),
            )
    }

    fn image_view(&self) -> impl IntoElement {
        let content = match self.image.as_ref().map(|(_, state)| state) {
            Some(ImageState::Ready(decoded)) => div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    img(ImageSource::Render(decoded.image.clone()))
                        .w(px(decoded.width))
                        .h(px(decoded.height))
                        .max_w_full()
                        .max_h_full()
                        .flex_shrink()
                        .min_h_0()
                        .object_fit(ObjectFit::Contain),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "{} × {}",
                            decoded.width.round(),
                            decoded.height.round()
                        )),
                ),
            Some(ImageState::Failed(error)) => {
                empty_state("Image preview unavailable", error.clone(), false)
            }
            _ => empty_state("Loading image…", "", false),
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .p_6()
            .child(content)
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

    fn branch_picker(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.picker_rows();
        let reserved = if self.picker_note.is_some() {
            400.
        } else {
            280.
        };
        let max_height = (f32::from(window.viewport_size().height) - reserved).max(60.);
        let height = ((rows.len().clamp(1, 11) as f32) * 30.).min(max_height);
        let switching = self.picker_mode == BranchPickerMode::Switch;
        let working = self.picker_mode != BranchPickerMode::Review;
        let title = match self.picker_mode {
            BranchPickerMode::Switch => "Branches".to_owned(),
            BranchPickerMode::Review => "Review branch".to_owned(),
            BranchPickerMode::PullFrom => {
                format!("Pull a remote branch → {}", self.snapshot.branch)
            }
            BranchPickerMode::Push => format!("Push {} → remote", self.snapshot.branch),
        };
        let hint = match self.picker_mode {
            BranchPickerMode::Switch => "Select a branch to see its available actions.",
            BranchPickerMode::Review => {
                "Fetches all remotes, then compares with the open GitLab MR or the merge-base with main."
            }
            BranchPickerMode::PullFrom => {
                "Choose the remote source. Fast-forward only; saves upstream tracking after success."
            }
            BranchPickerMode::Push => {
                "Choose a remote. Publishes the current branch with the same name and sets upstream tracking."
            }
        };
        let keys = match self.picker_mode {
            BranchPickerMode::Switch => "↑↓ choose · Enter / → actions · Esc close",
            BranchPickerMode::Review => "↑↓ choose · Enter review · Esc close",
            BranchPickerMode::PullFrom => "↑↓ choose · Enter pull · Esc close",
            BranchPickerMode::Push => "↑↓ choose · Enter push · Esc close",
        };
        let panel_bounds = self.branch_panel_bounds.clone();
        let panel = div()
            .relative()
            .key_context("BranchPicker")
            .on_action(cx.listener(|this, _: &PickerUp, _, cx| this.move_picker(-1, cx)))
            .on_action(cx.listener(|this, _: &PickerDown, _, cx| this.move_picker(1, cx)))
            .on_action(
                cx.listener(|this, _: &PickerCancel, window, cx| this.close_picker(window, cx)),
            )
            .on_action(cx.listener(|this, _: &PickerOpenActions, window, cx| {
                this.open_branch_actions(window, cx)
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(if switching { 440. } else { 560. }))
            .h_auto()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(0x35495f))
            .rounded(px(8.))
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(hint)),
            )
            .when(switching, |el| {
                el.child(
                    div()
                        .px_3()
                        .pb_2()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("fetch-branches")
                                .ghost()
                                .xsmall()
                                .label("Fetch all remotes")
                                .disabled(
                                    self.busy
                                        || self.branches_loading
                                        || self.review_step.is_some(),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.branch_operation(BranchOperation::Fetch, window, cx)
                                })),
                        )
                        .child(
                            Button::new("abort-merge")
                                .ghost()
                                .xsmall()
                                .label("Abort merge")
                                .disabled(self.busy || self.review_branch.is_some())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.branch_operation(BranchOperation::AbortMerge, window, cx)
                                })),
                        ),
                )
            })
            .when(working && !switching, |el| {
                el.child(
                    div().px_3().pb_2().flex().gap_2().child(
                        Button::new("back-to-branches")
                            .ghost()
                            .xsmall()
                            .label("Back to branches")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_branch_picker(BranchPickerMode::Switch, window, cx)
                            })),
                    ),
                )
            })
            .child(
                div().px_3().pb_2().child(
                    Input::new(&self.branch_input)
                        .small()
                        .prefix(ui_icon("branch", 13., MUTED)),
                ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .h(px(height))
                    .when(rows.is_empty(), |el| {
                        el.child(
                            div()
                                .h_full()
                                .px_4()
                                .flex()
                                .items_center()
                                .text_color(rgb(MUTED))
                                .child(if self.branches_loading {
                                    "Loading branches…"
                                } else if self.picker_mode == BranchPickerMode::Push {
                                    "No remotes. Configure a Git remote first."
                                } else {
                                    "No matching branches. Clear the filter or fetch remotes."
                                }),
                        )
                    })
                    .when(!rows.is_empty(), |el| {
                        el.child(
                            uniform_list(
                                "branches",
                                rows.len(),
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    let rows = this.picker_rows();
                                    range
                                        .filter_map(|index| {
                                            let row = *rows.get(index)?;
                                            Some(this.picker_row(index, row, cx))
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(self.branch_scroll.clone())
                            .h_full(),
                        )
                    }),
            )
            .when_some(self.picker_note.as_ref(), |el, note| {
                el.child(
                    div()
                        .id("branch-operation-error")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .px_4()
                        .py_2()
                        .text_size(px(11.))
                        .text_color(rgb(RED))
                        .child(note.chars().take(1800).collect::<String>()),
                )
            })
            .child(
                div()
                    .h(px(28.))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child(div().flex_1().min_w_0().truncate().child(if self.busy {
                        self.notice.clone()
                    } else if self.branches_loading {
                        "Loading branches…".to_owned()
                    } else {
                        format!(
                            "{} {}",
                            rows.len(),
                            if self.picker_mode == BranchPickerMode::Push {
                                "remotes"
                            } else {
                                "branches"
                            }
                        )
                    }))
                    .child(keys),
            )
            .child(
                canvas(
                    move |bounds, _, _| panel_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );

        let backdrop = div()
            .id("branch-picker-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_picker(window, cx)),
            );
        if working {
            let bounds = self.branch_button_bounds.get();
            let mut group = div().flex().items_start().gap_1().child(panel);
            if let Some(menu) = &self.branch_menu {
                let panel_bounds = self.branch_panel_bounds.get();
                let row_bounds = self
                    .branch_row_bounds
                    .borrow()
                    .get(&self.branch_cursor)
                    .copied();
                let offset = row_bounds
                    .map(|row| row.top() - panel_bounds.top())
                    .unwrap_or(px(0.));
                let menu_height = px(70. + menu.actions.len() as f32 * 32.);
                let offset = offset
                    .max(px(0.))
                    .min((panel_bounds.size.height - menu_height).max(px(0.)));
                group = group.child(div().mt(offset).child(self.branch_actions_menu(menu, cx)));
            }
            backdrop.child(
                anchored()
                    .position(point(bounds.left(), bounds.bottom() + px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(group),
            )
        } else {
            backdrop
                .flex()
                .justify_center()
                .pt(px(88.))
                .bg(hsla(0., 0., 0., 0.35))
                .child(panel)
        }
    }

    fn picker_row(&self, index: usize, row: PickerRow, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = index == self.branch_cursor;
        let row_bounds = self.branch_row_bounds.clone();
        let (name, tag, current) = match row {
            PickerRow::Branch(i) => {
                let branch = &self.branches[i];
                (
                    branch.name.clone(),
                    if branch.remote.is_some() {
                        "remote"
                    } else {
                        "local"
                    },
                    branch.current,
                )
            }
            PickerRow::Remote(i) => (
                format!(
                    "{} → {}/{}",
                    self.snapshot.branch, self.branch_remotes[i], self.snapshot.branch
                ),
                "remote",
                false,
            ),
            PickerRow::Typed => (format!("Review “{}”", self.branch_query), "", false),
        };
        div()
            .id(index)
            .relative()
            .when(self.busy || self.review_step.is_some(), |el| {
                el.opacity(0.55)
            })
            .h(px(30.))
            .w_full()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .bg(rgb(if selected { 0x2b4057 } else { PANEL }))
            .hover(|el| el.bg(rgb(SURFACE)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.branch_cursor = index;
                this.confirm_branch(window, cx);
            }))
            .child(ui_icon(
                "branch",
                13.,
                if tag == "remote" { MUTED } else { ACCENT },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .child(name),
            )
            .when(current, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(GREEN))
                        .child("current"),
                )
            })
            .when(!tag.is_empty(), |el| {
                el.child(div().text_size(px(10.)).text_color(rgb(MUTED)).child(tag))
            })
            .when(self.picker_mode == BranchPickerMode::Switch, |el| {
                el.child(ui_icon("chevron-right", 12., MUTED))
            })
            .child(
                canvas(
                    move |bounds, _, _| {
                        row_bounds.borrow_mut().insert(index, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }

    fn branch_actions_menu(&self, menu: &BranchMenu, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("branch-actions")
            .key_context("BranchActions")
            .track_focus(&self.branch_menu_focus)
            .on_action(cx.listener(|this, _: &PickerUp, _, cx| this.move_branch_action(-1, cx)))
            .on_action(cx.listener(|this, _: &PickerDown, _, cx| this.move_branch_action(1, cx)))
            .on_action(
                cx.listener(|this, _: &PickerAccept, window, cx| {
                    this.run_branch_action(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &PickerBack, window, cx| {
                    this.close_branch_actions(window, cx)
                }),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(400.))
            .py_1()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(0x35495f))
            .rounded(px(8.))
            .shadow_lg()
            .text_size(px(13.))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(menu.branch.name.clone()),
            )
            .children(menu.actions.iter().enumerate().map(|(index, action)| {
                let label = match action {
                    BranchAction::Checkout => {
                        if menu.branch.remote.is_some() {
                            "Checkout with tracking".to_owned()
                        } else {
                            "Checkout".to_owned()
                        }
                    }
                    BranchAction::Review => "Review branch".to_owned(),
                    BranchAction::Merge => format!("Merge into ‘{}’", self.snapshot.branch),
                    BranchAction::Pull => "Update · pull from upstream".to_owned(),
                    BranchAction::PullFrom if menu.branch.remote.is_some() => {
                        format!("Pull into ‘{}’", self.snapshot.branch)
                    }
                    BranchAction::PullFrom => "Pull from…".to_owned(),
                    BranchAction::Push => "Push…".to_owned(),
                };
                div()
                    .id(("branch-action", index))
                    .h(px(32.))
                    .mx_1()
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .bg(rgb(if menu.cursor == index {
                        0x2b4057
                    } else {
                        PANEL
                    }))
                    .hover(|el| el.bg(rgb(SURFACE)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(menu) = &mut this.branch_menu {
                            menu.cursor = index;
                        }
                        this.run_branch_action(window, cx);
                    }))
                    .child(div().truncate().child(label))
            }))
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child("↑↓ choose · Enter run · ← / Esc back"),
            )
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
            .when(!self.recent.is_empty(), |el| {
                el.child(
                    div()
                        .w(px(420.))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .px_3()
                                .pb_1()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child("Recent projects"),
                        )
                        .children(self.recent.iter().take(5).enumerate().map(|(ix, path)| {
                            let target = path.clone();
                            div()
                                .id(("welcome-recent", ix))
                                .px_3()
                                .py_1p5()
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|el| el.bg(rgb(SURFACE)))
                                .when(self.opening, |el| el.opacity(0.5))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if !this.opening {
                                        this.open(target.clone(), window, cx);
                                    }
                                }))
                                .child(recent_project_label(path))
                        })),
                )
            })
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
        // The last tab can also disappear during a background refresh.
        if self.repo.is_some() && self.selected.is_none() && !self.terminal_visible {
            self.terminal_visible = true;
            self.activate_project_terminal(window, cx);
        }
        let history_target = if self.review_branch.is_some() && self.review.is_none() {
            None
        } else {
            self.repo.as_ref().map(|repo| {
                (
                    repo.root.clone(),
                    self.review
                        .as_ref()
                        .map_or_else(|| "HEAD".to_owned(), |review| review.head.clone()),
                    self.review_branch
                        .clone()
                        .unwrap_or_else(|| self.snapshot.branch.clone()),
                )
            })
        };
        self.history.update(cx, |history, cx| {
            history.configure(history_target, self.history_visible, cx)
        });
        if self.find.read(cx).visible
            && self
                .find_diff
                .as_ref()
                .is_none_or(|diff| !Arc::ptr_eq(diff, &self.diff))
        {
            self.search_preview(cx);
        }
        self.publish_selection(window);
        div()
            .key_context("Give")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open_picker))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::copy_diff))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::open_find))
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
                    this.sidebar_dragging = false;
                    this.history_dragging = false;
                    this.preview_selecting = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.terminal_dragging = false;
                    this.sidebar_dragging = false;
                    this.history_dragging = false;
                    this.preview_selecting = false;
                }),
            )
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_history))
            .on_action(cx.listener(Self::close_preview))
            .on_action(
                cx.listener(|this, _: &Refresh, window, cx| this.manual_refresh(window, cx)),
            )
            .on_action(cx.listener(Self::open_branch_picker))
            .on_action(
                cx.listener(|this, _: &ExitReview, window, cx| this.exit_review(window, cx)),
            )
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
            .relative()
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
                        el.when(self.sidebar_visible, |el| el.child(self.sidebar(cx)))
                            .when(self.selected.is_some(), |el| {
                                el.child(self.preview(window, cx))
                            })
                    })
                    .when(self.repo.is_none(), |el| el.child(self.welcome(cx)))
                    .when(self.terminal_visible, |el| {
                        el.child(self.terminal_panel(window, cx))
                    }),
            )
            .when(self.history_visible && self.repo.is_some(), |el| el.child(
                div().flex().flex_col().flex_shrink_0()
                    .h(px(self.history_height.min((f32::from(window.viewport_size().height) - 180.).max(150.))))
                    .min_h_0().overflow_hidden()
                    .child(div().id("history-resize").h(px(4.)).w_full().flex_shrink_0()
                        .cursor_row_resize().bg(rgb(BORDER)).hover(|el| el.bg(rgb(ACCENT)))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            this.history_dragging = true;
                            cx.stop_propagation();
                        })))
                    .child(div().flex_1().min_h_0().child(self.history.clone()))
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .h(px(25.))
                    .flex_shrink_0()
                    .px_4()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child(div().id("toggle-history").flex().items_center().gap_1().h_full().px_2()
                        .cursor_pointer().rounded_sm()
                        .bg(rgb(if self.history_visible { SURFACE } else { PANEL }))
                        .hover(|el| el.bg(rgb(SURFACE)))
                        .tooltip(|window, cx| Tooltip::new("Git history").build(window, cx))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_history(&ToggleHistory, window, cx)))
                        .child(ui_icon("history", 13., if self.history_visible { ACCENT } else { MUTED }))
                        .child("Git"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(if self.error { RED } else { MUTED }))
                            .child(self.notice.clone()),
                    )
                    .child(
                        "Tab focus · ↑↓ / J K move · D deletions · Space / Ctrl+N next change · ⌘⇧R review branch",
                    ),
            )
            .when(self.picker_open, |el| el.child(self.branch_picker(window, cx)))
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}

/// A new right-click starts without the previous row's "Copy path".
fn clear_context_copy(
    this: &mut Give,
    event: &MouseDownEvent,
    _: &mut Window,
    _: &mut Context<Give>,
) {
    if event.button == MouseButton::Right {
        this.context_copy = None;
    }
}

fn load_slot(
    slot: &Slot,
    repo: &Repository,
    file: &ChangedFile,
    review: Option<&Review>,
) -> Result<give::preview_cache::Pair, String> {
    match review {
        Some(review) => slot.load_with(|| review.preview_pair(repo, file)),
        None => slot.load(repo, file),
    }
}

/// Untracked (not yet added to Git) is red, deleted grey, newly added green, otherwise
/// modified blue. Unchanged catalog files keep the plain text color.
fn file_name_color(file: &ChangedFile) -> u32 {
    if file.clean() {
        TEXT
    } else if file.untracked() || file.conflicted() {
        NAME_UNTRACKED
    } else if file.index == 'D' || file.worktree == 'D' {
        NAME_DELETED
    } else if file.index == 'A' {
        NAME_ADDED
    } else {
        NAME_MODIFIED
    }
}

fn brand_icon(side: f32) -> impl IntoElement {
    img(ImageSource::Resource(Resource::Embedded(
        "give-master.png".into(),
    )))
    .size(px(side))
    .object_fit(ObjectFit::Contain)
}

fn empty_state(title: impl Into<SharedString>, detail: impl Into<SharedString>, icon: bool) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .when(icon, |el| el.child(brand_icon(72.)))
        .child(
            div()
                .text_size(px(18.))
                .text_color(rgb(TEXT))
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_center()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(detail.into()),
        )
}

/// A project name over its shortened path, for recent-project lists.
fn recent_project_label(path: &std::path::Path) -> Div {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(ui_icon("folder", 14., MUTED))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT))
                        .truncate()
                        .child(name),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(MUTED))
                        .truncate()
                        .child(recent::display(path)),
                ),
        )
}

fn ui_icon(name: &'static str, side: f32, color: u32) -> impl IntoElement {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(side))
        .flex_shrink_0()
        .text_color(rgb(color))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::{Root, WindowExt};
    // The GPUI glob also exports a `test` macro; the generated attribute needs Rust's.
    use std::prelude::v1::test;

    #[gpui::test]
    fn quick_terminal_button_displays_modal(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::terminal_view::init(cx);
            crate::find_view::init(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Give::new(None, None, window, cx));
            view.update(cx, |view, cx| {
                view.repo = Some(Repository {
                    root: directory.path().to_owned(),
                });
                view.terminal_visible = true;
                view.activate_project_terminal(window, cx);
            });
            Root::new(view, window, cx)
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let button = cx
            .debug_bounds("quick-terminal-launcher")
            .expect("terminal launcher is rendered");
        let main_focus = cx.update(|window, cx| window.focused(cx));
        cx.simulate_click(button.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.update(|window, cx| window.has_active_dialog(cx)),
            "click opens a dialog"
        );
        let modal = cx
            .debug_bounds("quick-terminal-content")
            .expect("quick terminal modal must be rendered after clicking its launcher");
        assert!(modal.size.width > px(400.) && modal.size.height > px(200.));
        assert_ne!(cx.update(|window, cx| window.focused(cx)), main_focus);

        cx.simulate_keystrokes("enter escape");
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
        cx.simulate_keystrokes("cmd-w");
        cx.refresh().unwrap();
        cx.run_until_parked();
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), main_focus);

        cx.simulate_click(button.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
        assert!(cx.debug_bounds("quick-terminal-content").is_some());
        cx.simulate_click(point(px(20.), px(100.)), Modifiers::default());
        cx.run_until_parked();
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), main_focus);
    }
}
