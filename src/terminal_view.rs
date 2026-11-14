//! GPUI terminal renderer. The PTY and ANSI state live in give::terminal.
use alacritty_terminal::event::Event;
use give::{
    prompt,
    rcli::Agent,
    search,
    terminal::{
        self, CellFlags, CursorKind, Frame, Mode, SearchMatch, SelectionKind, Size, Terminal,
    },
};
use give::{settings, theme};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable, WindowExt,
    button::{Button, ButtonVariants},
    menu::{PopupMenu, PopupMenuItem},
    notification::Notification,
};
use std::{
    cell::Cell,
    collections::VecDeque,
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

actions!(
    terminal,
    [
        TerminalCopy,
        TerminalPaste,
        TerminalSelectAll,
        TerminalContextMenu,
        CloseQuickTerminal,
        TogglePrompt
    ]
);

struct QuickTerminalClose;

struct TerminalMenu {
    view: Entity<PopupMenu>,
    position: Point<Pixels>,
    block: Option<Range<usize>>,
    revision: u64,
    display_offset: usize,
    _subscription: Subscription,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-f", crate::Find, Some("Terminal && !Input")),
        KeyBinding::new("secondary-c", TerminalCopy, Some("Terminal")),
        KeyBinding::new("secondary-v", TerminalPaste, Some("Terminal")),
        KeyBinding::new("secondary-a", TerminalSelectAll, Some("Terminal")),
        KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
        KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
        KeyBinding::new("shift-f10", TerminalContextMenu, Some("Terminal")),
        KeyBinding::new("secondary-w", CloseQuickTerminal, Some("QuickTerminal")),
        KeyBinding::new("secondary-j", TogglePrompt, Some("Terminal")),
    ]);
    crate::prompt_bar::init(cx);
}

/// Clipboard images in the form `give::clipboard_file` saves.
pub fn clipboard_images(images: Vec<Image>) -> Vec<give::clipboard_file::Image> {
    images
        .into_iter()
        .map(|image| give::clipboard_file::Image {
            extension: match image.format() {
                ImageFormat::Png => "png",
                ImageFormat::Jpeg => "jpg",
                ImageFormat::Webp => "webp",
                ImageFormat::Gif => "gif",
                ImageFormat::Svg => "svg",
                ImageFormat::Bmp => "bmp",
                ImageFormat::Tiff => "tiff",
            },
            bytes: image.bytes().to_vec(),
        })
        .collect()
}

/// Pauses between the parts of a prompt bar message, so the agent attaches pasted
/// images and takes the pasted text before Enter submits it.
const IMAGE_SETTLE: Duration = Duration::from_millis(250);
const PASTE_SETTLE: Duration = Duration::from_millis(120);
/// How soon a foreground group without a detected agent is looked up again.
const AGENT_RETRY: Duration = Duration::from_secs(2);

/// How long the send button waits for the agent to react to Enter.
const SEND_ECHO: Duration = Duration::from_millis(1500);

enum Input {
    Bytes(Vec<u8>),
    Images(Vec<Image>),
}

pub struct TerminalView {
    is_quick: bool,
    quick_terminal: Option<Entity<TerminalView>>,
    quick_open: bool,
    accessibility: crate::accessibility::Text,
    terminal: Option<Terminal>,
    frame: Option<Arc<Frame>>,
    revision: u64,
    theme_revision: u64,
    focus: FocusHandle,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    cell_width: f32,
    /// Horizontal offset of the grid inside a panel narrower than it, in pixels.
    scroll_x: f32,
    /// When `scroll_x` last moved; the horizontal bar shows only shortly after.
    scrolled_at: Option<Instant>,
    /// The cursor last brought into view; scrolling follows only its moves.
    followed_cursor: Option<(usize, usize)>,
    size: Size,
    path: PathBuf,
    title: String,
    error: Option<String>,
    active: bool,
    selecting: bool,
    mouse_button: Option<u8>,
    hovered: Option<(Point<Pixels>, bool)>,
    context_menu: Option<TerminalMenu>,
    read_task: Option<Task<()>>,
    wheel: f32,
    preedit: String,
    preedit_selection: Range<usize>,
    queued: Vec<u8>,
    input: VecDeque<Input>,
    paste_pending: bool,
    generation: u64,
    blink: bool,
    blink_at: Instant,
    find: Entity<crate::find_view::FindPanel>,
    find_matches: Arc<Vec<SearchMatch>>,
    find_current: Option<usize>,
    find_request: u64,
    find_revision: Option<u64>,
    find_pending: bool,
    find_started: Instant,
    /// The agent owning the terminal, detected from its foreground process group.
    agent: Option<Agent>,
    agent_group: Option<u32>,
    /// When the foreground group was last looked up, while a lookup runs or found
    /// no agent; it is looked up again after `AGENT_RETRY`.
    agent_checked: Option<Instant>,
    prompt: Entity<crate::prompt_bar::PromptBar>,
    /// Expanded over the agent's own prompt, or folded away to a small pill.
    prompt_open: bool,
    /// Focus the prompt bar on the next render; the agent just appeared.
    prompt_focus_pending: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<QuickTerminalClose> for TerminalView {}

impl TerminalView {
    pub fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let find = cx.new(|cx| crate::find_view::FindPanel::new(window, cx));
        let prompt = cx.new(|cx| crate::prompt_bar::PromptBar::new(path.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&prompt, window, |this, _, event, window, cx| {
                use crate::prompt_bar::Event;
                match event {
                    Event::Submit(message) => {
                        this.deliver(&message.text, &message.images, window, cx)
                    }
                    Event::Command(command) => {
                        this.deliver(&format!("/{}", command.name), &[], window, cx)
                    }
                    Event::Interrupt => this.send(b"\x1b".to_vec(), cx),
                    Event::Collapse => this.set_prompt_open(false, window, cx),
                }
            }),
            cx.subscribe_in(&find, window, |this, _, event, window, cx| {
                match event {
                    crate::find_view::Event::Changed => this.search_terminal(true, cx),
                    crate::find_view::Event::Next => this.next_match(false, cx),
                    crate::find_view::Event::Previous => this.next_match(true, cx),
                    crate::find_view::Event::Closed => {
                        this.find_request += 1;
                        this.find_pending = false;
                        this.find_matches = Arc::default();
                        this.focus.focus(window);
                    }
                }
                cx.notify();
            }),
            cx.on_focus(&focus, window, |this, _, cx| {
                this.focus_event(true);
                this.blink = true;
                cx.notify();
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                this.focus_event(false);
                cx.notify();
            }),
        ];
        let mut this = Self {
            is_quick: false,
            quick_terminal: None,
            quick_open: false,
            accessibility: crate::accessibility::Text::default(),
            terminal: None,
            frame: None,
            revision: 0,
            theme_revision: theme::revision(),
            focus,
            bounds: Rc::new(Cell::new(Bounds::default())),
            cell_width: 8.4,
            scroll_x: 0.,
            scrolled_at: None,
            followed_cursor: None,
            size: Size {
                rows: 32,
                cols: settings::current().terminal_min_columns,
                pixel_width: 1008,
                pixel_height: 672,
            },
            path,
            title: "Terminal".into(),
            error: None,
            active: true,
            selecting: false,
            mouse_button: None,
            hovered: None,
            context_menu: None,
            read_task: None,
            wheel: 0.,
            preedit: String::new(),
            preedit_selection: 0..0,
            queued: Vec::new(),
            input: VecDeque::new(),
            paste_pending: false,
            generation: 0,
            blink: true,
            blink_at: Instant::now(),
            find,
            find_matches: Arc::default(),
            find_current: None,
            find_request: 0,
            find_revision: None,
            find_pending: false,
            find_started: Instant::now(),
            agent: None,
            agent_group: None,
            agent_checked: None,
            prompt,
            prompt_open: true,
            prompt_focus_pending: false,
            _subscriptions: subscriptions,
        };
        this.launch(cx);
        cx.spawn(async move |view, cx| {
            loop {
                let interval = view.update(cx, |this, cx| {
                    this.poll(cx);
                    if this.active {
                        Duration::from_millis(16)
                    } else {
                        Duration::from_millis(200)
                    }
                });
                let Ok(interval) = interval else {
                    break;
                };
                cx.background_executor().timer(interval).await;
            }
        })
        .detach();
        this
    }

