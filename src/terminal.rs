//! A real PTY and Alacritty grid, independent of the desktop renderer.
//! Parsing, writes and resizing run off the UI thread. Hiding a view keeps this
//! owner alive; dropping it terminates its shell without waiting on the UI thread.
use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term, TermMode, cell::Flags, color::Colors},
    vte::ansi::{Color, CursorShape, Processor, Rgb},
};
use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

pub const BACKGROUND: u32 = 0x172330;
pub const FOREGROUND: u32 = 0xcbd5e1;
pub use alacritty_terminal::selection::SelectionType as SelectionKind;
pub use alacritty_terminal::term::TermMode as Mode;
pub use alacritty_terminal::term::cell::Flags as CellFlags;
pub use alacritty_terminal::vte::ansi::CursorShape as CursorKind;
pub use portable_pty::PtySize as Size;

#[derive(Clone, Copy)]
struct GridSize {
    rows: usize,
    cols: usize,
}
impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}
impl From<PtySize> for GridSize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows.max(1) as usize,
            cols: size.cols.max(2) as usize,
        }
    }
}

enum Io {
    Write(Vec<u8>),
    Resize(PtySize),
    Stop,
}

#[derive(Clone)]
struct Events {
    io: mpsc::Sender<Io>,
    ui: mpsc::SyncSender<Event>,
    size: Arc<AtomicU64>,
}
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::PtyWrite(text) => Some(text),
            Event::ColorRequest(index, format) => {
                let rgb = palette(index);
                Some(format(Rgb {
                    r: (rgb >> 16) as u8,
                    g: (rgb >> 8) as u8,
                    b: rgb as u8,
                }))
            }
            Event::TextAreaSizeRequest(format) => {
                let n = self.size.load(Ordering::Relaxed);
                Some(format(WindowSize {
                    num_lines: n as u16,
                    num_cols: (n >> 16) as u16,
                    cell_width: (n >> 32) as u16,
                    cell_height: (n >> 48) as u16,
                }))
            }
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => None,
            other => {
                let _ = self.ui.try_send(other);
                None
            }
        };
        if let Some(text) = reply {
            let _ = self.io.send(Io::Write(text.into_bytes()));
        }
    }
}

struct State {
    term: Term<Events>,
    exited: Option<String>,
}

#[derive(Clone)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
    pub text: String,
    pub foreground: u32,
    pub background: u32,
    pub flags: Flags,
    pub selected: bool,
    pub hyperlink: Option<String>,
}
#[derive(Clone)]
pub struct Frame {
    pub rows: usize,
    pub columns: usize,
    pub cells: Vec<Cell>,
    pub text: String,
    pub selected_text: Option<String>,
    pub cursor: Option<(usize, usize, CursorShape)>,
    pub mode: TermMode,
    pub display_offset: usize,
    pub history: usize,
    pub exited: Option<String>,
}

pub struct Terminal {
    state: Arc<Mutex<State>>,
    revision: Arc<AtomicU64>,
    io: mpsc::Sender<Io>,
    events: mpsc::Receiver<Event>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    size: Arc<AtomicU64>,
}

fn pack_size(size: PtySize) -> u64 {
    size.rows as u64
        | (size.cols as u64) << 16
        | ((size.pixel_width / size.cols.max(1)) as u64) << 32
        | ((size.pixel_height / size.rows.max(1)) as u64) << 48
}

