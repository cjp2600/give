//! GPUI terminal renderer. The PTY and ANSI state live in give::terminal.
use alacritty_terminal::event::Event;
use give::terminal::{self, CellFlags, CursorKind, Frame, Mode, SelectionKind, Size, Terminal};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
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

const FONT_SIZE: f32 = 14.;
/// About Menlo's own line height, so glyphs drawn by the font nearly touch.
const ROW_HEIGHT: f32 = 17.;
/// The grid is never narrower than this; a narrower panel scrolls horizontally,
/// a wider one gets as many columns as fit.
const MIN_COLUMNS: u16 = 100;
actions!(terminal, [TerminalCopy, TerminalPaste, TerminalSelectAll]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-c", TerminalCopy, Some("Terminal")),
        KeyBinding::new("secondary-v", TerminalPaste, Some("Terminal")),
        KeyBinding::new("secondary-a", TerminalSelectAll, Some("Terminal")),
        KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
        KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
    ]);
}

enum Input {
    Bytes(Vec<u8>),
    Images(Vec<Image>),
}

pub struct TerminalView {
    accessibility: crate::accessibility::Text,
    terminal: Option<Terminal>,
    frame: Option<Arc<Frame>>,
    revision: u64,
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
    wheel: f32,
    preedit: String,
    preedit_selection: Range<usize>,
    queued: Vec<u8>,
    input: VecDeque<Input>,
    paste_pending: bool,
    generation: u64,
    blink: bool,
    blink_at: Instant,
    _subscriptions: Vec<Subscription>,
}

impl TerminalView {
    pub fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
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
            accessibility: crate::accessibility::Text::default(),
            terminal: None,
            frame: None,
            revision: 0,
            focus,
            bounds: Rc::new(Cell::new(Bounds::default())),
            cell_width: 8.4,
            scroll_x: 0.,
            scrolled_at: None,
            followed_cursor: None,
            size: Size {
                rows: 32,
                cols: MIN_COLUMNS,
                pixel_width: 1008,
                pixel_height: 672,
            },
            path,
            title: "Terminal".into(),
            error: None,
            active: true,
            selecting: false,
            mouse_button: None,
            wheel: 0.,
            preedit: String::new(),
            preedit_selection: 0..0,
            queued: Vec::new(),
            input: VecDeque::new(),
            paste_pending: false,
            generation: 0,
            blink: true,
            blink_at: Instant::now(),
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
        self.generation += 1;
        let generation = self.generation;
        self.input.clear();
        self.queued.clear();
        self.paste_pending = false;
        self.terminal = None;
        self.frame = None;
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
        self.active = active;
        self.accessibility.invalidate();
        self.accessibility.publish(
            1,
            "Terminal",
            active && self.focus.is_focused(window),
            active,
            self.bounds.get(),
            window,
        );
        if active {
            self.poll(cx);
            self.focus.focus(window);
        }
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
                    let task = cx.background_executor().spawn(async move {
                        let images = images
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
                            .collect();
                        give::clipboard_file::save(images)
                    });
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
            && revision != self.revision
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
            changed = true;
        }
        if self.active && self.blink_at.elapsed() >= Duration::from_millis(600) {
            self.blink = !self.blink;
            self.blink_at = Instant::now();
            changed = true;
        }
        if changed && self.active {
            cx.notify();
        }
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
        let y = (f32::from(position.y - bounds.top()) / ROW_HEIGHT).max(0.);
        (
            y.floor().min(self.size.rows.saturating_sub(1) as f32) as usize,
            x.floor().min(self.size.cols.saturating_sub(1) as f32) as usize,
            x.fract() >= 0.5,
        )
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
                    if event.modifiers.shift {
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
        let delta = event.delta.pixel_delta(px(ROW_HEIGHT));
        if delta.x.abs() > delta.y.abs() {
            // Clamped against the panel width when the grid is next painted.
            self.scroll_x = (self.scroll_x - f32::from(delta.x)).max(0.);
            self.scrolled_at = Some(Instant::now());
            cx.notify();
            cx.stop_propagation();
            return;
        }
        self.wheel += f32::from(event.delta.pixel_delta(px(ROW_HEIGHT)).y) / ROW_HEIGHT;
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
            1,
            "Terminal",
            self.active && self.focus.is_focused(window),
            self.active,
            self.bounds.get(),
            window,
        );
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let bounds_cell = self.bounds.clone();
        div()
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(terminal::BACKGROUND))
            .text_color(rgb(terminal::FOREGROUND))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
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
                    .border_color(rgb(0x2a394b))
                    .text_size(px(12.))
                    .child(div().truncate().child(self.title.clone()))
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
                    ),
            )
            .child(
                div()
                    .id("terminal-grid")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .p_2()
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
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
                                        let font = gpui::font("Menlo");
                                        let run = TextRun {
                                            len: 1,
                                            font,
                                            color: rgb(terminal::FOREGROUND).into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        let line = window.text_system().shape_line(
                                            "M".into(),
                                            px(FONT_SIZE),
                                            &[run],
                                            None,
                                        );
                                        this.cell_width = f32::from(line.width).max(1.);
                                        let view_width = f32::from(bounds.size.width);
                                        let cols = ((view_width / this.cell_width).floor() as u16)
                                            .max(MIN_COLUMNS);
                                        let grid_width = cols as f32 * this.cell_width;
                                        let size = Size {
                                            rows: (f32::from(bounds.size.height) / ROW_HEIGHT)
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
                                    paint_frame(
                                        &frame,
                                        grid,
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
                    ),
            )
            .child(
                div()
                    .h(px(23.))
                    .px_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(0x2a394b))
                    .text_size(px(10.))
                    .text_color(rgb(0x7a8da5))
                    .child(
                        self.error
                            .clone()
                            .or_else(|| self.frame.as_ref().and_then(|f| f.exited.clone()))
                            .unwrap_or_else(|| {
                                format!(
                                    "{} × {}   ·   ⌘C copy   ⌘V paste",
                                    self.size.cols, self.size.rows
                                )
                            }),
                    ),
            )
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
        Hsla::from(rgb(0x7a8da5)).opacity(0.5 * opacity),
    ));
}

