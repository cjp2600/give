//! The message composer that stands in for an agent's own terminal prompt. It owns
//! the draft and its images; the terminal view types the finished message.
use give::{
    prompt::{self, Command},
    rcli::Agent,
    theme,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
};
use std::{path::PathBuf, time::Duration};

actions!(prompt_bar, [Submit, Newline, PasteIntoPrompt, Interrupt]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some("PromptBar > Input")),
        KeyBinding::new("shift-enter", Newline, Some("PromptBar > Input")),
        KeyBinding::new("alt-enter", Newline, Some("PromptBar > Input")),
        KeyBinding::new("secondary-v", PasteIntoPrompt, Some("PromptBar > Input")),
        KeyBinding::new("escape", Interrupt, Some("PromptBar > Input")),
    ]);
}

pub struct Message {
    pub text: String,
    pub images: Vec<PathBuf>,
}

pub enum Event {
    Submit(Message),
    /// A command picked from the menu, to type as is.
    Command(&'static Command),
    /// Escape: the agent's own interrupt.
    Interrupt,
    /// Ask to hide the bar and hand the keyboard to the terminal.
    Collapse,
}

/// Dragged by the bar's top edge to resize its input area.
struct ResizePrompt;

pub struct PromptBar {
    input: Entity<InputState>,
    project: PathBuf,
    agent: Agent,
    /// The input area's height dragged for this project and agent.
    height: Option<f32>,
    /// From the pointer to the input area's top, measured when a resize starts.
    resize_offset: Option<Pixels>,
    _save: Task<()>,
    placeholder_for: Option<Agent>,
    images: Vec<PathBuf>,
    /// Clipboard images still being written to disk.
    saving: usize,
    sending: bool,
    error: Option<String>,
    _subscription: Subscription,
}

impl EventEmitter<Event> for PromptBar {}

impl PromptBar {
    pub fn new(project: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).auto_grow(1, 8));
        let subscription = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let agent = Agent::Claude;
        let height = Self::stored_height(&project, agent);
        Self {
            input,
            project,
            agent,
            height,
            resize_offset: None,
            _save: Task::ready(()),
            placeholder_for: None,
            images: Vec::new(),
            saving: 0,
            sending: false,
            error: None,
            _subscription: subscription,
        }
    }

    pub fn set_agent(&mut self, agent: Agent, cx: &mut Context<Self>) {
        if self.agent != agent {
            self.agent = agent;
            self.height = Self::stored_height(&self.project, agent);
            cx.notify();
        }
    }

    fn stored_height(project: &std::path::Path, agent: Agent) -> Option<f32> {
        prompt::Heights::store().and_then(|file| prompt::Heights::load(&file).get(project, agent))
    }

    /// The input area's least height: the dragged one, else two rows for Codex, whose
    /// own prompt sits taller than Claude's.
    fn min_height(&self, window: &Window) -> Option<Pixels> {
        self.height.map(px).or(match self.agent {
            Agent::Claude => None,
            Agent::Codex => Some(px(20.) + window.rem_size() * 2.5),
        })
    }

    /// Follows the pointer dragging the top edge; the bar stays anchored at the bottom.
    fn resize(
        &mut self,
        event: &DragMoveEvent<ResizePrompt>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let area = event.bounds;
        let offset = *self
            .resize_offset
            .get_or_insert(area.top() - event.event.position.y);
        let limit = (f32::from(window.viewport_size().height) * 0.6).max(60.);
        let height = f32::from(area.bottom() - event.event.position.y - offset).clamp(40., limit);
        if self.height != Some(height) {
            self.height = Some(height);
            self.save_height(cx);
            cx.notify();
        }
    }

    /// Double-clicking the edge restores the agent's default height.
    fn reset_height(&mut self, cx: &mut Context<Self>) {
        if self.height.take().is_some() {
            self.save_height(cx);
            cx.notify();
        }
    }

    /// Stores the height once dragging pauses, merged into the file other projects share.
    fn save_height(&mut self, cx: &mut Context<Self>) {
        let (project, agent, height) = (self.project.clone(), self.agent, self.height);
        self._save = cx.background_executor().spawn({
            let executor = cx.background_executor().clone();
            async move {
                executor.timer(Duration::from_millis(300)).await;
                let Some(file) = prompt::Heights::store() else {
                    return;
                };
                let mut heights = prompt::Heights::load(&file);
                heights.set(&project, agent, height);
                let _ = heights.save(&file);
            }
        });
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    }

    /// The terminal has typed the last message; accept the next one.
    pub fn finish_sending(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.sending = false;
        self.error = error;
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim_end().to_owned();
        if self.sending || self.saving > 0 || text.trim().is_empty() && self.images.is_empty() {
            return;
        }
        self.sending = true;
        self.error = None;
        let message = Message {
            text,
            images: std::mem::take(&mut self.images),
        };
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.emit(Event::Submit(message));
        cx.notify();
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.insert("\n", window, cx));
    }

    /// Images become chips saved as files; text goes into the draft.
    fn paste(&mut self, _: &PasteIntoPrompt, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let text = item.text();
        let images = item
            .into_entries()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image),
                _ => None,
            })
            .collect::<Vec<_>>();
        if images.is_empty() {
            if let Some(text) = text {
                self.input
                    .update(cx, |input, cx| input.insert(text, window, cx));
            }
            return;
        }
        self.saving += 1;
        let task = cx.background_executor().spawn(async move {
            give::clipboard_file::save(crate::terminal_view::clipboard_images(images))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.saving -= 1;
                match result {
                    Ok(paths) => this.images.extend(paths),
                    Err(error) => this.error = Some(format!("Clipboard: {error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_image(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.images.len() {
            self.images.remove(index);
            cx.notify();
        }
    }

    fn render_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &theme::current().colors;
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .px_3()
            .pt_2()
            .children(self.images.iter().enumerate().map(|(index, path)| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                div()
                    .id(("prompt-image", index))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(22.))
                    .pl_2()
                    .rounded_md()
                    .bg(rgb(p.secondary))
                    .border_1()
                    .border_color(rgb(p.border))
                    .text_size(px(11.))
                    .text_color(rgb(p.text))
                    .tooltip({
                        let path = path.to_string_lossy().into_owned();
                        move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(path.clone()).build(window, cx)
                        }
                    })
                    .child(
                        div()
                            .text_color(rgb(p.accent))
                            .child(format!("Image #{}", index + 1)),
                    )
                    .child(div().text_color(rgb(p.muted)).child(name))
                    .child(
                        Button::new(("remove-prompt-image", index))
                            .ghost()
                            .xsmall()
                            .label("×")
                            .tooltip("Remove image")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_image(index, cx)),
                            ),
                    )
            }))
            .when(self.saving > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(22.))
                        .px_2()
                        .rounded_md()
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(p.border))
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child("Saving image…"),
                )
            })
    }
}

