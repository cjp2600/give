# give

Native desktop companion to [giv](../giv), built with Rust and [GPUI](https://gpui.rs/). This first version focuses on local Git review: working/staged files, virtualized unified diffs, filtering, per-file and repository-wide staging, and committing the existing index.

## Run

```sh
cd /Users/stanislavsemenov/pp/give
make run
# Open a repository immediately:
make run ARGS=../giv
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

Open repositories through the native folder chooser (⌘O). Switch Working/Staged with the tabs; use arrow keys to select files. ⌘R refreshes, ⌘⇧C copies the full selected diff, and ⌘↵ commits staged changes when a message is present. The repository refreshes every two seconds. No files are automatically staged or committed. There are no discard, push, or publish actions in this version.

## Structure

- `src/git.rs`: repository operations, bounded subprocess output/timeouts, NUL-delimited status parsing, diff preparation. No UI dependency in this module.
- `src/ui.rs`: asynchronous GPUI views, file filtering, selection, and the commit workflow.
- `src/main.rs`: application window, menus, and keyboard actions.
- `assets/icons/`: source artwork and platform variants.
- `scripts/`: reproducible icon and macOS bundle generation.
- `tests/repository.rs`: real-Git integration cases, using disposable fixtures only.

GPUI and GPUI Component are pinned to compatible published versions (`0.2.2` and `0.5.1`). All rendering is native GPUI; no WebView feature is enabled. The original Go/Bubble Tea `giv` is a separate application. Agent Dock, Live Review notes, and GitLab reviews have not yet been ported.

## UI direction

A cartoon welding helmet is the app mark: warm orange shell, dark frame, cool blue glass. Charcoal surfaces keep code quiet; amber identifies primary actions, green/red indicate additions/deletions. The icon master and light/dark app icons are generated with the built-in imagegen tool. Prompts and exported sizes are documented in `assets/icons/README.md`.

macOS is the platform verified for this first version. GPUI is cross-platform, but Windows and Linux builds still need to be validated on those systems.