    fn launch(&mut self, cx: &mut Context<Self>) {
        self.find_request += 1;
        self.find_pending = false;
        self.find_revision = None;
        self.find_matches = Arc::default();
        self.generation += 1;
        let generation = self.generation;
        self.input.clear();
        self.queued.clear();
        self.paste_pending = false;
        self.terminal = None;
        self.frame = None;
        self.hovered = None;
        self.context_menu = None;
        self.read_task = None;
        self.error = None;
        self.revision = 0;
        let path = self.path.clone();
        let size = self.size;
        let task = cx
            .background_executor()
            .spawn(async move { Terminal::spawn(&path, size) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(terminal) => {
                        terminal.resize(this.size);
                        if !this.queued.is_empty() {
                            terminal.write(std::mem::take(&mut this.queued));
                        }
                        this.terminal = Some(terminal);
                        this.poll(cx);
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_visible(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !active && self.quick_open {
            self.finish_quick_terminal(window, cx);
            window.close_dialog(cx);
        }
        self.active = active;
        self.accessibility.invalidate();
        self.accessibility.publish(
            if self.is_quick { 2 } else { 1 },
            if self.is_quick {
                "Quick terminal"
            } else {
                "Terminal"
            },
            active && self.focus.is_focused(window),
            active,
            self.bounds.get(),
            window,
        );
        if active {
            self.poll(cx);
            self.focus_input(window, cx);
        }
    }

    fn prompt_visible(&self) -> bool {
        !self.is_quick && self.agent.is_some()
    }

    /// Focuses the prompt bar while it covers the agent's prompt, else the terminal.
    fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt_visible() && self.prompt_open {
            self.prompt
                .update(cx, |prompt, cx| prompt.focus(window, cx));
        } else {
            self.focus.focus(window);
        }
    }

    fn set_prompt_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_open = open;
        self.focus_input(window, cx);
        cx.notify();
    }

    fn toggle_prompt(&mut self, _: &TogglePrompt, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt_visible() {
            self.set_prompt_open(!self.prompt_open, window, cx);
            cx.stop_propagation();
        }
    }

    /// Looks up the agent when another process group takes over the terminal, and
    /// again while that group shows none: a lookup between the shell's fork and the
    /// agent's exec, or a failed `ps`, must not hide the bar for the agent's lifetime.
    fn watch_agent(&mut self, cx: &mut Context<Self>) {
        let group = self.foreground();
        let retry = group.is_some()
            && self.agent.is_none()
            && self
                .agent_checked
                .is_some_and(|checked| checked.elapsed() >= AGENT_RETRY);
        if group == self.agent_group && !retry {
            return;
        }
        self.agent_group = group;
        let Some(group) = group else {
            self.agent = None;
            self.agent_checked = None;
            cx.notify();
            return;
        };
        // Also blocks further retries until this lookup answers.
        self.agent_checked = None;
        let task = cx
            .background_executor()
            .spawn(async move { prompt::detect_agent(group) });
        cx.spawn(async move |view, cx| {
            let agent = task.await;
            let _ = view.update(cx, |this, cx| {
                if this.agent_group != Some(group) {
                    return;
                }
                let appeared = this.agent.is_none() && agent.is_some();
                this.agent = agent;
                this.agent_checked = agent.is_none().then(Instant::now);
                if let Some(agent) = agent {
                    this.prompt
                        .update(cx, |prompt, cx| prompt.set_agent(agent, cx));
                }
                if appeared {
                    this.prompt_open = true;
                    this.prompt_focus_pending = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Types a prompt bar message into the agent: images as pasted paths, the text as
    /// one paste, then Enter. A known command is typed as keys so the agent's command
    /// line opens; one with a picker hands the keyboard to the terminal.
    fn deliver(
        &mut self,
        text: &str,
        images: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = self.mode();
        let command = self.agent.and_then(|agent| prompt::command_of(agent, text));
        let mut parts = Vec::new();
        if !images.is_empty() {
            let paths = give::clipboard_file::shell_paths(images);
            parts.push((terminal::paste_sequence(&paths, mode), IMAGE_SETTLE));
        }
        if !text.is_empty() {
            let bytes = if command.is_some() && images.is_empty() {
                text.trim().as_bytes().to_vec()
            } else {
                terminal::paste_sequence(text, mode)
            };
            parts.push((bytes, PASTE_SETTLE));
        }
        parts.push((b"\r".to_vec(), Duration::ZERO));
        if command.is_some_and(|command| command.interactive) {
            self.set_prompt_open(false, window, cx);
        }
        let generation = self.generation;
        cx.spawn(async move |view, cx| {
            let mut sent_at = 0;
            for (bytes, settle) in parts {
                let Ok(revision) = view.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.send(bytes, cx);
                    }
                    this.terminal.as_ref().map_or(0, Terminal::revision)
                }) else {
                    return;
                };
                sent_at = revision;
                cx.background_executor().timer(settle).await;
            }
            let started = Instant::now();
            while started.elapsed() < SEND_ECHO {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let Ok(revision) = view.update(cx, |this, _| {
                    this.terminal.as_ref().map_or(0, Terminal::revision)
                }) else {
                    return;
                };
                if revision != sent_at {
                    break;
                }
            }
            let _ = view.update(cx, |this, cx| {
                let error = (this.generation != generation || this.terminal.is_none())
                    .then(|| "The terminal restarted before the message was sent".to_owned());
                this.prompt
                    .update(cx, |prompt, cx| prompt.finish_sending(error, cx));
            });
        })
        .detach();
    }

    /// Whether a program other than the shell owns this terminal, such as an agent.
    pub fn busy(&self) -> bool {
        self.terminal.as_ref().is_some_and(Terminal::busy)
    }

    /// The program running in place of the shell, by its process group.
    pub fn foreground(&self) -> Option<u32> {
        self.terminal.as_ref().and_then(Terminal::foreground)
    }

    /// Types `line` and Enter into the shell. Refuses while another program owns the
    /// terminal, so a command never lands in an agent's prompt.
    pub fn run_line(&mut self, line: &str, cx: &mut Context<Self>) -> bool {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        match &self.terminal {
            Some(terminal) if terminal.busy() => return false,
            Some(terminal) => terminal.write(bytes),
            None if self.error.is_none() => self.queued.extend(bytes),
            None => return false,
        }
        cx.notify();
        true
    }

    /// Opens the quick terminal and runs `line` there, beside the main session.
    pub fn run_in_quick_terminal(
        &mut self,
        line: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_quick {
            return false;
        }
        if !self.quick_open {
            self.open_quick_terminal(window, cx);
        }
        let Some(quick) = self.quick_terminal.clone() else {
            return false;
        };
        quick.update(cx, |view, cx| view.run_line(line, cx))
    }

    fn open_quick_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_quick || window.has_active_dialog(cx) {
            return;
        }
        let quick = if let Some(quick) = &self.quick_terminal {
            quick.clone()
        } else {
            let quick = cx.new(|cx| {
                let mut view = Self::new(self.path.clone(), window, cx);
                view.is_quick = true;
                view
            });
            self._subscriptions.push(cx.subscribe_in(
                &quick,
                window,
                |this, _, _: &QuickTerminalClose, window, cx| {
                    this.finish_quick_terminal(window, cx);
                    window.close_dialog(cx);
                },
            ));
            self.quick_terminal = Some(quick.clone());
            quick
        };
        // Hide the main terminal from selection readers while the modal owns input.
        // Its PTY and running processes continue independently.
        self.set_visible(false, window, cx);
        self.quick_open = true;
        let owner = cx.entity().downgrade();
        let directory = self.path.to_string_lossy().into_owned();
        let content = quick.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let viewport = window.viewport_size();
            let width = (f32::from(viewport.width) - 80.).min(1040.);
            let height = (f32::from(viewport.height) - 120.).min(600.);
            let owner = owner.clone();
            dialog
                .title(
                    div()
                        .px_3()
                        .py_2()
                        .pr_10()
                        .flex()
                        .gap_3()
                        .text_size(px(12.))
                        .child("Quick terminal")
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(theme::current().colors.muted))
                                .child(directory.clone()),
                        ),
                )
                .width(px(width))
                .margin_top(px((f32::from(viewport.height) - height) / 2. - 30.))
                .p_0()
                .gap_0()
                .overflow_hidden()
                // Enter, Escape and other shell/TUI keys belong to the terminal.
                .keyboard(false)
                .child(
                    div()
                        .debug_selector(|| "quick-terminal-content".into())
                        .w_full()
                        .h(px(height - 40.))
                        .child(content.clone()),
                )
                .on_close(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| this.finish_quick_terminal(window, cx));
                })
        });
        quick.update(cx, |view, cx| view.set_visible(true, window, cx));
        cx.notify();
    }

    fn finish_quick_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_open = false;
        if let Some(quick) = &self.quick_terminal {
            quick.update(cx, |view, cx| view.set_visible(false, window, cx));
        }
        self.set_visible(true, window, cx);
        cx.notify();
    }

    fn mode(&self) -> Mode {
        self.frame
            .as_ref()
            .map_or_else(Mode::default, |frame| frame.mode)
    }
    fn focus_event(&self, focused: bool) {
        if self.mode().contains(Mode::FOCUS_IN_OUT)
            && let Some(terminal) = &self.terminal
        {
            terminal.write(if focused {
                b"\x1b[I".to_vec()
            } else {
                b"\x1b[O".to_vec()
            });
        }
    }
    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.input.push_back(Input::Bytes(bytes));
        self.flush_input(cx);
    }
    fn flush_input(&mut self, cx: &mut Context<Self>) {
        if self.paste_pending {
            return;
        }
        while let Some(input) = self.input.pop_front() {
            match input {
                Input::Bytes(bytes) => self.write_now(bytes, cx),
                Input::Images(images) => {
                    self.paste_pending = true;
                    let generation = self.generation;
                    let task = cx
                        .background_executor()
                        .spawn(async move { give::clipboard_file::save(clipboard_images(images)) });
                    cx.spawn(async move |view, cx| {
                        let result = task.await;
                        let _ = view.update(cx, |this, cx| {
                            if this.generation != generation {
                                return;
                            }
                            this.paste_pending = false;
                            match result {
                                Ok(paths) => {
                                    let text = give::clipboard_file::shell_paths(&paths);
                                    this.write_now(
                                        terminal::paste_sequence(&text, this.mode()),
                                        cx,
                                    );
                                    this.error = None;
                                }
                                Err(error) => this.error = Some(format!("Clipboard: {error:#}")),
                            }
                            this.flush_input(cx);
                            cx.notify();
                        });
                    })
                    .detach();
                    return;
                }
            }
        }
    }
    fn write_now(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.bottom();
            terminal.clear_selection();
            terminal.write(bytes);
        } else {
            self.queued.extend(bytes);
        }
        self.blink = true;
        self.blink_at = Instant::now();
        self.poll(cx);
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        if self.terminal.is_none() {
            return;
        }
        if !self.is_quick {
            self.watch_agent(cx);
        }
        let Some(terminal) = &self.terminal else {
            return;
        };
        let mut changed = false;
        for event in terminal.events() {
            match event {
                Event::Title(title) => {
                    self.title = title;
                    changed = true;
                }
                Event::ResetTitle => {
                    self.title = "Terminal".into();
                    changed = true;
                }
                Event::ClipboardStore(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text))
                }
                Event::ClipboardLoad(_, formatter) => {
                    let text = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    terminal.write(formatter(&text).into_bytes());
                }
                Event::Bell => {
                    self.blink = true;
                    changed = true;
                }
                _ => {}
            }
        }
        let revision = terminal.revision();
        if self.active
            && (revision != self.revision || self.theme_revision != theme::revision())
            && let Some(frame) = terminal.frame()
        {
            self.accessibility.set_value(frame.text.clone());
            let selected = frame.selected_text.clone().unwrap_or_default();
            let start = frame
                .text
                .find(&selected)
                .map_or(0, |byte| frame.text[..byte].encode_utf16().count());
            let end = start + selected.encode_utf16().count();
            self.accessibility.set_selection(selected, start..end);
            self.frame = Some(Arc::new(frame));
            self.revision = revision;
            self.theme_revision = theme::revision();
            changed = true;
        }
        if self.active
            && settings::current().cursor_blink
            && self.blink_at.elapsed() >= Duration::from_millis(600)
        {
            self.blink = !self.blink;
            self.blink_at = Instant::now();
            changed = true;
        }
        if changed && self.active {
            cx.notify();
        }
        if self.active
            && self.find.read(cx).visible
            && !self.find_pending
            && !self.find.read(cx).query.text.is_empty()
            && self
                .frame
                .as_ref()
                .is_some_and(|frame| Some(frame.content_revision) != self.find_revision)
            && self.find_started.elapsed() >= Duration::from_millis(200)
        {
            self.search_terminal(false, cx);
        }
    }

    fn open_find(&mut self, _: &crate::Find, window: &mut Window, cx: &mut Context<Self>) {
        self.find.update(cx, |find, cx| find.open(window, cx));
        self.search_terminal(true, cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn search_terminal(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.find_request += 1;
        let request = self.find_request;
        let query = self.find.read(cx).query.clone();
        if reveal {
            self.find_matches = Arc::default();
            self.find_current = None;
        }
        if query.text.is_empty() {
            self.find_pending = false;
            self.find
                .update(cx, |find, cx| find.set_results(None, 0, false, cx));
            return;
        }
        let Some(source) = self.terminal.as_ref().map(Terminal::search_source) else {
            return;
        };
        self.find_pending = true;
        self.find_started = Instant::now();
        self.find.update(cx, |find, cx| {
            find.set_results(self.find_current, self.find_matches.len(), true, cx)
        });
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
            let result = cx
                .background_executor()
                .spawn(async move { source.search(&query) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.find_request != request {
                    return;
                }
                this.find_pending = false;
                this.find_revision = Some(result.revision);
                let previous = this
                    .find_current
                    .and_then(|index| this.find_matches.get(index))
                    .cloned();
                let first_visible = -(this.frame.as_ref().map_or(0, |f| f.display_offset) as i32);
                this.find_current = if result.matches.is_empty() {
                    None
                } else {
                    Some(
                        previous
                            .and_then(|m| result.matches.iter().position(|next| next == &m))
                            .unwrap_or_else(|| {
                                result
                                    .matches
                                    .iter()
                                    .position(|m| m.start().line.0 >= first_visible)
                                    .unwrap_or(0)
                            }),
                    )
                };
                this.find_matches = Arc::new(result.matches);
                this.reveal_match(reveal, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn next_match(&mut self, backwards: bool, cx: &mut Context<Self>) {
        self.find_current =
            search::next_index(self.find_current, self.find_matches.len(), backwards);
        self.reveal_match(true, cx);
        cx.notify();
    }

    fn reveal_match(&mut self, reveal: bool, cx: &mut Context<Self>) {
        if reveal
            && let Some(m) = self
                .find_current
                .and_then(|index| self.find_matches.get(index))
        {
            if let Some(terminal) = &self.terminal {
                terminal.reveal_match(m);
            }
            let column = m.start().column.0 as f32 * self.cell_width;
            let width = f32::from(self.bounds.get().size.width);
            self.scroll_x = (column - width / 2.).max(0.);
        }
        self.find.update(cx, |find, cx| {
            find.set_results(self.find_current, self.find_matches.len(), false, cx)
        });
        self.poll(cx);
    }

    fn copy(&mut self, _: &TerminalCopy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.as_ref().and_then(Terminal::selected_text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    fn paste(&mut self, _: &TerminalPaste, _: &mut Window, cx: &mut Context<Self>) {
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
        if !images.is_empty() {
            self.input.push_back(Input::Images(images));
            self.flush_input(cx);
        } else if let Some(text) = text {
            self.send(terminal::paste_sequence(&text, self.mode()), cx);
        }
    }

    fn select_all(&mut self, _: &TerminalSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.select_all();
            self.poll(cx);
        }
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let stroke = &event.keystroke;
        if stroke.modifiers.platform {
            return;
        }
        if stroke.modifiers.shift && matches!(stroke.key.as_str(), "pageup" | "pagedown") {
            if let Some(terminal) = &self.terminal {
                terminal.scroll(if stroke.key == "pageup" {
                    self.size.rows as i32
                } else {
                    -(self.size.rows as i32)
                });
            }
            self.poll(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if let Some(bytes) =
            terminal::key_sequence(&stroke.key, modifiers(stroke.modifiers), self.mode())
        {
            self.send(bytes, cx);
            window.prevent_default();
            cx.stop_propagation();
        }
    }
    /// Bring a moved cursor into view; manual scrolling stays put while it rests.
    fn follow_cursor(&mut self, view_width: f32) {
        let cursor = self
            .frame
            .as_ref()
            .and_then(|frame| frame.cursor)
            .map(|(row, column, _)| (row, column));
        if cursor == self.followed_cursor {
            return;
        }
        self.followed_cursor = cursor;
        let Some((_, column)) = cursor else {
            return;
        };
        let left = column as f32 * self.cell_width;
        let right = left + self.cell_width;
        if left < self.scroll_x {
            self.scroll_x = left;
        } else if right > self.scroll_x + view_width {
            self.scroll_x = right - view_width;
        }
    }
    fn grid_point(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        let bounds = self.bounds.get();
        let x = ((f32::from(position.x - bounds.left()) + self.scroll_x) / self.cell_width).max(0.);
        let y = (f32::from(position.y - bounds.top()) / settings::terminal_line_height()).max(0.);
        (
            y.floor().min(self.size.rows.saturating_sub(1) as f32) as usize,
            x.floor().min(self.size.cols.saturating_sub(1) as f32) as usize,
            x.fract() >= 0.5,
        )
    }

    fn highlighted_block(&self) -> Option<Range<usize>> {
        let frame = self.frame.as_ref()?;
        if let Some(menu) = &self.context_menu {
            return (menu.revision == frame.content_revision
                && menu.display_offset == frame.display_offset)
                .then(|| menu.block.clone())
                .flatten();
        }
        let (position, shift) = self.hovered?;
        if self.selecting
            || !self.bounds.get().contains(&position)
            || (self.mode().intersects(Mode::MOUSE_MODE) && !shift)
        {
            return None;
        }
        frame.block_at(self.grid_point(position).0)
    }

    fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.grid_point(position).0;
        let frame = self.frame.as_ref();
        let text = frame.and_then(|frame| self.terminal.as_ref()?.block_text(row, frame));
        let block = text.as_ref().and_then(|_| frame?.block_at(row));
        let revision = frame.map_or(0, |frame| frame.content_revision);
        let display_offset = frame.map_or(0, |frame| frame.display_offset);
        let selection = self.terminal.as_ref().and_then(Terminal::selected_text);
        let focus = self.focus.clone();
        let read_text = text.clone();
        let read_view = cx.entity().downgrade();
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            menu.action_context(focus)
                .item(
                    PopupMenuItem::new("Copy block")
                        .disabled(text.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(text) = &text {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            }
                        }),
                )
                .when(cfg!(target_os = "macos"), |menu| {
                    menu.item(
                        PopupMenuItem::new("Read block")
                            .disabled(read_text.is_none())
                            .on_click(move |_, window, cx| {
                                if let Some(text) = &read_text {
                                    let _ = read_view.update(cx, |this, cx| {
                                        this.read_block(text.clone(), window, cx);
                                    });
                                }
                            }),
                    )
                })
                .item(
                    PopupMenuItem::new("Copy selection")
                        .disabled(selection.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(text) = &selection {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            }
                        }),
                )
                .separator()
                .menu("Paste", Box::new(TerminalPaste))
                .menu("Select all", Box::new(TerminalSelectAll))
        });
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu = None;
            this.hovered = None;
            cx.notify();
        });
        menu.focus_handle(cx).focus(window);
        self.context_menu = Some(TerminalMenu {
            view: menu,
            position,
            block,
            revision,
            display_offset,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn read_block(&mut self, text: String, window: &Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        // Let the menu dismiss before asking VoiceReader to read this clipboard.
        // A newer request cancels this one.
        self.read_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            #[cfg(not(test))]
            let result = cx
                .background_executor()
                .spawn(async { crate::voice_reader::read_clipboard() })
                .await;
            #[cfg(test)]
            let result = crate::voice_reader::read_clipboard();
            let _ = view.update_in(cx, |_, window, cx| {
                if let Err(message) = result {
                    window.push_notification(Notification::error(message).autohide(false), cx);
                }
            });
        }));
    }

    fn keyboard_context_menu(
        &mut self,
        _: &TerminalContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(frame) = &self.frame else {
            return;
        };
        let row = frame.cursor.map_or(0, |(row, _, _)| row);
        let bounds = self.bounds.get();
        self.open_context_menu(
            point(
                bounds.left(),
                bounds.top() + px(row as f32 * settings::terminal_line_height()),
            ),
            window,
            cx,
        );
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        cx.stop_propagation();
        let (row, column, half) = self.grid_point(event.position);
        let mode = self.mode();
        let button = match event.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return,
        };
        if event.modifiers.platform
            && let Some(uri) = self
                .frame
                .as_ref()
                .and_then(|frame| {
                    frame
                        .cells
                        .iter()
                        .find(|cell| cell.row == row && cell.column == column)
                })
                .and_then(|cell| cell.hyperlink.as_ref())
        {
            cx.open_url(uri);
            return;
        }
        if mode.intersects(Mode::MOUSE_MODE) && !event.modifiers.shift {
            self.mouse_button = Some(button);
            if let Some(terminal) = &self.terminal {
                terminal.write(terminal::mouse_sequence(
                    button,
                    row,
                    column,
                    false,
                    modifiers(event.modifiers),
                    mode,
                ));
            }
        } else if button == 2 && self.bounds.get().contains(&event.position) {
            self.open_context_menu(event.position, window, cx);
        } else if button == 0 {
            self.selecting = true;
            let kind = if event.modifiers.alt {
                SelectionKind::Block
            } else {
                match event.click_count {
                    2 => SelectionKind::Semantic,
                    3.. => SelectionKind::Lines,
                    _ => SelectionKind::Simple,
                }
            };
            if let Some(terminal) = &self.terminal {
                terminal.select(
                    row,
                    column,
                    half,
                    // In a mouse-aware program Shift belongs to the terminal:
                    // it starts a selection rather than extending a missing one.
                    // Double/triple-click still starts word/line selection.
                    if event.modifiers.shift
                        && !mode.intersects(Mode::MOUSE_MODE)
                        && event.click_count == 1
                    {
                        None
                    } else {
                        Some(kind)
                    },
                );
            }
            self.poll(cx);
        }
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let previous = self.highlighted_block();
        self.hovered = Some((event.position, event.modifiers.shift));
        if previous != self.highlighted_block() {
            cx.notify();
        }
        let (row, column, half) = self.grid_point(event.position);
        let mode = self.mode();
        if self.selecting && event.dragging() {
            if let Some(terminal) = &self.terminal {
                terminal.select(row, column, half, None);
            }
            self.poll(cx);
        } else if (mode.contains(Mode::MOUSE_MOTION)
            || (self.mouse_button.is_some() && mode.contains(Mode::MOUSE_DRAG)))
            && let Some(terminal) = &self.terminal
        {
            terminal.write(terminal::mouse_sequence(
                self.mouse_button.unwrap_or(3) + 32,
                row,
                column,
                false,
                modifiers(event.modifiers),
                mode,
            ));
        }
    }
    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        if let Some(button) = self.mouse_button.take() {
            let (row, column, _) = self.grid_point(event.position);
            if let Some(terminal) = &self.terminal {
                terminal.write(terminal::mouse_sequence(
                    button,
                    row,
                    column,
                    true,
                    modifiers(event.modifiers),
                    self.mode(),
                ));
            }
        }
        self.poll(cx);
    }
    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event
            .delta
            .pixel_delta(px(settings::terminal_line_height()));
        if delta.x.abs() > delta.y.abs() {
            // Clamped against the panel width when the grid is next painted.
            self.scroll_x = (self.scroll_x - f32::from(delta.x)).max(0.);
            self.scrolled_at = Some(Instant::now());
            cx.notify();
            cx.stop_propagation();
            return;
        }
        self.wheel += f32::from(
            event
                .delta
                .pixel_delta(px(settings::terminal_line_height()))
                .y,
        ) / settings::terminal_line_height();
        let lines = self.wheel.trunc() as i32;
        self.wheel -= lines as f32;
        if lines == 0 {
            return;
        }
        let mode = self.mode();
        let (row, column, _) = self.grid_point(event.position);
        if let Some(terminal) = &self.terminal {
            if mode.intersects(Mode::MOUSE_MODE) && !event.modifiers.shift {
                for _ in 0..lines.unsigned_abs().min(100) {
                    terminal.write(terminal::mouse_sequence(
                        if lines > 0 { 64 } else { 65 },
                        row,
                        column,
                        false,
                        modifiers(event.modifiers),
                        mode,
                    ));
                }
            } else if mode.contains(Mode::ALT_SCREEN | Mode::ALTERNATE_SCROLL)
                && !event.modifiers.shift
            {
                for _ in 0..lines.unsigned_abs().min(100) {
                    if let Some(bytes) = terminal::key_sequence(
                        if lines > 0 { "up" } else { "down" },
                        Default::default(),
                        mode,
                    ) {
                        terminal.write(bytes);
                    }
                }
            } else {
                terminal.scroll(lines);
            }
        }
        self.poll(cx);
        cx.stop_propagation();
    }
}

