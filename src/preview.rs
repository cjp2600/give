//! Full-file overlay ported from giv's tui/diffview.go and tui/preview.go.
use crate::git::{Diff, DiffLine, LineKind};
use std::{ops::Range, path::Path, sync::LazyLock};
use syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    parsing::{SyntaxDefinition, SyntaxSet, SyntaxSetBuilder},
};

pub const MAX_LINES: usize = 6000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub color: u32,
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_nonewlines);
/// Grammars missing from Syntect's bundled set, kept separate so the bundled
/// dump loads without relinking.
static EXTRA_SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(|| {
    let mut builder = SyntaxSetBuilder::new();
    builder.add(
        SyntaxDefinition::load_from_str(
            include_str!("../assets/syntaxes/Dart.sublime-syntax"),
            false,
            None,
        )
        .expect("bundled Dart grammar"),
    );
    builder.build()
});
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

pub fn warm_up() {
    LazyLock::force(&SYNTAXES);
    LazyLock::force(&EXTRA_SYNTAXES);
    LazyLock::force(&THEMES);
}

/// Highlight each displayed source row, as Chroma does in giv. Byte ranges are
/// retained so GPUI can shape the entire row without creating an element per token.
pub fn highlight(path: &Path, lines: &[DiffLine]) -> Vec<Vec<Highlight>> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let extension = path.extension().unwrap_or_default().to_string_lossy();
    let find = |set: &'static SyntaxSet| {
        set.find_syntax_by_extension(&name)
            .or_else(|| set.find_syntax_by_extension(&extension))
            .map(|syntax| (syntax, set))
    };
    let (syntax, set) = find(&SYNTAXES)
        .or_else(|| find(&EXTRA_SYNTAXES))
        .unwrap_or_else(|| (SYNTAXES.find_syntax_plain_text(), &SYNTAXES));
    lines
        .iter()
        .map(|line| {
            let mut highlighter = HighlightLines::new(syntax, &THEMES.themes["base16-ocean.dark"]);
            let mut offset = 0;
            highlighter
                .highlight_line(&line.text, set)
                .unwrap_or_default()
                .into_iter()
                .map(|(style, text)| {
                    let start = offset;
                    offset += text.len();
                    // Muted blue editor palette; preserve distinct token categories.
                    let color = match (style.foreground.r, style.foreground.g, style.foreground.b) {
                        (101, 115, 126) => 0x74879c, // comment
                        (180, 142, 173) => 0xb49adb, // keyword
                        (163, 190, 140) => 0x9fbea8, // string
                        (208, 135, 112) => 0xd6bb85, // number
                        (143, 161, 179) => 0x8cb5d6, // type
                        (136, 192, 208) => 0x8cb5d6,
                        (191, 97, 106) => 0xd78b97,
                        (235, 203, 139) => 0x8db5d0,
                        _ => 0xcbd5e1,
                    };
                    Highlight {
                        range: start..offset,
                        color,
                    }
                })
                .collect()
        })
        .collect()
}

