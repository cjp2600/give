use crate::{Commit, CopyDiff, NextFile, OpenRepository, PreviousFile, Refresh, ToggleLayer};
use give::git::{ChangedFile, Diff, Layer, LineKind, Repository, Snapshot};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use std::{path::PathBuf, time::Duration};

const BACKGROUND: u32 = 0x141719;
const PANEL: u32 = 0x1a1e21;
const SURFACE: u32 = 0x22272b;
const BORDER: u32 = 0x30373c;
const TEXT: u32 = 0xe2e7e9;
const MUTED: u32 = 0x86949c;
const ACCENT: u32 = 0xf5aa57;
const GREEN: u32 = 0x9bd5b3;
const RED: u32 = 0xe99b9b;

pub struct Give {
    repo: Option<Repository>,
    snapshot: Snapshot,
    layer: Layer,
    selected: Option<PathBuf>,
    diff: Diff,
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
                    this.reconcile_selection();
                    this.load_diff(false, cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&commit_input, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut this = Self {
            repo: None,
            snapshot: Snapshot::default(),
            layer: Layer::Working,
            selected: None,
            diff: Diff::default(),
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
            refreshing: false,
            preview_loading: false,
            epoch: 0,
            snapshot_request: 0,
            preview_request: 0,
            _subscriptions: subscriptions,
        };
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

    fn visible_files(&self) -> Vec<ChangedFile> {
        self.snapshot
            .files
            .iter()
            .filter(|file| {
                (if self.layer == Layer::Working {
                    file.unstaged()
                } else {
                    file.staged()
                }) && (self.filter.is_empty()
                    || file
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&self.filter))
            })
            .cloned()
            .collect()
    }

    fn reconcile_selection(&mut self) {
        let files = self.visible_files();
        if !files
            .iter()
            .any(|file| Some(&file.path) == self.selected.as_ref())
        {
            self.selected = files.first().map(|file| file.path.clone());
            self.diff_scroll = UniformListScrollHandle::new();
        }
    }

