mod accessibility;
mod terminal_view;
mod ui;

use gpui::{actions, prelude::*, *};
use gpui_component::{Root, Theme, ThemeMode};
use std::{borrow::Cow, path::PathBuf};

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "give-master.png" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/give-master.png"
            ))),
            "icons/folder.svg" => Some(Cow::Borrowed(include_bytes!("../assets/ui/folder.svg"))),
            "icons/folder-open.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/ui/folder-open.svg"
            ))),
            "icons/panel-right.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/ui/panel-right.svg"
            ))),
            "icons/file.svg" => Some(Cow::Borrowed(include_bytes!("../assets/ui/file.svg"))),
            "icons/code.svg" => Some(Cow::Borrowed(include_bytes!("../assets/ui/code.svg"))),
            "icons/branch.svg" => Some(Cow::Borrowed(include_bytes!("../assets/ui/branch.svg"))),
            "icons/chevron-right.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/ui/chevron-right.svg"
            ))),
            "icons/chevron-down.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/ui/chevron-down.svg"
            ))),
            "icons/search.svg" => Some(Cow::Borrowed(include_bytes!("../assets/ui/search.svg"))),
            "icons/config.svg" => Some(Cow::Borrowed(include_bytes!("../assets/ui/config.svg"))),
            "icons/markdown.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/ui/markdown.svg")))
            }
            _ => None,
        })
    }
    fn list(&self, prefix: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok([
            "give-master.png",
            "icons/folder.svg",
            "icons/folder-open.svg",
            "icons/file.svg",
            "icons/panel-right.svg",
            "icons/code.svg",
            "icons/branch.svg",
            "icons/chevron-right.svg",
            "icons/chevron-down.svg",
            "icons/search.svg",
            "icons/config.svg",
            "icons/markdown.svg",
        ]
        .into_iter()
        .filter(|path| path.starts_with(prefix))
        .map(Into::into)
        .collect())
    }
}

actions!(
    give,
    [
        Quit,
        OpenRepository,
        Refresh,
        Commit,
        NextFile,
        PreviousFile,
        SwitchPane,
        ToggleDeletions,
        NextChange,
        ActivateRow,
        CollapseRow,
        ExpandRow,
        PageDown,
        PageUp,
        FirstRow,
        LastRow,
        CopyDiff,
        ToggleTerminal,
        CopySelection,
        SelectAllText,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        ReviewBranch,
        ExitReview,
        PickerUp,
        PickerDown,
        PickerCancel
    ]
);