impl Render for PromptBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &theme::current().colors;
        let agent = self.agent;
        let min_height = self.min_height(window);
        if self.placeholder_for != Some(agent) {
            self.placeholder_for = Some(agent);
            self.input.update(cx, |input, cx| {
                input.set_placeholder(format!("Message {}…", agent.label()), window, cx)
            });
        }
        let focused = self.is_focused(window, cx);
        let empty = self.input.read(cx).value().trim().is_empty() && self.images.is_empty();
        let view = cx.entity().downgrade();
        div()
            .key_context("PromptBar")
            .debug_selector(|| "prompt-bar".into())
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(|_, _: &Interrupt, _, cx| cx.emit(Event::Interrupt)))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if focused { p.accent } else { p.popup_border }))
            .bg(rgb(p.surface))
            .shadow_lg()
            .text_color(rgb(p.text))
            .when(!self.images.is_empty() || self.saving > 0, |el| {
                el.child(self.render_chips(cx))
            })
            .child(
                div()
                    .id("prompt-input-area")
                    .debug_selector(|| "prompt-input-area".into())
                    .flex()
                    .flex_col()
                    .px_1()
                    .pt(px(4.))
                    .when_some(min_height, |el, height| el.min_h(height))
                    .on_drag_move(cx.listener(Self::resize))
                    .child(
                        Input::new(&self.input)
                            .appearance(false)
                            .flex_grow()
                            .items_start(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pb_1p5()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(
                        Button::new("prompt-commands")
                            .ghost()
                            .xsmall()
                            .label("/")
                            .tooltip(format!("{} commands", agent.label()))
                            .disabled(self.sending)
                            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _, _| {
                                let mut menu = menu
                                    .min_w(px(300.))
                                    .max_h(px(360.))
                                    .scrollable(true)
                                    .label(format!("{} commands", agent.label()));
                                for command in prompt::commands(agent) {
                                    let view = view.clone();
                                    menu =
                                        menu.item(
                                            PopupMenuItem::new(format!(
                                                "/{}  —  {}",
                                                command.name, command.description
                                            ))
                                            .on_click(move |_, _, cx| {
                                                let _ = view.update(cx, |_, cx| {
                                                    cx.emit(Event::Command(command))
                                                });
                                            }),
                                        );
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("prompt-collapse")
                            .ghost()
                            .xsmall()
                            .label("Terminal")
                            .tooltip("Hide the prompt bar and type in the terminal (⌘J)")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Collapse))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(if self.error.is_some() { p.red } else { p.muted }))
                            .child(match (&self.error, self.sending) {
                                (Some(error), _) => error.clone(),
                                (None, true) => format!("Sending to {}…", agent.label()),
                                (None, false) => {
                                    "⏎ send   ⇧⏎ new line   ⌘V image   esc interrupt".to_owned()
                                }
                            }),
                    )
                    .child(
                        Button::new("prompt-send")
                            .primary()
                            .xsmall()
                            .label(if self.sending { "Sending" } else { "Send" })
                            .loading(self.sending)
                            .disabled(empty || self.saving > 0)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit(&Submit, window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("prompt-resize")
                    .absolute()
                    .top(px(-4.))
                    .left_0()
                    .right_0()
                    .h(px(8.))
                    .cursor_row_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.resize_offset = None),
                    )
                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.reset_height(cx);
                        }
                    }))
                    .on_drag(ResizePrompt, |_, _, _, cx| cx.new(|_| EmptyView)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::Root;
    use std::prelude::v1::test;

    #[gpui::test]
    fn double_click_selects_unicode_words_in_message_input(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            init(cx);
        });
        let mut bar = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view =
                cx.new(|cx| PromptBar::new(PathBuf::from("/tmp/give-word-test"), window, cx));
            bar = Some(view.clone());
            Root::new(view, window, cx)
        });
        let bar = bar.unwrap();
        for (text, expected) in [("hello world", "hello"), ("Привет сосед", "Привет")]
        {
            cx.update(|window, cx| {
                bar.update(cx, |bar, cx| {
                    bar.input
                        .update(cx, |input, cx| input.set_value(text, window, cx));
                });
            });
            cx.refresh().unwrap();
            let area = cx.debug_bounds("prompt-input-area").unwrap();
            let position = point(area.left() + px(28.), area.top() + px(16.));
            cx.simulate_event(MouseDownEvent {
                button: MouseButton::Left,
                position,
                click_count: 2,
                ..Default::default()
            });
            cx.simulate_event(MouseUpEvent {
                button: MouseButton::Left,
                position,
                ..Default::default()
            });
            cx.simulate_keystrokes("cmd-c");
            cx.update(|_, cx| {
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref(),
                    Some(expected),
                    "double-click in {text:?}"
                );
            });
        }
    }
}
