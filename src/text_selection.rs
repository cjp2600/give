//! Selection in the displayed source buffer. Offsets are UTF-8 byte boundaries;
//! gutters and diff decorations are deliberately outside this buffer.
use crate::git::DiffLine;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: usize,
    pub byte: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Point,
    pub head: Point,
}

impl Selection {
    pub fn new(point: Point) -> Self {
        Self {
            anchor: point,
            head: point,
        }
    }
    pub fn ordered(&self) -> (Point, Point) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
    pub fn range(&self, row: usize, text: &str) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if row < start.row || row > end.row {
            return None;
        }
        let start = if row == start.row {
            boundary(text, start.byte)
        } else {
            0
        };
        let end = if row == end.row {
            boundary(text, end.byte)
        } else {
            text.len()
        };
        Some(start.min(end)..end)
    }
    pub fn text(&self, lines: &[DiffLine]) -> String {
        if self.anchor == self.head || lines.is_empty() {
            return String::new();
        }
        let (start, end) = self.ordered();
        lines
            .iter()
            .enumerate()
            .skip(start.row)
            .take(end.row.saturating_sub(start.row) + 1)
            .map(|(row, line)| {
                self.range(row, &line.text)
                    .map(|r| line.text[r].to_owned())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
pub fn boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}
pub fn word(text: &str, byte: usize) -> Range<usize> {
    let byte = boundary(text, byte);
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let Some(character) = text[byte..].chars().next() else {
        return byte..byte;
    };
    let mut start = byte;
    for (index, c) in text[..byte].char_indices().rev() {
        if class(c) != class(character) {
            break;
        }
        start = index;
    }
    let mut end = byte;
    for (index, c) in text[byte..].char_indices() {
        if class(c) != class(character) {
            break;
        }
        end = byte + index + c.len_utf8();
    }
    start..end
}
pub fn step(mut point: Point, lines: &[DiffLine], vertical: isize, horizontal: isize) -> Point {
    if lines.is_empty() {
        return Point::default();
    }
    point.row = point.row.min(lines.len() - 1);
    if vertical != 0 {
        point.row = point
            .row
            .saturating_add_signed(vertical)
            .min(lines.len() - 1);
    }
    point.byte = boundary(&lines[point.row].text, point.byte);
    if horizontal < 0 {
        if point.byte > 0 {
            point.byte = lines[point.row].text[..point.byte]
                .char_indices()
                .last()
                .map_or(0, |(i, _)| i);
        } else if point.row > 0 {
            point.row -= 1;
            point.byte = lines[point.row].text.len();
        }
    } else if horizontal > 0 {
        if point.byte < lines[point.row].text.len() {
            point.byte += lines[point.row].text[point.byte..]
                .chars()
                .next()
                .unwrap()
                .len_utf8();
        } else if point.row + 1 < lines.len() {
            point.row += 1;
            point.byte = 0;
        }
    }
    point
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::full_context;
    #[test]
    fn reverse_multiline_copy_excludes_gutters_and_preserves_whitespace() {
        let lines = full_context("  первый\n\tsecond\nlast".as_bytes());
        let selection = Selection {
            anchor: Point { row: 2, byte: 2 },
            head: Point { row: 0, byte: 2 },
        };
        assert_eq!(selection.text(&lines), "первый\n        second\nla");
        assert_eq!(selection.range(1, &lines[1].text), Some(0..14));
    }
    #[test]
    fn unicode_edges_words_and_keyboard_selection_stay_at_character_boundaries() {
        let lines = full_context("a🙂б\nnext".as_bytes());
        let mut point = Point { row: 0, byte: 1 };
        point = step(point, &lines, 0, 1);
        assert_eq!(point.byte, 5);
        point = step(point, &lines, 0, 1);
        assert_eq!(point.byte, 7);
        point = step(point, &lines, 0, 1);
        assert_eq!(point, Point { row: 1, byte: 0 });
        assert_eq!(word("  hello_мир()", 4), 2..14);
        assert_eq!(boundary("a🙂б", 3), 1);
    }
}
