//! "Copy path" references for pasting into an agent: an absolute file path,
//! `path:line`, or `path:start-end` followed by the selected source.
use crate::{git::DiffLine, text_selection::Selection};
use std::path::Path;

/// The displayed file line of a preview row; deleted rows keep their old number.
fn number(lines: &[DiffLine], row: usize) -> Option<usize> {
    lines.get(row).and_then(|line| line.new.or(line.old))
}

/// `path:line` for one preview row.
pub fn line(path: &Path, lines: &[DiffLine], row: usize) -> String {
    match number(lines, row) {
        Some(number) => format!("{}:{number}", path.display()),
        None => path.display().to_string(),
    }
}

/// `path:start-end` and the selected text in a fenced block; `None` when nothing is selected.
pub fn selection(path: &Path, lines: &[DiffLine], selection: Selection) -> Option<String> {
    let mut text = selection.text(lines);
    if text.is_empty() {
        return None;
    }
    let (start, end) = selection.ordered();
    // A drag that stops at the start of a row selects nothing on that row.
    let last = if end.byte == 0 && end.row > start.row {
        text.pop();
        end.row - 1
    } else {
        end.row
    };
    let location = match (number(lines, start.row), number(lines, last)) {
        (Some(first), Some(last)) if first != last => {
            format!("{}:{first}-{last}", path.display())
        }
        (Some(first), _) | (None, Some(first)) => format!("{}:{first}", path.display()),
        (None, None) => path.display().to_string(),
    };
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat((longest + 1).max(3));
    let language = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .filter(|ext| ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_default();
    Some(format!("{location}\n{fence}{language}\n{text}\n{fence}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{git::LineKind, text_selection::Point};

    fn rows(texts: &[&str]) -> Vec<DiffLine> {
        texts
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                old: Some(i + 1),
                new: Some(i + 10),
                text: (*text).into(),
                kind: LineKind::Context,
            })
            .collect()
    }

    fn select(anchor: (usize, usize), head: (usize, usize)) -> Selection {
        Selection {
            anchor: Point {
                row: anchor.0,
                byte: anchor.1,
            },
            head: Point {
                row: head.0,
                byte: head.1,
            },
        }
    }

    #[test]
    fn line_uses_new_number_and_falls_back_to_old_for_deleted_rows() {
        let mut lines = rows(&["a", "b"]);
        lines[1].new = None;
        let path = Path::new("/repo/lib/main.dart");
        assert_eq!(line(path, &lines, 0), "/repo/lib/main.dart:10");
        assert_eq!(line(path, &lines, 1), "/repo/lib/main.dart:2");
    }

    #[test]
    fn selection_reports_line_range_and_fenced_text() {
        let lines = rows(&["fn a() {", "  b();", "}"]);
        let path = Path::new("/repo/src/a.rs");
        assert_eq!(
            selection(path, &lines, select((2, 1), (0, 3))).unwrap(),
            "/repo/src/a.rs:10-12\n```rs\na() {\n  b();\n}\n```"
        );
        // Ending at the start of a row does not count that row.
        assert_eq!(
            selection(path, &lines, select((0, 0), (1, 0))).unwrap(),
            "/repo/src/a.rs:10\n```rs\nfn a() {\n```"
        );
        assert_eq!(selection(path, &lines, select((1, 2), (1, 2))), None);
    }

    #[test]
    fn fence_outgrows_backticks_in_the_selection() {
        let lines = rows(&["say ```hi```"]);
        assert_eq!(
            selection(Path::new("/r/README"), &lines, select((0, 0), (0, 12))).unwrap(),
            "/r/README:10\n````\nsay ```hi```\n````"
        );
    }
}