fn paint_message(text: &str, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let run = TextRun {
        len: text.len(),
        font: gpui::font("Menlo"),
        color: rgb(0xdb929d).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.to_owned().into(), px(FONT_SIZE), &[run], None);
    let _ = line.paint(origin, px(ROW_HEIGHT), window, cx);
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
    frame: &Frame,
    bounds: Bounds<Pixels>,
    width: f32,
    cursor_state: (bool, bool),
    preedit: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let (focused, blink) = cursor_state;
    let cursor = frame
        .cursor
        .filter(|(row, column, _)| *row < frame.rows && *column < frame.columns);
    for cell in &frame.cells {
        if cell.row >= frame.rows || cell.column >= frame.columns {
            continue;
        }
        let position = point(
            bounds.left() + px(cell.column as f32 * width),
            bounds.top() + px(cell.row as f32 * ROW_HEIGHT),
        );
        let is_cursor = cursor.is_some_and(|(row, col, _)| row == cell.row && col == cell.column);
        let mut background = if cell.selected {
            0x355575
        } else {
            cell.background
        };
        if is_cursor
            && focused
            && blink
            && cursor.is_some_and(|(_, _, shape)| shape == CursorKind::Block)
        {
            background = 0x87b1d7;
        }
        window.paint_quad(fill(
            Bounds::new(position, size(px(width), px(ROW_HEIGHT))),
            rgb(background),
        ));
    }
    for cell in &frame.cells {
        if cell.row >= frame.rows || cell.column >= frame.columns {
            continue;
        }
        let position = point(
            bounds.left() + px(cell.column as f32 * width),
            bounds.top() + px(cell.row as f32 * ROW_HEIGHT),
        );
        let foreground = if focused
            && blink
            && cursor.is_some_and(|(row, column, shape)| {
                row == cell.row && column == cell.column && shape == CursorKind::Block
            }) {
            terminal::BACKGROUND
        } else {
            cell.foreground
        };
        if cell.text.is_empty() || cell.text == " " {
            continue;
        }
        let mut chars = cell.text.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(rects) = give::terminal_glyphs::rects(c, width, ROW_HEIGHT, 1.)
        {
            paint_cell_rects(&rects, position, foreground, window);
            continue;
        }
        let mut font = gpui::font("Menlo");
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
        let line =
            window
                .text_system()
                .shape_line(cell.text.clone().into(), px(FONT_SIZE), &[run], None);
        let _ = line.paint(position, px(ROW_HEIGHT), window, cx);
    }
    if let Some((row, column, shape)) = cursor {
        let pos = point(
            bounds.left() + px(column as f32 * width),
            bounds.top() + px(row as f32 * ROW_HEIGHT),
        );
        if focused && blink {
            let caret = match shape {
                CursorKind::Beam => Some(Bounds::new(pos, size(px(2.), px(ROW_HEIGHT)))),
                CursorKind::Underline => Some(Bounds::new(
                    point(pos.x, pos.y + px(ROW_HEIGHT - 2.)),
                    size(px(width), px(2.)),
                )),
                _ => None,
            };
            if let Some(caret) = caret {
                window.paint_quad(fill(caret, rgb(0x87b1d7)));
            }
        } else if !focused {
            for rect in [
                Bounds::new(pos, size(px(width), px(1.))),
                Bounds::new(pos, size(px(1.), px(ROW_HEIGHT))),
                Bounds::new(
                    point(pos.x, pos.y + px(ROW_HEIGHT - 1.)),
                    size(px(width), px(1.)),
                ),
                Bounds::new(
                    point(pos.x + px(width - 1.), pos.y),
                    size(px(1.), px(ROW_HEIGHT)),
                ),
            ] {
                window.paint_quad(fill(rect, rgb(0x71859d)));
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
                bounds.top() + px(row as f32 * ROW_HEIGHT),
            ),
            size(px(self.cell_width), px(ROW_HEIGHT)),
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
