//! Literal Unicode search shared by source previews and terminal history.
use crate::git::DiffLine;
use regex::Regex;
use std::ops::Range;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
}

impl Query {
    pub fn pattern(&self) -> String {
        let literal = regex::escape(&self.text);
        if self.case_sensitive {
            format!("(?-i:{literal})")
        } else {
            format!("(?i:{literal})")
        }
    }

    pub fn source_matches(&self, lines: &[DiffLine]) -> Vec<SourceMatch> {
        if self.text.is_empty() {
            return Vec::new();
        }
        let Ok(regex) = Regex::new(&self.pattern()) else {
            return Vec::new();
        };
        lines
            .iter()
            .enumerate()
            .flat_map(|(row, line)| {
                regex.find_iter(&line.text).map(move |m| SourceMatch {
                    row,
                    range: m.range(),
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMatch {
    pub row: usize,
    pub range: Range<usize>,
}

pub fn next_index(current: Option<usize>, count: usize, backwards: bool) -> Option<usize> {
    if count == 0 {
        None
    } else {
        Some(match current {
            Some(index) if backwards => (index + count - 1) % count,
            Some(index) => (index + 1) % count,
            None if backwards => count - 1,
            None => 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::full_context;

    #[test]
    fn literal_search_preserves_unicode_byte_ranges_and_case_choice() {
        let lines = full_context("🙂ПрИвЕт привет\na.b axb a.b".as_bytes());
        let mut query = Query {
            text: "привет".into(),
            case_sensitive: false,
        };
        let matches = query.source_matches(&lines);
        assert_eq!(matches.len(), 2);
        assert_eq!(&lines[0].text[matches[0].range.clone()], "ПрИвЕт");
        query.case_sensitive = true;
        assert_eq!(query.source_matches(&lines).len(), 1);
        query.text = "a.b".into();
        assert_eq!(query.source_matches(&lines).len(), 2);
        query.text.clear();
        assert!(query.source_matches(&lines).is_empty());
    }

    #[test]
    fn navigation_wraps_in_both_directions_and_handles_no_results() {
        assert_eq!(next_index(Some(2), 3, false), Some(0));
        assert_eq!(next_index(Some(0), 3, true), Some(2));
        assert_eq!(next_index(None, 3, true), Some(2));
        assert_eq!(next_index(Some(0), 0, false), None);
    }
}
