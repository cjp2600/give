# give

Native Rust desktop version of [giv](../giv), built with Rust and [GPUI](https://gpui.rs/). The basic review workflow follows giv: a collapsible change tree on the left and the full current file on the right, with Git changes overlaid on its contents.

## Run

```sh
cd /Users/stanislavsemenov/pp/give
make run
# Open a repository immediately:
make run ARGS=../giv
# Review a branch against main, like `giv --review`:
make run ARGS="../giv --review my-feature"
```

Requires Git, Rust 1.94+, and platform build dependencies for GPUI. On macOS, install Xcode. The GPUI `runtime_shaders` feature compiles Metal shaders at app startup, so a separately downloaded Metal compiler is not required. On Apple Silicon, use an **aarch64** Rust host toolchain so bindgen can load the native libclang. This Mac has a project-local rustup override; other projects keep their own toolchains.

## Commands

| Command | Result |
| --- | --- |
| `make run` | Build and start a development app (a `.app` bundle with Dock icon on macOS) |
| `make build` | Build optimized executable |
| `make test` | Run tests against temporary Git repositories |
| `make check` | Verify formatting and run Clippy |
| `make fmt` | Format Rust source |
| `make icons` | Rebuild platform icon sizes from the artwork |
| `make app` | Build `dist/give.app` with its macOS icon |
| `make install` | Build the optimized release bundle and copy it to `/Applications/give.app` |

Open repositories through the native folder chooser (⌘O). The left tree contains **all** changed paths in separate tracked and untracked sections. File names are colored by change: blue modified, green newly added, grey deleted, red untracked (not yet added to Git) or conflicted. Directories sort before files; single-child directory chains compact into `parent/child`. Click a folder or press Enter to expand/collapse it. Folder state and file selection survive refreshes. Filtering temporarily expands matching paths.

The right pane follows `giv`'s `BuildPreview` and `renderFullFileWithDiff` algorithms: read the full current file and overlay `git diff -w HEAD` hunks. Unchanged regions remain visible, additions have green backgrounds, and **deletions are hidden by default**. Press **D** to insert deleted rows in red with a second line-number gutter. The preview includes staged and unstaged edits together, ignores whitespace-only differences, expands tabs to 8-column stops, and displays up to 6,000 rows. New/untracked files show full content without change backgrounds, matching giv. Missing files or an invalid overlay use a patch fallback; binary files show a notice. Syntax uses Syntect grammars with a muted blue editor palette; token boundaries can differ from giv's Chroma lexer.

**Tab** switches tree/preview focus. **↑↓ / J K** navigate, **PgUp/PgDn** move a page, and **Home/End** jump to the boundaries. **Space / Ctrl+N** cycle change blocks and center them, including at EOF. The always-visible right scrollbar marks every highlighted change at its relative file position. Click a marker to center that change, click the track to jump, or drag the thumb to scroll. Lines wider than the pane scroll horizontally (trackpad or Shift+wheel). Each pane scrolls independently; scrolling the tree does not change selection. D/J/K shortcuts also work at their Russian-layout positions. Text fields retain normal text input.

Select source text by dragging across rows; double-click selects a word, triple-click selects a line, and Shift extends a selection. ⌘A selects the displayed file and ⌘C copies the selection without gutters or diff markers. Shift+arrows extend a selection with the keyboard. Dragging outside the preview scrolls it. On macOS both source and terminal selections expose `AXSelectedText`, UTF-16 ranges and native Services copying for selection readers; the reader's own hotkey remains available.

The title-bar panel button or **⌘B** toggles a resizable terminal on the right. It runs the user's login shell on a real PTY using Alacritty's terminal engine and portable-pty, with ANSI/truecolor output, alternate screens, history, Unicode/IME input, selection/copy, bracketed paste and mouse reporting. Each opened project has a separate session starting in its repository root. Switching back restores that session, including running processes; collapsing the panel keeps it alive. Sessions end when the app closes. A finished shell can be restarted from its header. The terminal grid is always 100 columns wide, whatever the panel width: a narrower panel scrolls it horizontally (trackpad or Shift+wheel, with a thin position bar), and a moved cursor is brought back into view. Rows are 17 px, close to Menlo's own line height. Box-drawing characters (U+2500–U+257F, except diagonals) and block elements (U+2580–U+259F) are painted as pixel-snapped cell rectangles instead of font glyphs, as Alacritty and Ghostty do, so TUI frames, borders and bars join without gaps; rounded corners are drawn square.

**⌘V** in the terminal pastes ordinary text. For clipboard images it first saves the original encoded bytes to a unique private `give-clipboard-*` directory in the system temporary folder, then inserts a quoted absolute path, without pressing Enter. File writing runs in the background; subsequent input remains ordered. Files remain available after the clipboard changes or the app exits, until the temporary directory is removed. The terminal research and library comparison are in [research/native-terminal.md](research/native-terminal.md).

⌘R refreshes, ⌘⇧C copies the selected patch, and ⌘↵ commits the existing staged index when a message is present. Stage controls remain available; no files are automatically staged or committed. Git reads, full-file overlays, tree construction/filtering, and syntax highlighting run on background workers. Content paints before syntax is ready. Syntax is computed lazily in 48-row chunks around the visible viewport; switching D reuses unchanged rows. A bounded LRU cache (12 files / approximately 24 MiB, retaining the current file if larger) stores both deletion modes with shared references; at most two neighboring files are prefetched. Concurrent requests for the same file share one load. Per-file versions invalidate changed previews while unchanged files remain cached. Refresh keeps the current content visible and does not rebuild an unchanged tree. Filtering debounces by 75 ms. Both panes virtualize their visible rows.

**Review branch** in the title bar, **⌘⇧R** or `--review <branch>` opens giv's branch review. The picker lists local branches (current first), then remote-tracking branches, newest commit first; it shows known refs immediately and adds new remote branches after a background fetch. Type to filter; Enter reviews the highlighted branch, or the typed name when no branch has it. The review repeats giv's algorithm: `git fetch --all --prune`; for a GitLab `origin`, `glab` finds the open merge request for the branch (the remote prefix is removed) and its latest collected diff version supplies base and head; otherwise base is the merge-base with the first of `origin/main`, `origin/master`, `main`, `master`, and head is the branch (a name that exists only remotely falls back to `origin/<name>`). Files come from `git diff --name-status base head` (renames are their destination, modified). Previews read immutable blobs at head, never the working tree: deleted files show a notice, new files show every line as an addition, modified files overlay `git diff -w base head` with old and new line-number gutters. GitHub and other remotes review locally with a status note. The review is read-only and is not polled; ⌘R fetches and reloads it, **Exit review** returns to local changes. MR comments are not yet ported.

The repository refreshes every two seconds outside review. There are no discard, push, or publish actions in this version.

## Structure

- `src/git.rs`: repository operations, bounded subprocess output/timeouts, NUL-delimited status parsing, diff preparation. No UI dependency in this module.
- `src/tree.rs`: file trie, section ordering, directory compaction and collapse state.
- `src/preview.rs`: full-file diff overlay, deletion toggle, tab expansion, change blocks and syntax colors.
- `src/review.rs`: branch listing, fetch, GitLab merge request refs through `glab`, merge-base fallback, review file list and previews.
- `src/preview_cache.rs`: bounded preview cache, shared loading slots, reusable deletion modes and background syntax.
- `src/scrollbar.rs`: overview marker positions, thumb geometry and click targets.
- `src/terminal.rs`: background PTY lifecycle, ANSI grid, terminal input protocols and selection.
- `src/terminal_glyphs.rs`: box-drawing and block-element geometry.
- `src/terminal_view.rs`: native GPUI terminal rendering, IME, mouse/keyboard input and image paste.
- `src/clipboard_file.rs`: persistent temporary clipboard files and shell argument quoting.
- `src/text_selection.rs`: UTF-8 source selection and copying.
- `src/accessibility.rs` and `src/platform/selection.m`: macOS selection-reader and Services bridge.
- `src/ui.rs`: asynchronous GPUI views, file filtering, tree/preview navigation, and the commit workflow.
- `src/main.rs`: application window, menus, and keyboard actions.
- `assets/icons/`: source artwork and platform variants.
- `assets/ui/`: embedded SVG file, folder and navigation icons.
- `scripts/`: reproducible icon and macOS bundle generation.
- `examples/preview_profile.rs`: repeatable cold-content, lazy-syntax, cache and large-tree timings (`cargo run --example preview_profile`).
- `tests/repository.rs`: real-Git integration cases, using disposable fixtures only.
- `tests/review.rs`: branch review cases, including remote-only branches.
- `tests/preview.rs`: tree/preview cases, including 30 golden cases exported from the original Go renderer. Regenerate with `python3 scripts/generate-giv-preview-fixtures.py ../giv`; Go's overlay mechanism runs the exporter without modifying giv's files.

GPUI and GPUI Component are pinned to compatible published versions (`0.2.2` and `0.5.1`). All rendering is native GPUI; no WebView feature is enabled. The original Go/Bubble Tea `giv` is a separate application. Agent Dock, Live Review notes, and GitLab MR comments have not yet been ported; branch review has.

## UI direction

A custom title bar keeps native macOS window controls alongside the project and branch. Deep blue surfaces, soft blue accents, a 15 px code font with 26 px line height, compact file icons and indentation guides follow the supplied editor reference. Green/red backgrounds distinguish additions/deletions. The app mark is a single flat hexagon in sky blue on a midnight-blue tile. The SVG masters and all platform exports are documented in `assets/icons/README.md`.

Desktop selection and image-paste integration can be checked with `swift scripts/check-desktop.swift <give-executable> <temporary-repository>`; the first fixture must contain an untracked `readme.txt` with `Selection reader test` and `Русский текст для чтения`. The script starts its own process, restores the clipboard, and uses macOS Accessibility/Automation permissions. `scripts/check-selected-text.swift <pid> <expected-text>` probes a selection in an existing window.

macOS is the platform verified for this first version. GPUI is cross-platform, but Windows and Linux builds still need to be validated on those systems.