impl TerminalView {
    /// The prompt bar over the bottom rows, where the agent draws its own prompt, or
    /// the pill that brings it back.
    fn render_prompt(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = &theme::current().colors;
        let background = rgb(p.terminal_background);
        if !self.prompt_open {
            let label = format!("Message {}  ⌘J", self.agent.map_or("agent", Agent::label));
            return div()
                .absolute()
                .bottom_2()
                .right_3()
                .occlude()
                .child(
                    Button::new("prompt-open")
                        .small()
                        .label(label)
                        .tooltip("Show the prompt bar")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_prompt_open(true, window, cx)
                        })),
                )
                .into_any_element();
        }
        let mut faded: Hsla = background.into();
        faded.a = 0.;
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .cursor_default()
            .flex()
            .flex_col()
            .child(div().h(px(28.)).bg(linear_gradient(
                0.,
                linear_color_stop(background, 0.),
                linear_color_stop(faded, 1.),
            )))
            .child(
                div()
                    .px_2()
                    .pb(px(24.))
                    .bg(background)
                    .child(self.prompt.clone()),
            )
            .into_any_element()
    }
}

fn modifiers(m: gpui::Modifiers) -> terminal::Modifiers {
    terminal::Modifiers {
        control: m.control,
        alt: m.alt,
        shift: m.shift,
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.accessibility.publish(
            if self.is_quick { 2 } else { 1 },
            if self.is_quick {
                "Quick terminal"
            } else {
                "Terminal"
            },
            self.active && self.focus.is_focused(window),
            self.active,
            self.bounds.get(),
            window,
        );
        if std::mem::take(&mut self.prompt_focus_pending) {
            cx.defer_in(window, |this, window, cx| {
                // Only follow a terminal the user is typing in.
                if this.active && this.focus.is_focused(window) {
                    this.focus_input(window, cx);
                }
            });
        }
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let bounds_cell = self.bounds.clone();
        let prompt = self.prompt_visible().then(|| self.render_prompt(cx));
        div()
            .key_context(if self.is_quick {
                "Terminal QuickTerminal"
            } else {
                "Terminal"
            })
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::current().colors.terminal_background))
            .text_color(rgb(theme::current().colors.terminal_foreground))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::keyboard_context_menu))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::toggle_prompt))
            .on_action(cx.listener(|this, _: &CloseQuickTerminal, _, cx| {
                if this.is_quick {
                    cx.emit(QuickTerminalClose);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .h(px(35.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(theme::current().colors.border))
                    .text_size(px(12.))
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.title.clone()),
                    )
                    .when(
                        self.error.is_some() && self.terminal.is_none()
                            || self
                                .frame
                                .as_ref()
                                .is_some_and(|frame| frame.exited.is_some()),
                        |el| {
                            el.child(
                                Button::new("new-shell")
                                    .ghost()
                                    .xsmall()
                                    .label("New shell")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.launch(cx);
                                        this.focus.focus(window);
                                    })),
                            )
                        },
                    )
                    .when(!self.is_quick, |el| {
                        el.child(
                            div()
                                .debug_selector(|| "quick-terminal-launcher".into())
                                .child(
                                    Button::new("quick-terminal")
                                        .ghost()
                                        .xsmall()
                                        .icon(
                                            gpui_component::Icon::empty()
                                                .path("icons/terminal.svg"),
                                        )
                                        .tooltip("Open quick terminal")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_quick_terminal(window, cx);
                                        })),
                                ),
                        )
                    }),
            )
            .when(self.find.read(cx).visible, |el| el.child(self.find.clone()))
            .child(
                div()
                    .id("terminal-grid")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .p_2()
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if !hovered {
                            this.hovered = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
                    .on_scroll_wheel(cx.listener(Self::wheel))
                    .child(
                        div().size_full().overflow_hidden().child(
                            canvas(
                                move |bounds, window, cx| {
                                    bounds_cell.set(bounds);
                                    entity.update(cx, |this, _| {
                                        let font = gpui::font(settings::font_family());
                                        let run = TextRun {
                                            len: 1,
                                            font,
                                            color: rgb(theme::current().colors.terminal_foreground)
                                                .into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        let line = window.text_system().shape_line(
                                            "M".into(),
                                            px(settings::terminal_font_size()),
                                            &[run],
                                            None,
                                        );
                                        this.cell_width = f32::from(line.width).max(1.);
                                        let view_width = f32::from(bounds.size.width);
                                        let cols = ((view_width / this.cell_width).floor() as u16)
                                            .max(settings::current().terminal_min_columns);
                                        let grid_width = cols as f32 * this.cell_width;
                                        let size = Size {
                                            rows: (f32::from(bounds.size.height)
                                                / settings::terminal_line_height())
                                            .floor()
                                            .max(1.)
                                                as u16,
                                            cols,
                                            pixel_width: grid_width as u16,
                                            pixel_height: f32::from(bounds.size.height).max(1.)
                                                as u16,
                                        };
                                        if this.size != size {
                                            this.size = size;
                                            if let Some(terminal) = &this.terminal {
                                                terminal.resize(size);
                                            }
                                        }
                                        let previous = this.scroll_x;
                                        this.follow_cursor(view_width);
                                        this.scroll_x = this
                                            .scroll_x
                                            .clamp(0., (grid_width - view_width).max(0.));
                                        if this.scroll_x != previous {
                                            this.scrolled_at = Some(Instant::now());
                                        }
                                        (
                                            this.frame.clone(),
                                            this.focus.is_focused(window),
                                            this.blink,
                                            this.preedit.clone(),
                                            this.cell_width,
                                            this.error.clone(),
                                            this.scroll_x,
                                            grid_width,
                                            this.scrolled_at,
                                        )
                                    })
                                },
                                move |bounds,
                                      (
                                    frame,
                                    focused,
                                    blink,
                                    preedit,
                                    cell_width,
                                    error,
                                    scroll_x,
                                    grid_width,
                                    scrolled_at,
                                ),
                                      window,
                                      cx| {
                                    let focus = paint_entity.read(cx).focus.clone();
                                    window.handle_input(
                                        &focus,
                                        ElementInputHandler::new(bounds, paint_entity.clone()),
                                        cx,
                                    );
                                    let Some(frame) = frame else {
                                        if let Some(error) = error {
                                            paint_message(&error, bounds.origin, window, cx);
                                        }
                                        return;
                                    };
                                    let grid = Bounds::new(
                                        point(bounds.left() - px(scroll_x), bounds.top()),
                                        size(px(grid_width), bounds.size.height),
                                    );
                                    let view = paint_entity.read(cx);
                                    let current = view.find_current;
                                    let found = view.find_matches.clone();
                                    let matches =
                                        if view.find_revision == Some(frame.content_revision) {
                                            found.as_slice()
                                        } else {
                                            &[]
                                        };
                                    paint_frame(
                                        (&frame, matches, current),
                                        (grid, bounds, view.highlighted_block()),
                                        cell_width,
                                        (focused, blink),
                                        &preedit,
                                        window,
                                        cx,
                                    );
                                    paint_scrollbar(bounds, grid, scrolled_at, window);
                                },
                            )
                            .size_full(),
                        ),
                    )
                    .children(prompt),
            )
            .child(
                div()
                    .h(px(23.))
                    .px_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(theme::current().colors.border))
                    .text_size(px(10.))
                    .text_color(rgb(theme::current().colors.muted))
                    .truncate()
                    .child(
                        self.error
                            .clone()
                            .or_else(|| self.frame.as_ref().and_then(|f| f.exited.clone()))
                            .unwrap_or_else(|| {
                                format!(
                                    "{} × {}   ·   ⌘C copy   ⌘V paste   ·   Right-click to copy block",
                                    self.size.cols, self.size.rows
                                )
                            }),
                    ),
            )
            .when_some(self.context_menu.as_ref(), |el, menu| {
                el.child(
                    deferred(
                        anchored()
                            .position(menu.position)
                            .snap_to_window_with_margin(px(8.))
                            .child(menu.view.clone()),
                    )
                    .with_priority(1),
                )
            })
    }
}

