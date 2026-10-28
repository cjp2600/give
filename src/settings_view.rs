//! Global settings editor. Changes are staged until Apply; themes preview immediately.
use give::{
    settings::{self, FontFamily, Settings},
    theme,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Appearance,
    Terminal,
    Workspace,
}

#[derive(Clone, Copy)]
enum Number {
    TerminalFont,
    PreviewFont,
    Columns,
    Refresh,
    Recent,
    History,
}

pub struct SettingsView {
    draft: Settings,
    section: Section,
    error: Option<String>,
    saving: bool,
    path: Option<PathBuf>,
}

pub fn open(window: &mut Window, cx: &mut App) {
    open_at(settings::store(), window, cx);
}

fn restore_theme(cx: &mut App) {
    if let Some(theme) = theme::find(&settings::current().theme) {
        crate::appearance::preview(theme, cx);
    }
}

fn open_at(path: Option<PathBuf>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let view = cx.new(|_| SettingsView {
        draft: settings::current(),
        section: Section::Appearance,
        error: None,
        saving: false,
        path,
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(div().px_5().py_4().child("Settings"))
            .width(px(820.))
            .margin_top(px(24.))
            .p_0()
            .gap_0()
            .overlay_closable(false)
            .child(view.clone())
            .on_close(|_, _, cx| restore_theme(cx))
    });
}

impl SettingsView {
    fn adjust(&mut self, field: Number, delta: i32, cx: &mut Context<Self>) {
        match field {
            Number::TerminalFont => {
                self.draft.terminal_font_size =
                    (self.draft.terminal_font_size + delta as f32).clamp(10., 28.)
            }
            Number::PreviewFont => {
                self.draft.preview_font_size =
                    (self.draft.preview_font_size + delta as f32).clamp(10., 28.)
            }
            Number::Columns => {
                self.draft.terminal_min_columns =
                    (self.draft.terminal_min_columns as i32 + delta * 10).clamp(40, 200) as u16
            }
            Number::Refresh => {
                self.draft.refresh_seconds =
                    (self.draft.refresh_seconds as i32 + delta).clamp(1, 30) as u64
            }
            Number::Recent => {
                self.draft.recent_limit =
                    (self.draft.recent_limit as i32 + delta * 5).clamp(5, 50) as usize
            }
            Number::History => {
                self.draft.history_page_size =
                    (self.draft.history_page_size as i32 + delta * 25).clamp(25, 500) as usize
            }
        }
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.error = Some("The user settings directory is unavailable.".into());
            cx.notify();
            return;
        };
        self.saving = true;
        self.error = None;
        let draft = self.draft.clone();
        let task = cx
            .background_executor()
            .spawn(async move { draft.save(&path).map(|()| draft) });
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(settings) => {
                        crate::appearance::apply(settings, cx);
                        window.close_dialog(cx);
                    }
                    Err(error) => {
                        this.error = Some(format!("Could not save settings: {error:#}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn number(
        &self,
        title: &'static str,
        detail: &'static str,
        field: Number,
        value: String,
        cx: &mut Context<Self>,
    ) -> Div {
        row(title, detail).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new((title, 0_usize))
                        .small()
                        .label("−")
                        .disabled(self.saving)
                        .on_click(cx.listener(move |this, _, _, cx| this.adjust(field, -1, cx))),
                )
                .child(div().w(px(72.)).text_center().child(value))
                .child(
                    Button::new((title, 1_usize))
                        .small()
                        .label("+")
                        .disabled(self.saving)
                        .on_click(cx.listener(move |this, _, _, cx| this.adjust(field, 1, cx))),
                ),
        )
    }

