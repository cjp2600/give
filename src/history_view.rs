use super::{
    ACCENT, BORDER, MUTED, NAME_ADDED, NAME_DELETED, NAME_MODIFIED, PANEL, RED, SURFACE, TEXT,
    ui_icon,
};
use crate::{CopyPath, FirstRow, LastRow, NextFile, PageDown, PageUp, PreviousFile};
use give::{
    git::Repository,
    history::{self, Commit, Details, Page},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    tooltip::Tooltip,
};
use std::{path::PathBuf, time::Duration};

const PAGE_SIZE: usize = 100;

/// Owns asynchronous history reads independently of the working-tree preview.
/// A generation guards project/ref switches; an ID guards commit detail reads.
pub(super) struct HistoryPanel {
    target: Option<(PathBuf, String, String)>,
    visible: bool,
    generation: u64,
    limit: usize,
    page: Page,
    selected: Option<String>,
    details: Option<Details>,
    loading: bool,
    details_loading: bool,
    error: Option<String>,
    details_error: Option<String>,
    scroll: UniformListScrollHandle,
    files_scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl HistoryPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if view.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            target: None,
            visible: false,
            generation: 0,
            limit: PAGE_SIZE,
            page: Page::default(),
            selected: None,
            details: None,
            loading: false,
            details_loading: false,
            error: None,
            details_error: None,
            scroll: UniformListScrollHandle::new(),
            files_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn configure(
        &mut self,
        target: Option<(PathBuf, String, String)>,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self.target != target;
        let opening = visible && !self.visible;
        self.visible = visible;
        if changed {
            self.generation += 1;
            self.target = target;
            self.limit = PAGE_SIZE;
            self.page = Page::default();
            self.selected = None;
            self.details = None;
            self.loading = false;
            self.details_loading = false;
            self.error = None;
            self.details_error = None;
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
        if changed || opening {
            self.refresh(cx);
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.visible || self.loading {
            return;
        }
        let Some((root, revision, _)) = self.target.clone() else {
            return;
        };
        self.loading = true;
        let generation = self.generation;
        let limit = self.limit;
        let task = cx
            .background_executor()
            .spawn(async move { history::load(&Repository { root }, &revision, limit) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page) => {
                        this.error = None;
                        if this.page != page {
                            this.page = page;
                            if !this
                                .page
                                .commits
                                .iter()
                                .any(|commit| Some(&commit.id) == this.selected.as_ref())
                            {
                                this.selected = None;
                                this.details = None;
                                this.details_error = None;
                                this.details_loading = false;
                                if !this.page.commits.is_empty() {
                                    this.select(0, cx);
                                }
                            }
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(commit) = self.page.commits.get(index).cloned() else {
            return;
        };
        if self.selected.as_ref() == Some(&commit.id) {
            return;
        }
        let Some((root, _, _)) = self.target.clone() else {
            return;
        };
        self.selected = Some(commit.id.clone());
        self.details = None;
        self.details_error = None;
        self.details_loading = true;
        self.files_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let generation = self.generation;
        let id = commit.id.clone();
        let task = cx
            .background_executor()
            .spawn(async move { history::details(&Repository { root }, &commit) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if generation != this.generation || this.selected.as_ref() != Some(&id) {
                    return;
                }
                this.details_loading = false;
                match result {
                    Ok(details) => this.details = Some(details),
                    Err(error) => this.details_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn navigate(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.page.commits.is_empty() {
            return;
        }
        let index = self
            .page
            .commits
            .iter()
            .position(|c| Some(&c.id) == self.selected.as_ref())
            .unwrap_or(0);
        let next = index
            .saturating_add_signed(step)
            .min(self.page.commits.len() - 1);
        self.select(next, cx);
        self.scroll.scroll_to_item(next, ScrollStrategy::Center);
    }

    fn commit_row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let commit = &self.page.commits[index];
        let selected = self.selected.as_ref() == Some(&commit.id);
        let merge = commit.parents.len() > 1;
        let tooltip = format!(
            "{}\n{} <{}>\n{}\n{}{}",
            commit.subject,
            commit.author,
            commit.email,
            commit.date,
            commit.id,
            if merge { "\nMerge commit" } else { "" }
        );
        div()
            .id(("commit", index))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .h(px(30.))
            .bg(rgb(if selected { 0x2b425a } else { PANEL }))
            .hover(|el| el.bg(rgb(SURFACE)))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focus.focus(window);
                this.select(index, cx);
            }))
            .child(
                div()
                    .w(px(16.))
                    .flex_shrink_0()
                    .text_color(rgb(ACCENT))
                    .child(if merge {
                        ui_icon("branch", 13., ACCENT).into_any_element()
                    } else {
                        div().text_size(px(14.)).child("•").into_any_element()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(commit.subject.clone()),
            )
            .when(!commit.references.is_empty(), |el| {
                el.child(
                    div()
                        .max_w(px(145.))
                        .px_1()
                        .rounded_sm()
                        .bg(rgb(SURFACE))
                        .text_color(rgb(ACCENT))
                        .text_size(px(10.))
                        .truncate()
                        .child(commit.references.clone()),
                )
            })
            .child(
                div()
                    .w(px(115.))
                    .flex_shrink_0()
                    .truncate()
                    .text_color(rgb(MUTED))
                    .child(commit.author.clone()),
            )
            .child(
                div()
                    .w(px(98.))
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child(short_date(commit)),
            )
            .child(
                div()
                    .w(px(57.))
                    .flex_shrink_0()
                    .font_family("Menlo")
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child(short_id(&commit.id)),
            )
    }

    fn detail_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let commit = self
            .page
            .commits
            .iter()
            .find(|c| Some(&c.id) == self.selected.as_ref());
        div()
            .flex()
            .flex_col()
            .w(relative(0.36))
            .min_w(px(260.))
            .h_full()
            .border_l_1()
            .border_color(rgb(BORDER))
            .when_some(commit, |el, commit| {
                let identity = format!("{} <{}> · {}", commit.author, commit.email, commit.date);
                let id = commit.id.clone();
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .text_color(rgb(ACCENT))
                                        .child(short_id(&id)),
                                )
                                .child(
                                    Button::new("copy-commit")
                                        .ghost()
                                        .small()
                                        .label("Copy hash")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                id.clone(),
                                            ))
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(MUTED))
                                .truncate()
                                .child(identity),
                        )
                        .when(commit.parents.len() > 1, |el| {
                            el.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(ACCENT))
                                    .child("Merge · changes against first parent"),
                            )
                        }),
                )
            })
            .when_some(self.details.as_ref(), |el, details| {
                el.child(
                    div()
                        .id("commit-message")
                        .max_h(px(76.))
                        .overflow_y_scroll()
                        .px_3()
                        .py_2()
                        .text_size(px(11.))
                        .child(details.message.clone()),
                )
                .child(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(10.))
                        .text_color(rgb(MUTED))
                        .child(format!("CHANGED FILES · {}", details.files.len())),
                )
                .child(
                    uniform_list(
                        "commit-files",
                        details.files.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                            let Some(details) = &this.details else {
                                return Vec::new();
                            };
                            range
                                .map(|index| {
                                    let file = &details.files[index];
                                    let color = match file.status {
                                        'A' => NAME_ADDED,
                                        'D' => NAME_DELETED,
                                        _ => NAME_MODIFIED,
                                    };
                                    let path = file.path.to_string_lossy().into_owned();
                                    let label = file
                                        .original_path
                                        .as_ref()
                                        .map(|old| format!("{} → {path}", old.display()))
                                        .unwrap_or(path);
                                    div()
                                        .id(("commit-file", index))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_3()
                                        .h(px(27.))
                                        .tooltip({
                                            let label = label.clone();
                                            move |window, cx| {
                                                Tooltip::new(label.clone()).build(window, cx)
                                            }
                                        })
                                        .child(
                                            div()
                                                .w(px(12.))
                                                .flex_shrink_0()
                                                .text_color(rgb(color))
                                                .child(file.status.to_string()),
                                        )
                                        .child(ui_icon("file", 13., MUTED))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(color))
                                                .child(label),
                                        )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(self.files_scroll.clone())
                    .flex_1()
                    .min_h_0(),
                )
                .when(details.files.is_empty(), |el| {
                    el.child(message("No file changes in this commit."))
                })
            })
            .when(self.details_loading, |el| {
                el.child(message("Loading changed files…"))
            })
            .when_some(self.details_error.as_ref(), |el, error| {
                el.child(message(error.clone()).text_color(rgb(RED)))
            })
            .when(commit.is_none(), |el| {
                el.child(message("Select a commit to see its files."))
            })
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .target
            .as_ref()
            .map(|(_, _, label)| label.clone())
            .unwrap_or_default();
        div()
            .key_context("History")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(rgb(PANEL))
            .text_color(rgb(TEXT))
            .text_size(px(11.))
            .on_action(cx.listener(|this, _: &NextFile, _, cx| this.navigate(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousFile, _, cx| this.navigate(-1, cx)))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.navigate(8, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.navigate(-8, cx)))
            .on_action(cx.listener(|this, _: &FirstRow, _, cx| this.navigate(isize::MIN, cx)))
            .on_action(cx.listener(|this, _: &LastRow, _, cx| this.navigate(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| {
                if let Some(id) = &this.selected {
                    cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                }
            }))
            .child(
                div()
                    .h(px(28.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .text_color(rgb(MUTED))
                    .child(ui_icon("branch", 13., ACCENT))
                    .child(div().text_color(rgb(TEXT)).child("HISTORY"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(ACCENT))
                            .child(label),
                    )
                    .child(
                        Button::new("refresh-history")
                            .ghost()
                            .small()
                            .label("Refresh")
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("close-history")
                            .ghost()
                            .small()
                            .label("×")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(crate::ToggleHistory), cx)
                            }),
                    ),
            )
            .when_some(self.error.as_ref(), |el, error| {
                el.child(message(error.clone()).text_color(rgb(RED)))
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .when(self.page.commits.is_empty(), |el| {
                                el.child(message(if self.target.is_none() {
                                    "Waiting for branch review…"
                                } else if self.loading {
                                    "Loading commits…"
                                } else {
                                    "No commits yet."
                                }))
                            })
                            .when(!self.page.commits.is_empty(), |el| {
                                el.child(
                                    uniform_list(
                                        "history-commits",
                                        self.page.commits.len(),
                                        cx.processor(
                                            |this, range: std::ops::Range<usize>, _, cx| {
                                                range
                                                    .map(|index| this.commit_row(index, cx))
                                                    .collect::<Vec<_>>()
                                            },
                                        ),
                                    )
                                    .track_scroll(self.scroll.clone())
                                    .flex_1()
                                    .min_h_0(),
                                )
                            })
                            .when(self.page.has_more, |el| {
                                el.child(
                                    div()
                                        .h(px(28.))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .border_t_1()
                                        .border_color(rgb(BORDER))
                                        .child(
                                            Button::new("older-commits")
                                                .ghost()
                                                .small()
                                                .label(if self.loading {
                                                    "Loading…"
                                                } else {
                                                    "Load older commits"
                                                })
                                                .disabled(self.loading)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.limit += PAGE_SIZE;
                                                    this.refresh(cx);
                                                })),
                                        ),
                                )
                            }),
                    )
                    .child(self.detail_panel(cx)),
            )
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}
fn short_date(commit: &Commit) -> String {
    commit.date.chars().take(16).collect()
}
fn message(text: impl Into<SharedString>) -> Div {
    div()
        .px_3()
        .py_2()
        .text_color(rgb(MUTED))
        .child(text.into())
}
