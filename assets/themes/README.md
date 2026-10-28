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
| One Dark | [Atom One Dark syntax palette](https://github.com/atom/one-dark-syntax/blob/master/styles/colors.less) |
| Monokai | [VS Code's Monokai adaptation](https://github.com/microsoft/vscode/blob/main/extensions/theme-monokai/themes/monokai-color-theme.json) |
| Gruvbox Dark / Light | [Original Gruvbox palette](https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim) |
| Solarized Dark / Light | [Ethan Schoonover's Solarized](https://ethanschoonover.com/solarized/) |
| GitHub Dark / Light | [GitHub's VS Code themes](https://github.com/primer/github-vscode-theme), using its pinned [Primer primitives 7.10.0](https://github.com/primer/github-vscode-theme/blob/main/package.json) |

Text and muted UI colors in the additional themes are adjusted where necessary to retain at least 4.5:1 contrast on their surfaces. Selected rows, diff backgrounds and primary-button states also preserve readable text. Syntax accents retain each scheme's token colors. Solarized Light rebases the ANSI grayscale slots to keep them usable on a light background.

ANSI truecolor supplied by terminal applications is preserved. The standard 216-color cube and grayscale ramp are protocol values; only the sixteen theme colors and default foreground, background and cursor are themed. Cached source highlights retain token roles so they change colors without reloading files.