impl Terminal {
    pub fn spawn(path: &Path, size: PtySize) -> Result<Self> {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| {
            if cfg!(windows) {
                "powershell.exe".into()
            } else {
                "/bin/sh".into()
            }
        });
        let mut command = CommandBuilder::new(shell);
        if !cfg!(windows) {
            command.arg("-l");
        }
        Self::spawn_command(path, size, command)
    }

    fn spawn_command(path: &Path, mut size: PtySize, mut command: CommandBuilder) -> Result<Self> {
        size.rows = size.rows.max(1);
        size.cols = size.cols.max(2);
        command.cwd(path);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "give");
        command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        command.env_remove("TMUX");
        command.env_remove("TMUX_PANE");
        let pair = native_pty_system()
            .openpty(size)
            .context("Could not create terminal PTY")?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .context("Could not start the system shell")?;
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        drop(pair.slave);
        let (io, incoming) = mpsc::channel();
        let (ui, events) = mpsc::sync_channel(256);
        let current_size = Arc::new(AtomicU64::new(pack_size(size)));
        let state = Arc::new(Mutex::new(State {
            term: Term::new(
                Config::default(),
                &GridSize::from(size),
                Events {
                    io: io.clone(),
                    ui,
                    size: current_size.clone(),
                },
            ),
            exited: None,
        }));
        let revision = Arc::new(AtomicU64::new(1));
        let model = state.clone();
        let changed = revision.clone();
        thread::Builder::new()
            .name("give-pty-read".into())
            .spawn(move || {
                let mut processor: Processor = Processor::new();
                let mut bytes = [0; 8192];
                loop {
                    match reader.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(count) => {
                            let mut state = model.lock().unwrap_or_else(|e| e.into_inner());
                            processor.advance(&mut state.term, &bytes[..count]);
                            drop(state);
                            changed.fetch_add(1, Ordering::Release);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })?;
        let model = state.clone();
        let changed = revision.clone();
        thread::Builder::new()
            .name("give-pty-write".into())
            .spawn(move || {
                let master = pair.master;
                while let Ok(command) = incoming.recv() {
                    match command {
                        Io::Write(bytes) => {
                            if writer
                                .write_all(&bytes)
                                .and_then(|_| writer.flush())
                                .is_err()
                            {
                                break;
                            }
                        }
                        Io::Resize(size) => {
                            let mut state = model.lock().unwrap_or_else(|e| e.into_inner());
                            state.term.resize(GridSize::from(size));
                            drop(state);
                            if master.resize(size).is_err() {
                                break;
                            }
                            changed.fetch_add(1, Ordering::Release);
                        }
                        Io::Stop => break,
                    }
                }
            })?;
        let model = state.clone();
        let changed = revision.clone();
        thread::Builder::new()
            .name("give-pty-wait".into())
            .spawn(move || {
                let status = child
                    .wait()
                    .map(|status| status.to_string())
                    .unwrap_or_else(|e| e.to_string());
                model.lock().unwrap_or_else(|e| e.into_inner()).exited = Some(status);
                changed.fetch_add(1, Ordering::Release);
            })?;
        Ok(Self {
            state,
            revision,
            io,
            events,
            killer,
            size: current_size,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
    pub fn write(&self, bytes: Vec<u8>) {
        let _ = self.io.send(Io::Write(bytes));
    }
    pub fn resize(&self, size: PtySize) {
        self.size.store(pack_size(size), Ordering::Relaxed);
        let _ = self.io.send(Io::Resize(size));
    }

    /// Never wait for the parser on the UI thread; retain the last painted frame.
    pub fn frame(&self) -> Option<Frame> {
        let state = self.state.try_lock().ok()?;
        Some(frame(&state))
    }

    pub fn scroll(&self, lines: i32) {
        if let Ok(mut state) = self.state.try_lock() {
            state.term.scroll_display(Scroll::Delta(lines));
            self.revision.fetch_add(1, Ordering::Release);
        }
    }
    pub fn bottom(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            state.term.scroll_display(Scroll::Bottom);
            self.revision.fetch_add(1, Ordering::Release);
        }
    }
    pub fn select(&self, row: usize, column: usize, right_half: bool, kind: Option<SelectionType>) {
        if let Ok(mut state) = self.state.try_lock() {
            let row = row.min(state.term.screen_lines() - 1);
            let col = column.min(state.term.columns() - 1);
            let point = Point::new(
                Line(row as i32 - state.term.grid().display_offset() as i32),
                Column(col),
            );
            let side = if right_half { Side::Right } else { Side::Left };
            if let Some(kind) = kind {
                state.term.selection = Some(Selection::new(kind, point, side));
            } else if let Some(selection) = &mut state.term.selection {
                selection.update(point, side);
            }
            self.revision.fetch_add(1, Ordering::Release);
        }
    }
    pub fn select_all(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            let start = Point::new(state.term.topmost_line(), Column(0));
            let end = Point::new(state.term.bottommost_line(), state.term.last_column());
            let mut selection = Selection::new(SelectionType::Lines, start, Side::Left);
            selection.update(end, Side::Right);
            state.term.selection = Some(selection);
            self.revision.fetch_add(1, Ordering::Release);
        }
    }
    pub fn clear_selection(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            state.term.selection = None;
            self.revision.fetch_add(1, Ordering::Release);
        }
    }
    pub fn selected_text(&self) -> Option<String> {
        self.state.try_lock().ok()?.term.selection_to_string()
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.killer.kill();
        let _ = self.io.send(Io::Stop);
    }
}