    fn appearance(&self, cx: &mut Context<Self>) -> Div {
        let p = &theme::current().colors;
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section_heading(
                "Appearance",
                "One palette for your workspace, terminal and code.",
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .children(theme::BUILTINS.iter().map(|theme| {
                        let selected = self.draft.theme == theme.id;
                        let id = theme.id.clone();
                        let c = &theme.colors;
                        div()
                            .id(SharedString::from(format!("theme-{}", theme.id)))
                            .debug_selector(move || format!("theme-{}", id))
                            .w(px(166.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(rgb(if selected { p.accent } else { p.border }))
                            .overflow_hidden()
                            .cursor_pointer()
                            .on_click({
                                let theme = theme.clone();
                                cx.listener(move |this, _, _, cx| {
                                    if this.saving {
                                        return;
                                    }
                                    this.draft.theme = theme.id.clone();
                                    crate::appearance::preview(theme.clone(), cx);
                                    cx.notify();
                                })
                            })
                            .child(
                                div()
                                    .h(px(62.))
                                    .px_3()
                                    .py_2()
                                    .bg(rgb(c.background))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(c.syntax_keyword))
                                            .child("fn build() {"),
                                    )
                                    .child(
                                        div().flex().gap_1().children(
                                            [c.accent, c.green, c.syntax_keyword, c.yellow, c.red]
                                                .into_iter()
                                                .map(|color| {
                                                    div()
                                                        .w(px(21.))
                                                        .h(px(5.))
                                                        .rounded(px(2.))
                                                        .bg(rgb(color))
                                                }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(c.syntax_string))
                                            .child("  \"Hello, give\""),
                                    ),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_2()
                                    .bg(rgb(p.panel))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div().flex_1().text_size(px(11.)).child(theme.name.clone()),
                                    )
                                    .when(selected, |el| {
                                        el.child(div().text_color(rgb(p.accent)).child("●"))
                                    }),
                            )
                    })),
            )
            .child(self.number(
                "Preview font size",
                "Source previews and diff text.",
                Number::PreviewFont,
                format!("{} px", self.draft.preview_font_size),
                cx,
            ))
            .child(
                row(
                    "Monospace font",
                    "Shared by the terminal and source previews.",
                )
                .child(div().flex().gap_1().children(
                    FontFamily::ALL.into_iter().enumerate().map(|(i, font)| {
                        Button::new(("font-family", i))
                            .small()
                            .label(font.label())
                            .when(self.draft.font_family == font, |button| button.primary())
                            .disabled(self.saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.draft.font_family = font;
                                cx.notify();
                            }))
                    }),
                )),
            )
    }

    fn terminal(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section_heading(
                "Terminal",
                "Tune readability without restarting your shell.",
            ))
            .child(self.number(
                "Font size",
                "The terminal grid resizes to match.",
                Number::TerminalFont,
                format!("{} px", self.draft.terminal_font_size),
                cx,
            ))
            .child(self.number(
                "Minimum columns",
                "Narrow panels scroll horizontally.",
                Number::Columns,
                format!("{} cols", self.draft.terminal_min_columns),
                cx,
            ))
            .child(
                row("Blinking cursor", "Applies to main and quick terminals.").child(
                    Button::new("cursor-blink")
                        .small()
                        .label(if self.draft.cursor_blink { "On" } else { "Off" })
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.cursor_blink = !this.draft.cursor_blink;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .p_4()
                    .rounded(px(6.))
                    .bg(rgb(theme::current().colors.terminal_background))
                    .font_family(self.draft.font_family.name())
                    .text_size(px(self.draft.terminal_font_size))
                    .text_color(rgb(theme::current().colors.terminal_foreground))
                    .child("$ cargo test\n  All tests passed.\n$ ▋"),
            )
    }

    fn workspace(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section_heading(
                "Workspace",
                "Defaults shared by every project and give instance.",
            ))
            .child(self.number(
                "Refresh interval",
                "Refresh working changes and visible Git history.",
                Number::Refresh,
                format!("{} sec", self.draft.refresh_seconds),
                cx,
            ))
            .child(
                row(
                    "Show deleted lines",
                    "Default source preview mode for all projects.",
                )
                .child(
                    Button::new("show-deletions")
                        .small()
                        .label(if self.draft.show_deletions {
                            "On"
                        } else {
                            "Off"
                        })
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.show_deletions = !this.draft.show_deletions;
                            cx.notify();
                        })),
                ),
            )
            .child(self.number(
                "Recent projects",
                "Number of projects kept in the recent list.",
                Number::Recent,
                self.draft.recent_limit.to_string(),
                cx,
            ))
            .child(self.number(
                "History page size",
                "Commits loaded with each history page.",
                Number::History,
                self.draft.history_page_size.to_string(),
                cx,
            ))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current();
        let p = &theme.colors;
        let height = (f32::from(window.viewport_size().height) - 180.).clamp(300., 500.);
        let content = match self.section {
            Section::Appearance => self.appearance(cx),
            Section::Terminal => self.terminal(cx),
            Section::Workspace => self.workspace(cx),
        };
        div()
            .debug_selector(|| "settings-dialog".into())
            .flex()
            .flex_col()
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .child(
                div()
                    .flex()
                    .h(px(height))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .child(
                        div()
                            .w(px(148.))
                            .flex_shrink_0()
                            .p_3()
                            .bg(rgb(p.panel))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .px_2()
                                    .py_2()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .child("PREFERENCES"),
                            )
                            .children(
                                [
                                    (Section::Appearance, "Appearance"),
                                    (Section::Terminal, "Terminal"),
                                    (Section::Workspace, "Workspace"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(i, (section, label))| {
                                    div()
                                        .id(("settings-section", i))
                                        .debug_selector(move || format!("settings-section-{i}"))
                                        .px_3()
                                        .py_2()
                                        .rounded(px(5.))
                                        .cursor_pointer()
                                        .bg(rgb(if self.section == section {
                                            p.selected
                                        } else {
                                            p.panel
                                        }))
                                        .hover(|el| el.bg(rgb(p.surface)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.section = section;
                                            cx.notify();
                                        }))
                                        .child(label)
                                }),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .px_2()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .child("GLOBAL SETTINGS\nAll projects · All instances"),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-scroll")
                            .debug_selector(|| "settings-scroll".into())
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .p_5()
                            .bg(rgb(p.background))
                            .child(content),
                    ),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().px_5().py_2().text_color(rgb(p.red)).child(error)),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .px_5()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("reset-settings")
                            .ghost()
                            .small()
                            .label("Restore defaults")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft = Settings::default();
                                crate::appearance::preview(
                                    theme::find(&this.draft.theme).unwrap(),
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted))
                            .child("Saved for the whole application"),
                    )
                    .child(
                        Button::new("cancel-settings")
                            .small()
                            .label("Cancel")
                            .disabled(self.saving)
                            .on_click(|_, window, cx| {
                                restore_theme(cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        div().debug_selector(|| "apply-settings".into()).child(
                            Button::new("apply-settings")
                                .primary()
                                .small()
                                .label(if self.saving { "Saving…" } else { "Apply" })
                                .disabled(self.saving)
                                .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                        ),
                    ),
            )
    }
}