/// The bar stays this long after the last horizontal scroll, then fades out.
const SCROLLBAR_HOLD: Duration = Duration::from_millis(800);
const SCROLLBAR_FADE: Duration = Duration::from_millis(300);

/// A thin position bar under a grid wider than the panel, shown while it scrolls.
fn paint_scrollbar(
    view: Bounds<Pixels>,
    grid: Bounds<Pixels>,
    scrolled_at: Option<Instant>,
    window: &mut Window,
) {
    let (view_width, grid_width) = (f32::from(view.size.width), f32::from(grid.size.width));
    let Some(elapsed) = scrolled_at.map(|at| at.elapsed()) else {
        return;
    };
    if grid_width <= view_width + 0.5 || elapsed >= SCROLLBAR_HOLD + SCROLLBAR_FADE {
        return;
    }
    let opacity =
        1. - elapsed.saturating_sub(SCROLLBAR_HOLD).as_secs_f32() / SCROLLBAR_FADE.as_secs_f32();
    window.request_animation_frame();
    let offset = f32::from(view.left() - grid.left());
    let thumb = Bounds::new(
        point(
            view.left() + px(offset / grid_width * view_width),
            view.bottom() - px(3.),
        ),
        size(px(view_width * view_width / grid_width), px(3.)),
    );
    window.paint_quad(fill(
        thumb,
        Hsla::from(rgb(theme::current().colors.muted)).opacity(0.5 * opacity),
    ));
}