    fn open_picker(&mut self, _: &OpenRepository, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let result = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        cx.spawn_in(window, async move |view, cx| match result.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = view.update_in(cx, |this, window, cx| this.open(path, window, cx));
                }
            }
            Ok(Err(error)) => {
                let _ = view.update(cx, |this, cx| this.show_error(error, cx));
            }
            _ => {}
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
                        this.snapshot = snapshot;
                        this.selected = None;
                        this.filter.clear();
                        this.filter_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.commit_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.reconcile_selection();
                        this.load_diff(false, cx);
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
                        this.snapshot = snapshot;
                        this.reconcile_selection();
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
        self.preview_request += 1;
        let request = self.preview_request;
        let epoch = self.epoch;
        let file = self
            .snapshot
            .files
            .iter()
            .find(|file| Some(&file.path) == self.selected.as_ref())
            .cloned();
        let (Some(repo), Some(file)) = (self.repo.clone(), file) else {
            self.diff = Diff::default();
            self.preview_loading = false;
            return;
        };
        let layer = self.layer;
        if !quiet {
            self.diff = Diff::default();
            self.preview_loading = true;
        }
        let task = cx
            .background_executor()
            .spawn(async move { repo.diff(&file, layer) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if epoch != this.epoch || request != this.preview_request {
                    return;
                }
                this.preview_loading = false;
                match result {
                    Ok(diff) => this.diff = diff,
                    Err(error) => {
                        this.diff = Diff {
                            notice: Some(format!("{error:#}")),
                            ..Diff::default()
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&path) {
            self.selected = Some(path);
            self.diff_scroll = UniformListScrollHandle::new();
            self.load_diff(false, cx);
        }
        self.focus.focus(window);
        cx.notify();
    }

    fn change_layer(&mut self, layer: Layer, cx: &mut Context<Self>) {
        if self.layer != layer {
            self.layer = layer;
            self.files_scroll = UniformListScrollHandle::new();
            self.reconcile_selection();
            self.diff_scroll = UniformListScrollHandle::new();
            self.load_diff(false, cx);
            cx.notify();
        }
    }

    fn navigate(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let files = self.visible_files();
        if files.is_empty() {
            return;
        }
        let index = files
            .iter()
            .position(|file| Some(&file.path) == self.selected.as_ref())
            .unwrap_or(0);
        let index = index.saturating_add_signed(offset).min(files.len() - 1);
        self.files_scroll.scroll_to_item(index, ScrollStrategy::Top);
        self.select(files[index].path.clone(), window, cx);
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
        let layer = self.layer;
        self.start_operation(
            if layer == Layer::Working {
                "Staging changes…"
            } else {
                "Unstaging changes…"
            },
            cx,
        );
        let task = cx.background_executor().spawn(async move {
            if layer == Layer::Working {
                repo.stage(file.as_ref())?;
            } else {
                repo.unstage(file.as_ref())?;
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
                        this.reconcile_selection();
                        this.load_diff(false, cx);
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

    fn copy_diff(&mut self, _: &CopyDiff, _: &mut Window, cx: &mut Context<Self>) {
        if !self.diff.raw.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.diff.raw.clone()));
            self.notice = "Diff copied to clipboard".into();
            self.error = false;
            cx.notify();
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .repo
            .as_ref()
            .and_then(|repo| repo.root.file_name())
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(64.))
            .px_5()
            .bg(rgb(PANEL))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(brand_icon(34.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(16.))
                                    .child(if name.is_empty() {
                                        "give".to_owned()
                                    } else {
                                        name
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child("A little room for better changes"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(self.repo.is_some(), |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(SURFACE))
                                .text_color(rgb(ACCENT))
                                .child(format!("⑂  {}", self.snapshot.branch)),
                        )
                    })
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .small()
                            .label("Refresh")
                            .disabled(self.repo.is_none() || self.busy || self.opening)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("open")
                            .small()
                            .label("Open repository")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_picker(&OpenRepository, window, cx)
                            })),
                    ),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let working = self
            .snapshot
            .files
            .iter()
            .filter(|file| file.unstaged())
            .count();
        let staged = self
            .snapshot
            .files
            .iter()
            .filter(|file| file.staged())
            .count();
        let files = self.visible_files();
        let has_visible = !files.is_empty();
        let staged_layer = self.layer == Layer::Staged;
        let can_commit = staged > 0
            && !self.busy
            && !self.opening
            && !self.commit_input.read(cx).value().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .w(px(310.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("working")
                                    .small()
                                    .when(!staged_layer, |el| el.primary())
                                    .label(format!("Working  {working}"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.change_layer(Layer::Working, cx)
                                    })),
                            )
                            .child(
                                Button::new("staged")
                                    .small()
                                    .when(staged_layer, |el| el.primary())
                                    .label(format!("Staged  {staged}"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.change_layer(Layer::Staged, cx)
                                    })),
                            ),
                    )
                    .child(Input::new(&self.filter_input).small()),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_color(rgb(MUTED))
                    .text_size(px(10.))
                    .child("CHANGED FILES")
                    .child(
                        Button::new("all")
                            .ghost()
                            .xsmall()
                            .label(if staged_layer {
                                "Unstage all"
                            } else {
                                "Stage all"
                            })
                            .disabled(
                                self.busy
                                    || self.opening
                                    || if staged_layer {
                                        staged == 0
                                    } else {
                                        working == 0
                                    },
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.mutate(true, window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(has_visible, |el| {
                        el.child(
                            uniform_list(
                                "files",
                                files.len(),
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|index| {
                                            let file = &files[index];
                                            let path = file.path.clone();
                                            let selected = this.selected.as_ref() == Some(&path);
                                            let name = path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .to_string();
                                            let parent = path
                                                .parent()
                                                .filter(|p| !p.as_os_str().is_empty())
                                                .map(|p| p.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "repository root".into());
                                            div()
                                                .id(index)
                                                .h(px(54.))
                                                .px_4()
                                                .flex()
                                                .items_center()
                                                .gap_3()
                                                .cursor_pointer()
                                                .bg(rgb(if selected { 0x30302b } else { PANEL }))
                                                .border_l_2()
                                                .border_color(rgb(if selected {
                                                    ACCENT
                                                } else {
                                                    PANEL
                                                }))
                                                .hover(|el| el.bg(rgb(SURFACE)))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.select(path.clone(), window, cx)
                                                    },
                                                ))
                                                .child(
                                                    div()
                                                        .text_size(px(12.))
                                                        .font_family("Menlo")
                                                        .text_color(rgb(if file.conflicted() {
                                                            RED
                                                        } else if file.untracked() || staged_layer {
                                                            GREEN
                                                        } else {
                                                            ACCENT
                                                        }))
                                                        .child(
                                                            file.status(staged_layer).to_string(),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .truncate()
                                                                .child(name),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(10.))
                                                                .text_color(rgb(MUTED))
                                                                .truncate()
                                                                .child(parent),
                                                        ),
                                                )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(self.files_scroll.clone())
                            .h_full(),
                        )
                    })
                    .when(!has_visible, |el| {
                        el.child(div().p_5().text_size(px(12.)).text_color(rgb(MUTED)).child(
                            if self.filter.is_empty() {
                                if staged_layer {
                                    "No staged changes yet"
                                } else {
                                    "No working changes"
                                }
                            } else {
                                "No matching files"
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(px(11.))
                            .child("READY TO COMMIT")
                            .child(
                                div()
                                    .text_color(rgb(MUTED))
                                    .child(format!("{staged} staged")),
                            ),
                    )
                    .child(Input::new(&self.commit_input).small().disabled(self.busy))
                    .child(
                        Button::new("commit")
                            .primary()
                            .label(if self.busy {
                                "Working…"
                            } else {
                                "Commit staged changes"
                            })
                            .disabled(!can_commit)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.commit(&Commit, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(MUTED))
                            .child("Only staged changes enter the commit."),
                    ),
            )
    }

    fn preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected.is_some();
        let path = self
            .selected
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| "Your changes, in focus".into());
        let staged = self.layer == Layer::Staged;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(BACKGROUND))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_5()
                    .h(px(55.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .truncate()
                            .child(path),
                    )
                    .when(selected, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .text_color(rgb(GREEN))
                                        .child(format!("+{}", self.diff.additions)),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(RED))
                                        .child(format!("−{}", self.diff.deletions)),
                                )
                                .child(
                                    Button::new("copy")
                                        .ghost()
                                        .small()
                                        .label("Copy diff")
                                        .disabled(self.diff.raw.is_empty())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.copy_diff(&CopyDiff, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("stage-file")
                                        .small()
                                        .label(if staged { "Unstage file" } else { "Stage file" })
                                        .disabled(self.busy || self.opening)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.mutate(false, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .when(self.preview_loading, |el| {
                el.child(empty_state(
                    "Loading changes…",
                    "Reading the local repository",
                    false,
                ))
            })
            .when(!self.preview_loading && !selected, |el| {
                el.child(empty_state(
                    if self.filter.is_empty() {
                        "A clear view. A clean slate."
                    } else {
                        "Nothing matches this filter"
                    },
                    if staged {
                        "Stage files from Working to prepare your next commit."
                    } else {
                        "Changes will appear here as you or your agent work."
                    },
                    true,
                ))
            })
            .when(!self.preview_loading && selected, |el| {
                el.when_some(self.diff.notice.clone(), |el, notice| {
                    el.child(
                        div()
                            .px_5()
                            .py_3()
                            .text_size(px(11.))
                            .text_color(rgb(ACCENT))
                            .bg(rgb(0x27241f))
                            .child(notice),
                    )
                })
                .child(
                    uniform_list(
                        "diff",
                        self.diff.lines.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                            range
                                .map(|index| {
                                    let line = &this.diff.lines[index];
                                    let (bg, fg) = match line.kind {
                                        LineKind::Added => (0x182b24, GREEN),
                                        LineKind::Removed => (0x302023, RED),
                                        LineKind::Hunk => (0x202831, 0x9abddd),
                                        LineKind::Header => (BACKGROUND, MUTED),
                                        LineKind::Context => (BACKGROUND, TEXT),
                                    };
                                    div()
                                        .h(px(23.))
                                        .min_w_full()
                                        .flex()
                                        .items_center()
                                        .bg(rgb(bg))
                                        .text_color(rgb(fg))
                                        .font_family("Menlo")
                                        .text_size(px(12.))
                                        .line_height(px(23.))
                                        .child(
                                            div()
                                                .w(px(48.))
                                                .flex_shrink_0()
                                                .text_right()
                                                .pr_2()
                                                .text_color(rgb(MUTED))
                                                .child(
                                                    line.old
                                                        .map(|n| n.to_string())
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .w(px(48.))
                                                .flex_shrink_0()
                                                .text_right()
                                                .pr_3()
                                                .text_color(rgb(MUTED))
                                                .child(
                                                    line.new
                                                        .map(|n| n.to_string())
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .whitespace_nowrap()
                                                .pr_6()
                                                .child(line.text.clone()),
                                        )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .track_scroll(self.diff_scroll.clone())
                    .flex_1()
                    .min_h_0(),
                )
            })
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Give")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open_picker))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::copy_diff))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &NextFile, window, cx| this.navigate(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousFile, window, cx| this.navigate(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleLayer, _, cx| {
                this.change_layer(
                    if this.layer == Layer::Working {
                        Layer::Staged
                    } else {
                        Layer::Working
                    },
                    cx,
                )
            }))
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
                    .when(self.repo.is_none(), |el| el.child(self.welcome(cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .min_h(px(30.))
                    .px_4()
                    .py_2()
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
                    .child("⌘O open  ·  ⌘R refresh  ·  ⌘↵ commit"),
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