fn section_heading(title: &'static str, detail: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(theme::current().colors.muted))
                .child(detail),
        )
}
fn row(title: &'static str, detail: &'static str) -> Div {
    div().flex().items_center().justify_between().gap_3().child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(title)
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(theme::current().colors.muted))
                    .child(detail),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::Root;
    use std::prelude::v1::test;

    #[gpui::test]
    fn theme_preview_can_be_cancelled_and_apply_saves_global_preferences(cx: &mut TestAppContext) {
        let original = settings::current();
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::terminal_view::init(cx);
            crate::find_view::init(cx);
            crate::appearance::apply(original.clone(), cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("settings.json");
        let (_, cx) = cx.add_window_view(|window, cx| {
            let give = cx.new(|cx| crate::ui::Give::new(None, None, window, cx));
            Root::new(give, window, cx)
        });
        cx.simulate_resize(size(px(900.), px(600.)));
        cx.update(|window, cx| open_at(Some(file.clone()), window, cx));
        cx.refresh().unwrap();
        cx.run_until_parked();
        assert!(cx.debug_bounds("settings-dialog").is_some());
        let nord = cx.debug_bounds("theme-nord").unwrap();
        cx.simulate_click(nord.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(theme::current().id, "nord");
        assert_eq!(
            settings::current(),
            original,
            "preview does not commit preferences"
        );
        assert!(!file.exists());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        assert_eq!(theme::current().id, original.theme);

        cx.update(|window, cx| open_at(Some(file.clone()), window, cx));
        cx.refresh().unwrap();
        cx.run_until_parked();
        let latte = cx.debug_bounds("theme-catppuccin-latte").unwrap();
        cx.simulate_click(latte.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(!theme::current().dark);
        let apply = cx.debug_bounds("apply-settings").unwrap();
        cx.simulate_click(apply.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        let saved = Settings::load(&file).unwrap();
        assert_eq!(saved.theme, "catppuccin-latte");
        assert_eq!(settings::current(), saved);

        // Newly added themes below the fold remain selectable in a small window.
        cx.update(|window, cx| open_at(Some(file.clone()), window, cx));
        cx.refresh().unwrap();
        cx.run_until_parked();
        let gallery = cx.debug_bounds("settings-scroll").unwrap();
        for _ in 0..5 {
            if cx
                .debug_bounds("theme-github-light")
                .is_some_and(|bounds| gallery.contains(&bounds.center()))
            {
                break;
            }
            cx.simulate_event(ScrollWheelEvent {
                position: gallery.center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            });
            cx.run_until_parked();
        }
        let github = cx.debug_bounds("theme-github-light").unwrap();
        assert!(gallery.contains(&github.center()));
        cx.simulate_click(github.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(theme::current().id, "github-light");
        assert!(!theme::current().dark);
        assert_eq!(
            settings::current(),
            saved,
            "selection remains a preview until Apply"
        );
        let apply = cx.debug_bounds("apply-settings").unwrap();
        cx.simulate_click(apply.center(), Modifiers::default());
        cx.run_until_parked();
        let saved = Settings::load(&file).unwrap();
        assert_eq!(saved.theme, "github-light");
        assert_eq!(settings::current(), saved);

        // Another instance writes the same file; this running app must follow it.
        cx.update(|_, cx| crate::appearance::watch(file.clone(), cx));
        cx.run_until_parked();
        let external = Settings {
            theme: "nord".into(),
            terminal_font_size: 18.,
            ..saved
        };
        external.save(&file).unwrap();
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(settings::current(), external);
        assert_eq!(theme::current().id, "nord");
        std::fs::write(&file, r#"{"refresh_seconds":0}"#).unwrap();
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(
            settings::current(),
            external,
            "invalid external edits preserve the last valid settings"
        );
        cx.update(|_, cx| crate::appearance::apply(original, cx));
    }
}