fn main() {
    let mut initial = None;
    let mut review = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let text = argument.to_string_lossy();
        if text == "--help" {
            println!(
                "give [repository] [--review <branch>]\n\nNative Git review on GPUI. Without a path, open a repository from the welcome screen.\n--review reviews a branch against main, like giv --review."
            );
            return;
        } else if text == "--review" {
            let Some(branch) = arguments.next() else {
                eprintln!("--review needs a branch name");
                std::process::exit(2);
            };
            review = Some(branch.to_string_lossy().into_owned());
        } else if let Some(branch) = text.strip_prefix("--review=") {
            review = Some(branch.to_owned());
        } else {
            initial = Some(PathBuf::from(argument));
        }
    }
    if review.is_some() && initial.is_none() {
        initial = std::env::current_dir().ok();
    }
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            terminal_view::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            let theme = Theme::global_mut(cx);
            theme.colors.background = rgb(0x1b2632).into();
            theme.colors.foreground = rgb(0xcbd5e1).into();
            theme.colors.border = rgb(0x2a394b).into();
            theme.colors.input = rgb(0x1c2a39).into();
            theme.colors.muted = rgb(0x223142).into();
            theme.colors.muted_foreground = rgb(0x7f92a8).into();
            theme.colors.secondary = rgb(0x25364a).into();
            theme.colors.secondary_hover = rgb(0x2e425a).into();
            theme.colors.secondary_active = rgb(0x354d67).into();
            theme.colors.secondary_foreground = rgb(0xcbd5e1).into();
            theme.colors.selection = rgb(0x304b67).into();
            theme.colors.title_bar = rgb(0x151e28).into();
            theme.colors.title_bar_border = rgb(0x263445).into();
            theme.colors.primary = rgb(0x83afd5).into();
            theme.colors.primary_hover = rgb(0xa2c8e7).into();
            theme.colors.primary_active = rgb(0x6997c0).into();
            theme.colors.primary_foreground = rgb(0x142334).into();
            theme.colors.ring = rgb(0x83afd5).into();
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new(
                    "secondary-c",
                    CopySelection,
                    Some("Give && !Input && !Terminal"),
                ),
                KeyBinding::new(
                    "secondary-a",
                    SelectAllText,
                    Some("Give && !Input && !Terminal"),
                ),
                KeyBinding::new(
                    "shift-left",
                    SelectLeft,
                    Some("Give && !Input && !Terminal"),
                ),
                KeyBinding::new(
                    "shift-right",
                    SelectRight,
                    Some("Give && !Input && !Terminal"),
                ),
                KeyBinding::new("shift-up", SelectUp, Some("Give && !Input && !Terminal")),
                KeyBinding::new(
                    "shift-down",
                    SelectDown,
                    Some("Give && !Input && !Terminal"),
                ),
                KeyBinding::new("secondary-b", ToggleTerminal, Some("Give")),
                KeyBinding::new("secondary-q", Quit, None),
                KeyBinding::new("secondary-o", OpenRepository, Some("Give")),
                KeyBinding::new("secondary-r", Refresh, Some("Give && !Terminal")),
                KeyBinding::new("secondary-enter", Commit, Some("Give && !Terminal")),
                KeyBinding::new("down", NextFile, Some("Give && !Input && !Terminal")),
                KeyBinding::new("up", PreviousFile, Some("Give && !Input && !Terminal")),
                KeyBinding::new("tab", SwitchPane, Some("Give && !Input && !Terminal")),
                KeyBinding::new("shift-tab", SwitchPane, Some("Give && !Input && !Terminal")),
                KeyBinding::new("d", ToggleDeletions, Some("Give && !Input && !Terminal")),
                KeyBinding::new("в", ToggleDeletions, Some("Give && !Input && !Terminal")),
                KeyBinding::new("space", NextChange, Some("Give && !Input && !Terminal")),
                KeyBinding::new("ctrl-n", NextChange, Some("Give && !Input && !Terminal")),
                KeyBinding::new("j", NextFile, Some("Give && !Input && !Terminal")),
                KeyBinding::new("о", NextFile, Some("Give && !Input && !Terminal")),
                KeyBinding::new("k", PreviousFile, Some("Give && !Input && !Terminal")),
                KeyBinding::new("л", PreviousFile, Some("Give && !Input && !Terminal")),
                KeyBinding::new("enter", ActivateRow, Some("Give && !Input && !Terminal")),
                KeyBinding::new("left", CollapseRow, Some("Give && !Input && !Terminal")),
                KeyBinding::new("right", ExpandRow, Some("Give && !Input && !Terminal")),
                KeyBinding::new("pagedown", PageDown, Some("Give && !Input && !Terminal")),
                KeyBinding::new("pageup", PageUp, Some("Give && !Input && !Terminal")),
                KeyBinding::new("home", FirstRow, Some("Give && !Input && !Terminal")),
                KeyBinding::new("end", LastRow, Some("Give && !Input && !Terminal")),
                KeyBinding::new("secondary-shift-c", CopyDiff, Some("Give && !Terminal")),
                KeyBinding::new("secondary-shift-r", ReviewBranch, Some("Give && !Terminal")),
                KeyBinding::new("up", PickerUp, Some("BranchPicker > Input")),
                KeyBinding::new("down", PickerDown, Some("BranchPicker > Input")),
                KeyBinding::new("escape", PickerCancel, Some("BranchPicker")),
            ]);
            cx.set_menus(vec![
                Menu {
                    name: "give".into(),
                    items: vec![MenuItem::action("Quit give", Quit)],
                },
                Menu {
                    name: "File".into(),
                    items: vec![
                        MenuItem::action("Open Repository…", OpenRepository),
                        MenuItem::action("Refresh", Refresh),
                    ],
                },
                Menu {
                    name: "View".into(),
                    items: vec![MenuItem::action("Toggle Right Terminal", ToggleTerminal)],
                },
                Menu {
                    name: "Changes".into(),
                    items: vec![
                        MenuItem::action("Switch Tree / Preview", SwitchPane),
                        MenuItem::action("Show / Hide Deleted Lines", ToggleDeletions),
                        MenuItem::action("Next Change", NextChange),
                        MenuItem::action("Copy Diff", CopyDiff),
                        MenuItem::action("Commit Staged Changes", Commit),
                    ],
                },
                Menu {
                    name: "Review".into(),
                    items: vec![
                        MenuItem::action("Review Branch…", ReviewBranch),
                        MenuItem::action("Exit Review", ExitReview),
                    ],
                },
            ]);
            let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
            let window = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(600.))),
                    titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                    ..Default::default()
                },
                |window, cx| {
                    let view =
                        cx.new(|cx| ui::Give::new(initial.clone(), review.clone(), window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            );
            match window {
                Ok(_) => cx.activate(true),
                Err(error) => {
                    eprintln!("Could not open give: {error:#}");
                    cx.quit();
                }
            }
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
}
