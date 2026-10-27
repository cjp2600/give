# give themes

Each JSON file is a complete palette: semantic UI roles, syntax token colors and the sixteen ANSI terminal colors. `src/theme.rs` validates and embeds them at build time; add its `include_str!` entry when adding a built-in theme. `#RRGGBB` is the only accepted color format. Light themes set `dark` to `false` so native controls and rendered Markdown use the right appearance.

The UI roles adapt these upstream palettes to give's layout; derived backgrounds for selections, search and diffs are intentionally softer than the accent colors. The original give palette remains the default.

| Theme | Upstream palette |
| --- | --- |
| give Dark | Original give colors |
| Nord | [Nord colors and palettes](https://www.nordtheme.com/docs/colors-and-palettes/) |
| Dracula | [Dracula specification](https://spec.draculatheme.com/) |
| Tokyo Night | [Enkia's Tokyo Night](https://github.com/tokyo-night/tokyo-night-vscode-theme) |
| Catppuccin Mocha / Latte | [Catppuccin palettes](https://github.com/catppuccin/catppuccin#-palette) |

ANSI truecolor supplied by terminal applications is preserved. The standard 216-color cube and grayscale ramp are protocol values; only the sixteen theme colors and default foreground, background and cursor are themed. Cached source highlights retain token roles so they change colors without reloading files.
