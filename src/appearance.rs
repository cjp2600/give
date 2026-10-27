//! Apply global preferences to every view, and watch the shared file across processes.
use give::{
    settings::{self, Settings},
    theme,
};
use gpui::*;
use gpui_component::{Theme, ThemeMode};
use std::{sync::Arc, time::Duration};

pub fn apply(settings: Settings, cx: &mut App) {
    let selected = theme::find(&settings.theme).expect("validated theme");
    settings::set(settings);
    preview(selected, cx);
}

pub fn preview(selected: Arc<theme::Theme>, cx: &mut App) {
    let mode = if selected.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let p = &selected.colors;
    Theme::change(mode, None, cx);
    let t = Theme::global_mut(cx);
    t.mono_font_family = settings::font_family().into();
    t.mono_font_size = px(settings::current().preview_font_size);
    let c = &mut t.colors;
    c.background = rgb(p.background).into();
    c.tiles = rgb(p.background).into();
    c.table = rgb(p.background).into();
    c.list = rgb(p.background).into();
    c.tab_active = rgb(p.background).into();
    c.success_foreground = rgb(p.background).into();
    c.danger_foreground = rgb(p.background).into();
    c.info_foreground = rgb(p.background).into();
    c.warning_foreground = rgb(p.background).into();
    c.popover = rgb(p.panel).into();
    c.accordion = rgb(p.panel).into();
    c.group_box = rgb(p.panel).into();
    c.tab = rgb(p.panel).into();
    c.sidebar = rgb(p.panel).into();
    c.list_head = rgb(p.panel).into();
    c.table_head = rgb(p.panel).into();
    c.tab_bar = rgb(p.panel).into();
    c.muted = rgb(p.surface).into();
    c.secondary = rgb(p.surface).into();
    c.skeleton = rgb(p.surface).into();
    c.list_even = rgb(p.surface).into();
    c.table_even = rgb(p.surface).into();
    c.description_list_label = rgb(p.surface).into();
    c.switch = rgb(p.surface).into();
    c.tab_bar_segmented = rgb(p.surface).into();
    c.border = rgb(p.border).into();
    c.input = rgb(p.border).into();
    c.window_border = rgb(p.border).into();
    c.sidebar_border = rgb(p.border).into();
    c.table_row_border = rgb(p.border).into();
    c.title_bar_border = rgb(p.border).into();
    c.slider_bar = rgb(p.guide).into();
    c.accent = rgb(p.selected).into();
    c.list_active = rgb(p.selected).into();
    c.table_active = rgb(p.selected).into();
    c.sidebar_accent = rgb(p.selected).into();
    c.drop_target = rgb(p.cursor_line).into();
    c.foreground = rgb(p.text).into();
    c.popover_foreground = rgb(p.text).into();
    c.group_box_foreground = rgb(p.text).into();
    c.secondary_foreground = rgb(p.text).into();
    c.sidebar_foreground = rgb(p.text).into();
    c.table_head_foreground = rgb(p.text).into();
    c.tab_active_foreground = rgb(p.text).into();
    c.accent_foreground = rgb(p.text).into();
    c.sidebar_accent_foreground = rgb(p.text).into();
    c.description_list_label_foreground = rgb(p.text).into();
    c.muted_foreground = rgb(p.muted).into();
    c.tab_foreground = rgb(p.muted).into();
    c.scrollbar_thumb = rgb(p.muted).into();
    c.caret = rgb(p.accent).into();
    c.ring = rgb(p.accent).into();
    c.link = rgb(p.accent).into();
    c.link_hover = rgb(p.accent).into();
    c.link_active = rgb(p.accent).into();
    c.drag_border = rgb(p.accent).into();
    c.sidebar_primary = rgb(p.accent).into();
    c.list_active_border = rgb(p.accent).into();
    c.table_active_border = rgb(p.accent).into();
    c.slider_thumb = rgb(p.accent).into();
    c.progress_bar = rgb(p.accent).into();
    c.scrollbar_thumb_hover = rgb(p.accent).into();
    c.info = rgb(p.accent).into();
    c.info_hover = rgb(p.accent).into();
    c.info_active = rgb(p.accent).into();
    c.secondary_hover = rgb(p.secondary_hover).into();
    c.accordion_hover = rgb(p.secondary_hover).into();
    c.list_hover = rgb(p.secondary_hover).into();
    c.table_hover = rgb(p.secondary_hover).into();
    c.secondary_active = rgb(p.secondary_active).into();
    c.primary = rgb(p.primary).into();
    c.switch_thumb = rgb(p.primary).into();
    c.primary_hover = rgb(p.primary_hover).into();
    c.primary_active = rgb(p.primary_active).into();
    c.primary_foreground = rgb(p.primary_foreground).into();
    c.sidebar_primary_foreground = rgb(p.primary_foreground).into();
    c.scrollbar = rgb(p.scrollbar_track).into();
    c.title_bar = rgb(p.title_bar).into();
    c.success = rgb(p.green).into();
    c.success_hover = rgb(p.green).into();
    c.success_active = rgb(p.green).into();
    c.bullish = rgb(p.green).into();
    c.danger = rgb(p.red).into();
    c.danger_hover = rgb(p.red).into();
    c.danger_active = rgb(p.red).into();
    c.bearish = rgb(p.red).into();
    c.warning = rgb(p.yellow).into();
    c.warning_hover = rgb(p.yellow).into();
    c.warning_active = rgb(p.yellow).into();
    c.selection = Hsla::from(rgb(p.accent)).opacity(0.28);
    c.overlay = Hsla::from(rgb(p.title_bar)).opacity(0.65);
    c.red = rgb(selected.ansi[1]).into();
    c.red_light = rgb(selected.ansi[9]).into();
    c.green = rgb(selected.ansi[2]).into();
    c.green_light = rgb(selected.ansi[10]).into();
    c.yellow = rgb(selected.ansi[3]).into();
    c.yellow_light = rgb(selected.ansi[11]).into();
    c.blue = rgb(selected.ansi[4]).into();
    c.blue_light = rgb(selected.ansi[12]).into();
    c.magenta = rgb(selected.ansi[5]).into();
    c.magenta_light = rgb(selected.ansi[13]).into();
    c.cyan = rgb(selected.ansi[6]).into();
    c.cyan_light = rgb(selected.ansi[14]).into();
    c.chart_1 = rgb(p.accent).into();
    c.chart_2 = rgb(p.green).into();
    c.chart_3 = rgb(p.yellow).into();
    c.chart_4 = rgb(p.red).into();
    c.chart_5 = rgb(p.syntax_keyword).into();
    let mut highlight = (*t.highlight_theme).clone();
    highlight.name = selected.name.clone();
    highlight.appearance = mode;
    highlight.style.editor_background = Some(rgb(p.background).into());
    highlight.style.editor_foreground = Some(rgb(p.text).into());
    highlight.style.editor_active_line = Some(rgb(p.cursor_line).into());
    highlight.style.editor_line_number = Some(rgb(p.muted).into());
    highlight.style.editor_active_line_number = Some(rgb(p.accent).into());
    let syntax = &mut highlight.style.syntax;
    // ThemeStyle's color fields are private; its supported JSON interface accepts Hsla.
    let style = |color| {
        Some(
            serde_json::from_value(serde_json::json!({
                "color": Hsla::from(rgb(color)), "font_style": null, "font_weight": null
            }))
            .expect("valid syntax style"),
        )
    };
    macro_rules! syntax {
        ($color:expr; $($field:ident),+ $(,)?) => { $(syntax.$field = style($color);)+ };
    }
    syntax!(p.text; embedded, emphasis, emphasis_strong, label, primary, punctuation,
        punctuation_bracket, punctuation_delimiter, punctuation_list_marker, punctuation_special);
    syntax!(p.syntax_comment; comment, comment_doc, hint, predictive);
    syntax!(p.syntax_keyword; keyword, operator, preproc, attribute);
    syntax!(p.syntax_number; number, boolean, constant);
    syntax!(p.syntax_string; string, string_escape, string_regex, string_special,
        string_special_symbol, text_literal);
    syntax!(p.syntax_type; type_, constructor, enum_, variant, tag, tag_doctype);
    syntax!(p.syntax_variable; variable, variable_special, property);
    syntax!(p.syntax_function; function, title, link_text, link_uri);
    t.highlight_theme = Arc::new(highlight);
    theme::set(selected);
    cx.refresh_windows();
}

pub fn init(cx: &mut App) {
    let file = settings::store();
    let loaded = file.as_ref().map(|path| Settings::load(path)).transpose();
    let initial = match loaded {
        Ok(settings) => settings.unwrap_or_default(),
        Err(error) => {
            eprintln!("give: could not load settings: {error:#}");
            Settings::default()
        }
    };
    apply(initial, cx);
    let Some(file) = file else {
        return;
    };
    watch(file, cx);
}

pub(crate) fn watch(file: std::path::PathBuf, cx: &mut App) {
    let mut seen = std::fs::read(&file).ok();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let file = file.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = std::fs::read(&file).ok();
                    let settings = Settings::load(&file);
                    (bytes, settings)
                })
                .await;
            if result.0 == seen {
                continue;
            }
            seen = result.0;
            match result.1 {
                Ok(settings) => {
                    if cx.update(|cx| apply(settings, cx)).is_err() {
                        break;
                    }
                }
                Err(error) => eprintln!("give: could not reload settings: {error:#}"),
            }
        }
    })
    .detach();
}