fn paint_message(text: &str, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let run = TextRun {
        len: text.len(),
        font: gpui::font(settings::font_family()),
        color: rgb(theme::current().colors.red).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        text.to_owned().into(),
        px(settings::terminal_font_size()),
        &[run],
        None,
    );
    let _ = line.paint(origin, px(settings::terminal_line_height()), window, cx);
}

/// Rectangles snapped to device pixels, so neighbouring cells join without seams.
fn paint_cell_rects(
    rects: &[give::terminal_glyphs::Rect],
    origin: Point<Pixels>,
    color: u32,
    window: &mut Window,
) {
    let scale = window.scale_factor().max(1.);
    let snap = |value: f32| (value * scale).round() / scale;
    let (left, top) = (f32::from(origin.x), f32::from(origin.y));
    for rect in rects {
        let x0 = snap(left + rect.x);
        let y0 = snap(top + rect.y);
        let x1 = snap(left + rect.x + rect.width).max(x0 + 1. / scale);
        let y1 = snap(top + rect.y + rect.height).max(y0 + 1. / scale);
        let mut fill_color = rgb(color);
        fill_color.a = rect.alpha;
        window.paint_quad(fill(
            Bounds::new(point(px(x0), px(y0)), size(px(x1 - x0), px(y1 - y0))),
            fill_color,
        ));
    }
}