fn frame(state: &State) -> Frame {
    let term = &state.term;
    let content = term.renderable_content();
    let display_offset = content.display_offset;
    let cursor_row = content.cursor.point.line.0 + display_offset as i32;
    let cursor = (cursor_row >= 0
        && cursor_row < term.screen_lines() as i32
        && content.cursor.shape != CursorShape::Hidden)
        .then_some((
            cursor_row.max(0) as usize,
            content.cursor.point.column.0,
            content.cursor.shape,
        ));
    let cells = content
        .display_iter
        .map(|indexed| {
            let cell = indexed.cell;
            let mut foreground = resolve(cell.fg, content.colors, cell.flags, true);
            let mut background = resolve(cell.bg, content.colors, cell.flags, false);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            if cell.flags.contains(Flags::DIM) {
                foreground = (((foreground & 0xff0000) * 2 / 3) & 0xff0000)
                    | (((foreground & 0xff00) * 2 / 3) & 0xff00)
                    | (((foreground & 0xff) * 2 / 3) & 0xff);
            }
            let mut text = cell.c.to_string();
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra);
            }
            if cell.flags.intersects(
                Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER | Flags::HIDDEN,
            ) {
                text.clear();
            }
            Cell {
                row: (indexed.point.line.0 + display_offset as i32).max(0) as usize,
                column: indexed.point.column.0,
                text,
                foreground,
                background,
                flags: cell.flags,
                selected: content
                    .selection
                    .is_some_and(|selection| selection.contains(indexed.point)),
                hyperlink: cell.hyperlink().map(|link| link.uri().to_owned()),
            }
        })
        .collect();
    let selected_text = term.selection_to_string();
    let start = content
        .selection
        .map_or(Line(-(display_offset as i32)), |s| {
            s.start.line.min(Line(-(display_offset as i32)))
        });
    let end = content.selection.map_or(
        Line(term.screen_lines() as i32 - display_offset as i32 - 1),
        |s| {
            s.end
                .line
                .max(Line(term.screen_lines() as i32 - display_offset as i32 - 1))
        },
    );
    let mut text = term.bounds_to_string(
        Point::new(start, Column(0)),
        Point::new(end, term.last_column()),
    );
    // A complete line selection includes its final newline in Alacritty.
    text.push('\n');
    Frame {
        text,
        selected_text,
        rows: term.screen_lines(),
        columns: term.columns(),
        cells,
        cursor,
        mode: content.mode,
        display_offset,
        history: term.history_size(),
        exited: state.exited.clone(),
    }
}