pub fn source_lines(body: &[u8]) -> Vec<String> {
    let body = body.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(body);
    let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<_> = text.split('\n').map(str::to_owned).collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

pub fn expand_tabs(text: &str) -> String {
    let mut result = String::new();
    let mut column = 0;
    for c in text.chars() {
        match c {
            '\t' => {
                let n = 8 - column % 8;
                result.extend(std::iter::repeat_n(' ', n));
                column += n;
            }
            '\n' => {
                result.push(c);
                column = 0;
            }
            '\r' => result.push(c),
            _ => {
                result.push(c);
                column += 1;
            }
        }
    }
    result
}

fn row(text: &str, kind: LineKind, old: Option<usize>, new: Option<usize>) -> DiffLine {
    DiffLine {
        text: expand_tabs(text),
        kind,
        old,
        new,
    }
}

#[derive(Debug)]
struct Hunk {
    old: usize,
    new: usize,
    ops: Vec<(char, String)>,
}

fn parse_hunks(raw: &str) -> Vec<Hunk> {
    let normalized = raw.replace("\r\n", "\n");
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in normalized.split('\n') {
        if line.starts_with("@@") {
            let mut parts = line.split_whitespace();
            let header = parts.next();
            let old = parts
                .next()
                .and_then(|p| p.strip_prefix('-'))
                .and_then(|p| p.split(',').next()?.parse::<usize>().ok());
            let new = parts
                .next()
                .and_then(|p| p.strip_prefix('+'))
                .and_then(|p| p.split(',').next()?.parse::<usize>().ok());
            if let (Some("@@"), Some(old), Some(new), Some("@@")) = (header, old, new, parts.next())
            {
                hunks.push(Hunk {
                    old,
                    new,
                    ops: Vec::new(),
                });
            }
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            continue;
        };
        if line.starts_with("diff --git") {
            break;
        }
        if line.is_empty() {
            hunk.ops.push((' ', String::new()));
        } else if matches!(line.as_bytes()[0], b' ' | b'+' | b'-') {
            hunk.ops
                .push((line.as_bytes()[0] as char, line[1..].to_owned()));
        }
    }
    for hunk in &mut hunks {
        while hunk
            .ops
            .last()
            .is_some_and(|(kind, text)| *kind == ' ' && text.is_empty())
        {
            hunk.ops.pop();
        }
    }
    hunks.sort_by_key(|h| (h.new, h.old));
    hunks
}

pub fn full_context(body: &[u8]) -> Vec<DiffLine> {
    source_lines(body)
        .iter()
        .take(MAX_LINES)
        .enumerate()
        .map(|(i, text)| row(text, LineKind::Context, Some(i + 1), Some(i + 1)))
        .collect()
}

/// Unchanged regions and hunk additions always read from the current full file.
/// Removed text comes from the patch. Giv uses current-file numbers in both
/// context gutters, including after insertions/deletions; preserve that rule.
pub fn overlay(body: &[u8], raw: &str, show_deletions: bool) -> Option<Vec<DiffLine>> {
    let full = source_lines(body);
    let hunks = parse_hunks(raw);
    if hunks.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    let mut pos = 1;
    for hunk in hunks {
        while pos < hunk.new && pos <= full.len() {
            if lines.len() >= MAX_LINES {
                return Some(lines);
            }
            lines.push(row(&full[pos - 1], LineKind::Context, Some(pos), Some(pos)));
            pos += 1;
        }
        let (mut new, mut old) = (hunk.new, hunk.old);
        for (kind, text) in hunk.ops {
            if lines.len() >= MAX_LINES {
                return Some(lines);
            }
            match kind {
                ' ' | '+' => {
                    let text = full.get(new.checked_sub(1)?)?;
                    lines.push(row(
                        text,
                        if kind == '+' {
                            LineKind::Added
                        } else {
                            LineKind::Context
                        },
                        if kind == '+' { None } else { Some(new) },
                        Some(new),
                    ));
                    new += 1;
                    if kind == ' ' {
                        old += 1;
                    }
                    pos = new;
                }
                '-' => {
                    if show_deletions {
                        lines.push(row(&text, LineKind::Removed, Some(old), None));
                    }
                    old += 1;
                }
                _ => {}
            }
        }
    }
    while pos <= full.len() && lines.len() < MAX_LINES {
        lines.push(row(&full[pos - 1], LineKind::Context, Some(pos), Some(pos)));
        pos += 1;
    }
    Some(lines)
}

pub fn fallback(diff: &Diff, show_deletions: bool) -> Vec<DiffLine> {
    diff.lines
        .iter()
        .filter(|line| {
            line.kind != LineKind::Hunk && (show_deletions || line.kind != LineKind::Removed)
        })
        .take(MAX_LINES)
        .map(|line| {
            let text = if matches!(
                line.kind,
                LineKind::Added | LineKind::Removed | LineKind::Context
            ) {
                line.text.get(1..).unwrap_or_default()
            } else {
                &line.text
            };
            row(text, line.kind, line.old, line.new)
        })
        .collect()
}

pub fn change_blocks(lines: &[DiffLine]) -> Vec<usize> {
    let mut previous = None;
    let mut starts = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if matches!(line.kind, LineKind::Added | LineKind::Removed) {
            if previous.is_none() || previous != i.checked_sub(1) {
                starts.push(i);
            }
            previous = Some(i);
        }
    }
    starts
}

pub fn known_text(path: &Path) -> bool {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    matches!(
        ext.as_str(),
        "go" | "proto"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "mjs"
            | "cjs"
            | "mts"
            | "cts"
            | "vue"
            | "svelte"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "less"
            | "sass"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "csv"
            | "tsv"
            | "env"
            | "ini"
            | "cfg"
            | "conf"
            | "py"
            | "rb"
            | "pl"
            | "pm"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "lua"
            | "php"
            | "r"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "hh"
            | "cxx"
            | "hxx"
            | "m"
            | "mm"
            | "swift"
            | "rs"
            | "zig"
            | "nim"
            | "java"
            | "kt"
            | "kts"
            | "scala"
            | "groovy"
            | "gradle"
            | "clj"
            | "cs"
            | "fs"
            | "vb"
            | "csproj"
            | "md"
            | "rst"
            | "txt"
            | "tex"
            | "adoc"
            | "org"
            | "make"
            | "cmake"
            | "dockerfile"
            | "tf"
            | "hcl"
            | "nix"
            | "sql"
            | "graphql"
            | "gql"
            | "thrift"
            | "avsc"
            | "fbs"
            | "dart"
            | "elm"
            | "ex"
            | "exs"
            | "erl"
            | "hrl"
            | "hs"
    )
}

pub fn binary_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "ico"
            | "webp"
            | "tiff"
            | "tif"
            | "mp3"
            | "mp4"
            | "wav"
            | "ogg"
            | "flac"
            | "avi"
            | "mkv"
            | "mov"
            | "webm"
            | "zip"
            | "tar"
            | "gz"
            | "bz2"
            | "xz"
            | "zst"
            | "7z"
            | "rar"
            | "o"
            | "a"
            | "so"
            | "dylib"
            | "dll"
            | "exe"
            | "class"
            | "pyc"
            | "pyo"
            | "wasm"
            | "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "ttf"
            | "otf"
            | "woff"
            | "woff2"
            | "eot"
            | "db"
            | "sqlite"
            | "sqlite3"
            | "bin"
            | "dat"
            | "pak"
            | "ds_store"
    )
}

pub fn binary_content(body: &[u8]) -> bool {
    if body.iter().take(8000).any(|b| *b == 0) {
        return true;
    }
    if body.len() < 4 {
        return false;
    }
    let magic = u32::from_be_bytes(body[..4].try_into().unwrap());
    matches!(
        magic,
        0x7f454c46
            | 0xfeedface
            | 0xfeedfacf
            | 0xcefaedfe
            | 0xcffaedfe
            | 0xcafebabe
            | 0x504b0304
            | 0x25504446
    ) || body.starts_with(b"MZ")
        || body.starts_with(&[0x1f, 0x8b])
}

pub fn binary_notice(raw: &str) -> bool {
    raw.lines().any(|line| {
        let line = line.trim();
        line.starts_with("Binary files ") && line.ends_with(" differ")
    })
}