fn paint_frame(
    (frame, matches, current): (&Frame, &[SearchMatch], Option<usize>),
    (bounds, viewport, block): (Bounds<Pixels>, Bounds<Pixels>, Option<Range<usize>>),
    width: f32,
    cursor_state: (bool, bool),
    preedit: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let palette = theme::current();
    let (focused, blink) = (
        cursor_state.0,
        cursor_state.1 || !settings::current().cursor_blink,
    );
    let cursor = frame
        .cursor
        .filter(|(row, column, _)| *row < frame.rows && *column < frame.columns);
    for cell in &frame.cells {
        if cell.row >= frame.rows || cell.column >= frame.columns {
            continue;
        }
        let position = point(
            bounds.left() + px(cell.column as f32 * width),
            bounds.top() + px(cell.row as f32 * settings::terminal_line_height()),
        );
        let is_cursor = cursor.is_some_and(|(row, col, _)| row == cell.row && col == cell.column);
        let point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(cell.row as i32 - frame.display_offset as i32),
            alacritty_terminal::index::Column(
                if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                    cell.column.saturating_sub(1)
                } else {
                    cell.column
                },
            ),
        );
        let index = matches.partition_point(|m| *m.end() < point);
        let matched = matches.get(index).is_some_and(|m| m.contains(&point));
        let mut background = if cell.selected {
            palette.colors.terminal_selection
        } else if matched {
            if Some(index) == current {
                palette.colors.search_active
            } else {
                palette.colors.search_match
            }
        } else {
            cell.background
        };
        if is_cursor
            && focused
            && blink
            && cursor.is_some_and(|(_, _, shape)| shape == CursorKind::Block)
        {
            background = palette.colors.cursor;
        }
        window.paint_quad(fill(
            Bounds::new(
                position,
                size(px(width), px(settings::terminal_line_height())),
            ),
            rgb(background),
        ));
    }
    if let Some(block) = block {
        let top = viewport.top() + px(block.start as f32 * settings::terminal_line_height());
        let height =
            px(block.len() as f32 * settings::terminal_line_height()).min(viewport.bottom() - top);
        let area = Bounds::new(
            point(viewport.left(), top),
            size(viewport.size.width, height),
        );
        window.paint_quad(fill(
            area,
            Hsla::from(rgb(palette.colors.terminal_foreground)).opacity(0.045),
        ));
        window.paint_quad(fill(
            Bounds::new(area.origin, size(px(2.), height)),
            Hsla::from(rgb(palette.colors.accent)).opacity(0.45),
        ));
    }
    for cell in &frame.cells {
        if cell.row >= frame.rows || cell.column >= frame.columns {
            continue;
        }
        let position = point(
            bounds.left() + px(cell.column as f32 * width),
            bounds.top() + px(cell.row as f32 * settings::terminal_line_height()),
        );
        let foreground = if focused
            && blink
            && cursor.is_some_and(|(row, column, shape)| {
                row == cell.row && column == cell.column && shape == CursorKind::Block
            }) {
            palette.colors.terminal_background
        } else {
            cell.foreground
        };
        if cell.text.is_empty() || cell.text == " " {
            continue;
        }
        let mut chars = cell.text.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(rects) =
                give::terminal_glyphs::rects(c, width, settings::terminal_line_height(), 1.)
        {
            paint_cell_rects(&rects, position, foreground, window);
            continue;
        }
        let mut font = gpui::font(settings::font_family());
        if cell.flags.contains(CellFlags::BOLD) {
            font.weight = FontWeight::BOLD;
        }
        if cell.flags.contains(CellFlags::ITALIC) {
            font.style = FontStyle::Italic;
        }
        let run = TextRun {
            len: cell.text.len(),
            font,
            color: rgb(foreground).into(),
            background_color: None,
            underline: cell
                .flags
                .intersects(CellFlags::ALL_UNDERLINES)
                .then_some(UnderlineStyle {
                    color: Some(rgb(foreground).into()),
                    thickness: px(1.),
                    wavy: cell.flags.contains(CellFlags::UNDERCURL),
                }),
            strikethrough: cell.flags.contains(CellFlags::STRIKEOUT).then_some(
                StrikethroughStyle {
                    color: Some(rgb(foreground).into()),
                    thickness: px(1.),
                },
            ),
        };
        let line = window.text_system().shape_line(
            cell.text.clone().into(),
            px(settings::terminal_font_size()),
            &[run],
            None,
        );
        let _ = line.paint(position, px(settings::terminal_line_height()), window, cx);
    }
    if let Some((row, column, shape)) = cursor {
        let pos = point(
            bounds.left() + px(column as f32 * width),
            bounds.top() + px(row as f32 * settings::terminal_line_height()),
        );
        if focused && blink {
            let caret = match shape {
                CursorKind::Beam => Some(Bounds::new(
                    pos,
                    size(px(2.), px(settings::terminal_line_height())),
                )),
                CursorKind::Underline => Some(Bounds::new(
                    point(pos.x, pos.y + px(settings::terminal_line_height() - 2.)),
                    size(px(width), px(2.)),
                )),
                _ => None,
            };
            if let Some(caret) = caret {
                window.paint_quad(fill(caret, rgb(palette.colors.cursor)));
            }
        } else if !focused {
            for rect in [
                Bounds::new(pos, size(px(width), px(1.))),
                Bounds::new(pos, size(px(1.), px(settings::terminal_line_height()))),
                Bounds::new(
                    point(pos.x, pos.y + px(settings::terminal_line_height() - 1.)),
                    size(px(width), px(1.)),
                ),
                Bounds::new(
                    point(pos.x + px(width - 1.), pos.y),
                    size(px(1.), px(settings::terminal_line_height())),
                ),
            ] {
                window.paint_quad(fill(rect, rgb(palette.colors.cursor_inactive)));
            }
        }
        if !preedit.is_empty() {
            paint_message(preedit, pos, window, cx);
        }
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf16_offset(&self.preedit, range.start);
        let end = utf16_offset(&self.preedit, range.end);
        *actual = Some(range);
        Some(self.preedit[start..end].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.preedit_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        self.preedit_selection = 0..0;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit.clear();
        self.preedit_selection = 0..0;
        self.send(text.as_bytes().to_vec(), cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = text.to_owned();
        let end = text.encode_utf16().count();
        self.preedit_selection = selected.unwrap_or(end..end);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, col, _) = self.frame.as_ref()?.cursor?;
        let bounds = self.bounds.get();
        Some(Bounds::new(
            point(
                bounds.left() + px(col as f32 * self.cell_width - self.scroll_x),
                bounds.top() + px(row as f32 * settings::terminal_line_height()),
            ),
            size(px(self.cell_width), px(settings::terminal_line_height())),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}
fn utf16_offset(text: &str, target: usize) -> usize {
    let mut offset = 0;
    for (index, ch) in text.char_indices() {
        if offset >= target {
            return index;
        }
        offset += ch.len_utf16();
    }
    text.len()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use gpui_component::Root;
    use std::prelude::v1::test;

    #[gpui::test]
    fn block_menu_copies_snapshot_and_restores_focus(cx: &mut TestAppContext) {
        exercise_block_menu(cx, false);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn read_block_copies_snapshot_before_invoking_reader(cx: &mut TestAppContext) {
        exercise_block_menu(cx, true);
    }

    fn exercise_block_menu(cx: &mut TestAppContext, read: bool) {
        let requests = crate::voice_reader::requests();
        cx.update(|cx| {
            gpui_component::init(cx);
            init(cx);
            crate::find_view::init(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let terminal_size = Size {
            rows: 8,
            cols: 80,
            pixel_width: 640,
            pixel_height: 160,
        };
        let mut command = portable_pty::CommandBuilder::new("/bin/sh");
        command.args([
            "-c",
            "printf 'first Привет\\r\\nsecond\\r\\n\\r\\nlast\\r\\n'; exec /bin/sleep 30",
        ]);
        let terminal = Terminal::spawn_command(directory.path(), terminal_size, command).unwrap();
        let started = Instant::now();
        let frame = loop {
            if let Some(frame) = terminal.frame()
                && frame.text.starts_with("first Привет\nsecond\n\nlast")
            {
                break frame;
            }
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut view = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let terminal_view = cx.new(|cx| {
                let mut this = TerminalView::new(directory.path().to_owned(), window, cx);
                this.generation += 1; // Ignore the shell launch scheduled by new().
                this.active = false;
                this.terminal = Some(terminal);
                this.frame = Some(Arc::new(frame));
                this.size = terminal_size;
                this.bounds.set(Bounds::new(
                    point(px(20.), px(40.)),
                    size(px(640.), px(160.)),
                ));
                this.hovered = Some((point(px(40.), px(45.)), false));
                assert_eq!(this.highlighted_block(), Some(0..2));
                this.selecting = true;
                assert_eq!(this.highlighted_block(), None);
                this.selecting = false;
                let mut mouse_frame = this.frame.as_ref().unwrap().as_ref().clone();
                mouse_frame.mode.insert(Mode::MOUSE_REPORT_CLICK);
                this.frame = Some(Arc::new(mouse_frame));
                let position = point(px(20. + this.cell_width * 8.), px(45.));
                this.mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers {
                            shift: true,
                            ..Default::default()
                        },
                        click_count: 2,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert_eq!(
                    this.terminal
                        .as_ref()
                        .and_then(Terminal::selected_text)
                        .as_deref(),
                    Some("Привет"),
                    "Shift + double-click must select words when bypassing tmux mouse input"
                );
                this.mouse_up(
                    &MouseUpEvent {
                        position,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                this.terminal.as_ref().unwrap().clear_selection();
                assert_eq!(this.highlighted_block(), None);
                this.mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Right,
                        position: point(px(40.), px(45.)),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert!(this.context_menu.is_none());
                this.hovered.as_mut().unwrap().1 = true;
                assert_eq!(this.highlighted_block(), Some(0..2));
                // Shift + right-click opens our menu even for a mouse-aware TUI.
                this.mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Right,
                        position: point(px(40.), px(45.)),
                        modifiers: Modifiers {
                            shift: true,
                            ..Default::default()
                        },
                        click_count: 1,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert!(this.context_menu.is_some());
                assert_eq!(this.highlighted_block(), Some(0..2));
                // The menu retains its original text when later output arrives.
                let mut changed = this.frame.as_ref().unwrap().as_ref().clone();
                changed.content_revision += 1;
                this.frame = Some(Arc::new(changed));
                assert_eq!(this.highlighted_block(), None);
                this
            });
            view = Some(terminal_view.clone());
            Root::new(terminal_view, window, cx)
        });
        let view = view.unwrap();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(if read {
            "down down enter"
        } else {
            "down enter"
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("first Привет\nsecond".into())
            );
            view.read_with(cx, |this, _| {
                assert!(this.context_menu.is_none());
                assert!(this.focus.is_focused(window));
            });
        });
        assert_eq!(crate::voice_reader::requests(), requests);
        if read {
            cx.background_executor
                .advance_clock(Duration::from_millis(200));
            cx.run_until_parked();
            assert_eq!(crate::voice_reader::requests(), requests + 1);
            // Replacing a pending request must not fire the reader twice.
            cx.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.read_block("older block".into(), window, cx);
                    this.read_block("newer block".into(), window, cx);
                });
            });
            cx.run_until_parked();
            cx.background_executor
                .advance_clock(Duration::from_millis(200));
            cx.run_until_parked();
            assert_eq!(crate::voice_reader::requests(), requests + 2);
            cx.update(|_, cx| {
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref(),
                    Some("newer block")
                );
            });
        }
        cx.simulate_keystrokes("shift-f10");
        view.read_with(cx, |this, _| assert!(this.context_menu.is_some()));
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| {
            view.read_with(cx, |this, _| {
                assert!(this.context_menu.is_none());
                assert!(this.focus.is_focused(window));
            });
        });
    }
}
