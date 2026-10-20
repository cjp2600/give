//! A compact find panel used independently by each preview and terminal.
use give::search::Query;
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};

actions!(find, [Next, Previous, Close, Focus]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Next, Some("FindPanel > Input")),
        KeyBinding::new("shift-enter", Previous, Some("FindPanel > Input")),
        KeyBinding::new("escape", Close, Some("FindPanel")),
        KeyBinding::new("secondary-f", Focus, Some("FindPanel")),
        KeyBinding::new("secondary-g", Next, Some("FindPanel")),
        KeyBinding::new("secondary-shift-g", Previous, Some("FindPanel")),
    ]);
}

pub enum Event {
    Changed,
    Next,
    Previous,
    Closed,
}

pub struct FindPanel {
    input: Entity<InputState>,
    pub query: Query,
    pub visible: bool,
    current: Option<usize>,
    count: usize,
    searching: bool,
    _subscription: Subscription,
}

impl EventEmitter<Event> for FindPanel {}

impl FindPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find…"));
        let subscription = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query.text = input.read(cx).value().to_string();
                cx.emit(Event::Changed);
                cx.notify();
            }
        });
        Self {
            input,
            query: Query::default(),
            visible: false,
            current: None,
            count: 0,
            searching: false,
            _subscription: subscription,
        }
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        self.focus(window, cx);
        cx.notify();
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.emit(Event::Closed);
        cx.notify();
    }

    pub fn set_results(
        &mut self,
        current: Option<usize>,
        count: usize,
        searching: bool,
        cx: &mut Context<Self>,
    ) {
        self.current = current;
        self.count = count;
        self.searching = searching;
        cx.notify();
    }
}

impl Render for FindPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("FindPanel")
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .flex_shrink_0()
            .bg(rgb(0x213143))
            .border_b_1()
            .border_color(rgb(0x2a394b))
            .text_color(rgb(0xcbd5e1))
            .text_size(px(11.))
            .on_action(cx.listener(|this, _: &Focus, window, cx| this.focus(window, cx)))
            .on_action(cx.listener(|_, _: &Next, _, cx| cx.emit(Event::Next)))
            .on_action(cx.listener(|_, _: &Previous, _, cx| cx.emit(Event::Previous)))
            .on_action(cx.listener(|this, _: &Close, _, cx| this.close(cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.input).small()),
            )
            .child(div().min_w(px(62.)).text_center().child(if self.searching {
                "Searching…".to_owned()
            } else if self.query.text.is_empty() {
                String::new()
            } else if self.count == 0 {
                "No results".to_owned()
            } else {
                format!(
                    "{}/{}",
                    self.current.map_or(0, |index| index + 1),
                    self.count
                )
            }))
            .child(
                Button::new("match-case")
                    .ghost()
                    .xsmall()
                    .label("Aa")
                    .selected(self.query.case_sensitive)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.query.case_sensitive = !this.query.case_sensitive;
                        cx.emit(Event::Changed);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("previous-match")
                    .ghost()
                    .xsmall()
                    .label("↑")
                    .disabled(self.count == 0 || self.searching)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Previous))),
            )
            .child(
                Button::new("next-match")
                    .ghost()
                    .xsmall()
                    .label("↓")
                    .disabled(self.count == 0 || self.searching)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Next))),
            )
            .child(
                Button::new("close-find")
                    .ghost()
                    .xsmall()
                    .label("×")
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
    }
}
