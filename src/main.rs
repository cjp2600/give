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
            _ => None,
        })
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec!["give-master.png".into()])
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
        ToggleLayer,
        CopyDiff
    ]
);

fn main() {
    let argument = std::env::args_os().nth(1);
    if argument.as_deref() == Some(std::ffi::OsStr::new("--help")) {
        println!(
            "give [repository]\n\nNative Git review on GPUI. Without a path, open a repository from the welcome screen."
        );
        return;
    }
    let initial = argument.map(PathBuf::from);
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            let theme = Theme::global_mut(cx);
            theme.colors.primary = rgb(0xf5aa57).into();
            theme.colors.primary_hover = rgb(0xffbd75).into();
            theme.colors.primary_active = rgb(0xd98d3f).into();
            theme.colors.primary_foreground = rgb(0x231b12).into();
            theme.colors.ring = rgb(0xf5aa57).into();
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("secondary-q", Quit, None),
                KeyBinding::new("secondary-o", OpenRepository, Some("Give")),
                KeyBinding::new("secondary-r", Refresh, Some("Give")),
                KeyBinding::new("secondary-enter", Commit, Some("Give")),
                KeyBinding::new("down", NextFile, Some("Give && !Input")),
                KeyBinding::new("up", PreviousFile, Some("Give && !Input")),
                KeyBinding::new("tab", ToggleLayer, Some("Give && !Input")),
                KeyBinding::new("secondary-shift-c", CopyDiff, Some("Give")),
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
                    name: "Changes".into(),
                    items: vec![
                        MenuItem::action("Switch Working / Staged", ToggleLayer),
                        MenuItem::action("Copy Diff", CopyDiff),
                        MenuItem::action("Commit Staged Changes", Commit),
                    ],
                },
            ]);
            let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
            let window = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(600.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("give".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| ui::Give::new(initial, window, cx));
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