fn resolve(color: Color, overrides: &Colors, flags: Flags, foreground: bool) -> u32 {
    let mut index = match color {
        Color::Spec(rgb) => return (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32,
        Color::Indexed(index) => index as usize,
        Color::Named(named) => named as usize,
    };
    if foreground && flags.contains(Flags::BOLD) && index < 8 {
        index += 8;
    }
    if let Some(rgb) = overrides[index] {
        return (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32;
    }
    palette(index)
}

pub fn palette(index: usize) -> u32 {
    const ANSI: [u32; 16] = [
        0x253242, 0xdd8a98, 0x9ac9ad, 0xd8bd86, 0x87b1d7, 0xb89eda, 0x88c3cc, 0xcbd5e1, 0x71859d,
        0xf0acb6, 0xb6dfc6, 0xe8d3a8, 0xa7cbed, 0xcfb9ed, 0xa4dce2, 0xf0f4fa,
    ];
    match index {
        0..16 => ANSI[index],
        16..232 => {
            let n = index - 16;
            let level = |v| if v == 0 { 0 } else { 55 + 40 * v };
            (level(n / 36) << 16 | level(n / 6 % 6) << 8 | level(n % 6)) as u32
        }
        232..256 => {
            let n = (8 + 10 * (index - 232)) as u32;
            n << 16 | n << 8 | n
        }
        256 | 267 => FOREGROUND,
        258 => 0x87b1d7,
        259..267 => ANSI[index - 259],
        _ => BACKGROUND,
    }
}

#[derive(Default, Clone, Copy)]
pub struct Modifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Non-text keys only. Committed Unicode text comes through the platform IME.
pub fn key_sequence(key: &str, modifiers: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let m = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = match key {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    let text = if let Some(last) = cursor {
        if m != 1 {
            format!("\x1b[1;{m}{last}")
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{last}")
        } else {
            format!("\x1b[{last}")
        }
    } else {
        match key {
            "enter" => "\r".into(),
            "escape" => "\x1b".into(),
            "backspace" if modifiers.alt => "\x1b\x7f".into(),
            "backspace" => "\x7f".into(),
            "tab" if modifiers.shift => "\x1b[Z".into(),
            "tab" => "\t".into(),
            "insert" | "delete" | "pageup" | "pagedown" => {
                let n = match key {
                    "insert" => 2,
                    "delete" => 3,
                    "pageup" => 5,
                    _ => 6,
                };
                if m == 1 {
                    format!("\x1b[{n}~")
                } else {
                    format!("\x1b[{n};{m}~")
                }
            }
            "f1" | "f2" | "f3" | "f4" if m == 1 => {
                format!("\x1bO{}", (b'P' + key[1..].parse::<u8>().ok()? - 1) as char)
            }
            k if k.starts_with('f')
                && k[1..]
                    .parse::<usize>()
                    .ok()
                    .is_some_and(|n| (1..=12).contains(&n)) =>
            {
                let n = [11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 23, 24]
                    [k[1..].parse::<usize>().ok()? - 1];
                if m == 1 {
                    format!("\x1b[{n}~")
                } else {
                    format!("\x1b[{n};{m}~")
                }
            }
            k if modifiers.control && k.len() == 1 => {
                let c = k.as_bytes()[0].to_ascii_uppercase();
                let control = match c {
                    b'@'..=b'_' => c & 0x1f,
                    b' ' | b'2' => 0,
                    b'?' | b'8' => 127,
                    _ => return None,
                };
                if modifiers.alt {
                    return Some(vec![27, control]);
                } else {
                    return Some(vec![control]);
                }
            }
            "space" if modifiers.control => return Some(vec![0]),
            k if modifiers.alt && k.chars().count() == 1 => format!("\x1b{k}"),
            _ => return None,
        }
    };
    Some(text.into_bytes())
}

pub fn paste_sequence(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
    } else {
        text.replace("\r\n", "\n").replace('\n', "\r").into_bytes()
    }
}

pub fn mouse_sequence(
    button: u8,
    row: usize,
    column: usize,
    release: bool,
    modifiers: Modifiers,
    mode: TermMode,
) -> Vec<u8> {
    let button = button
        + 4 * u8::from(modifiers.shift)
        + 8 * u8::from(modifiers.alt)
        + 16 * u8::from(modifiers.control);
    if mode.contains(TermMode::SGR_MOUSE) {
        format!(
            "\x1b[<{button};{};{}{}",
            column + 1,
            row + 1,
            if release { 'm' } else { 'M' }
        )
        .into_bytes()
    } else {
        let mut bytes = vec![27, b'[', b'M', if release { 35 } else { 32 + button }];
        for value in [column + 33, row + 33] {
            if mode.contains(TermMode::UTF8_MOUSE) {
                let mut b = [0; 4];
                bytes.extend_from_slice(
                    char::from_u32(value.min(0x7ff) as u32)
                        .unwrap()
                        .encode_utf8(&mut b)
                        .as_bytes(),
                );
            } else {
                bytes.push(value.min(255) as u8);
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn model(rows: u16, cols: u16) -> (State, mpsc::Receiver<Io>, mpsc::Receiver<Event>) {
        let (io, incoming) = mpsc::channel();
        let (ui, events) = mpsc::sync_channel(256);
        let size = Size {
            rows,
            cols,
            pixel_width: cols * 8,
            pixel_height: rows * 20,
        };
        let term = Term::new(
            Config::default(),
            &GridSize::from(size),
            Events {
                io,
                ui,
                size: Arc::new(AtomicU64::new(pack_size(size))),
            },
        );
        (State { term, exited: None }, incoming, events)
    }
    fn parse(state: &mut State, bytes: &[u8]) {
        let mut parser: Processor = Processor::new();
        parser.advance(&mut state.term, bytes);
    }
    #[test]
    fn ansi_colors_unicode_hyperlinks_and_alternate_screen_round_trip() {
        let (mut state, _, events) = model(5, 30);
        parse(&mut state, "\x1b[38;2;12;34;56mЯ界e\u{301}\x1b[0m\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\x1b]2;A terminal title\x07".as_bytes());
        let before = frame(&state);
        assert_eq!(before.cells[0].foreground, 0x0c2238);
        assert_eq!(before.cells[0].text, "Я");
        assert!(before.cells[1].flags.contains(Flags::WIDE_CHAR));
        assert!(before.cells[2].text.is_empty());
        assert_eq!(before.cells[3].text, "e\u{301}");
        assert_eq!(
            before.cells[4].hyperlink.as_deref(),
            Some("https://example.com")
        );
        assert!(
            events
                .try_iter()
                .any(|e| matches!(e, Event::Title(t) if t == "A terminal title"))
        );
        parse(
            &mut state,
            b"\x1b[?1049h\x1b[2J\x1b[Halternate\x1b[?2004h\x1b[?1006h",
        );
        let alternate = frame(&state);
        assert!(
            alternate
                .mode
                .contains(Mode::ALT_SCREEN | Mode::BRACKETED_PASTE | Mode::SGR_MOUSE)
        );
        assert!(alternate.text.starts_with("alternate"));
        parse(&mut state, b"\x1b[?1049l");
        assert_eq!(frame(&state).cells[0].text, "Я");
    }
    #[test]
    fn terminal_queries_receive_replies_instead_of_hanging_tui_programs() {
        let (mut state, io, _) = model(6, 40);
        parse(&mut state, b"\x1b[3;5H\x1b[6n\x1b]11;?\x07\x1b[18t");
        let replies: Vec<_> = io
            .try_iter()
            .filter_map(|v| {
                if let Io::Write(bytes) = v {
                    Some(String::from_utf8(bytes).unwrap())
                } else {
                    None
                }
            })
            .collect();
        assert!(replies.iter().any(|s| s == "\x1b[3;5R"), "{replies:?}");
        assert!(
            replies.iter().any(|s| s.starts_with("\x1b]11;rgb:")),
            "{replies:?}"
        );
        assert!(replies.iter().any(|s| s == "\x1b[8;6;40t"), "{replies:?}");
    }
    #[test]
    fn selection_copies_unicode_and_stays_available_in_scrollback() {
        let (mut state, _, _) = model(3, 30);
        parse(
            &mut state,
            "first\r\nРусский текст\r\nlast\r\nmore".as_bytes(),
        );
        let mut selection = Selection::new(
            SelectionType::Lines,
            Point::new(Line(-1), Column(0)),
            Side::Left,
        );
        selection.update(Point::new(Line(0), Column(29)), Side::Right);
        state.term.selection = Some(selection);
        let frame = frame(&state);
        assert_eq!(
            frame.selected_text.as_deref(),
            Some("first\nРусский текст\n")
        );
        assert!(frame.text.contains("first\nРусский текст"));
        assert!(frame.history > 0);
    }
    #[test]
    fn keyboard_paste_and_mouse_modes_match_xterm_protocols() {
        assert_eq!(
            key_sequence("up", Modifiers::default(), Mode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_sequence(
                "c",
                Modifiers {
                    control: true,
                    ..Default::default()
                },
                Mode::empty()
            ),
            Some(vec![3])
        );
        assert_eq!(key_sequence("я", Modifiers::default(), Mode::empty()), None);
        assert_eq!(
            paste_sequence("echo a\x1b[201~\nb", Mode::BRACKETED_PASTE),
            b"\x1b[200~echo a[201~\nb\x1b[201~"
        );
        assert_eq!(
            mouse_sequence(0, 2, 4, true, Modifiers::default(), Mode::SGR_MOUSE),
            b"\x1b[<0;5;3m"
        );
    }
    fn wait_for(terminal: &Terminal, expected: &str) -> Frame {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(frame) = terminal.frame()
                && frame.text.contains(expected)
            {
                return frame;
            }
            assert!(Instant::now() < until, "PTY did not output {expected:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn real_project_sessions_start_in_their_roots_and_accept_input_and_resize() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let start = |path: &Path| {
            let mut command = CommandBuilder::new("/bin/sh");
            command.args(["-c", "stty -echo; printf 'ROOT:%s\\r\\nREADY\\r\\n' \"$PWD\"; IFS= read -r line; printf 'INPUT:%s\\r\\n' \"$line\"; stty size; printf 'DONE\\r\\n'"]);
            Terminal::spawn_command(
                path,
                Size {
                    rows: 8,
                    cols: 120,
                    pixel_width: 960,
                    pixel_height: 160,
                },
                command,
            )
            .unwrap()
        };
        let a = start(first.path());
        let b = start(second.path());
        assert!(
            wait_for(&a, "READY")
                .text
                .contains(first.path().canonicalize().unwrap().to_str().unwrap())
        );
        assert!(
            wait_for(&b, "READY")
                .text
                .contains(second.path().canonicalize().unwrap().to_str().unwrap())
        );
        b.resize(Size {
            rows: 10,
            cols: 100,
            pixel_width: 800,
            pixel_height: 200,
        });
        b.write("project two\r".as_bytes().to_vec());
        let result = wait_for(&b, "DONE");
        assert!(result.text.contains("INPUT:project two"));
        assert!(result.text.contains("10 100"), "{:?}", result.text);
        a.write("первый проект\r".as_bytes().to_vec());
        assert!(wait_for(&a, "DONE").text.contains("INPUT:первый проект"));
    }
}
