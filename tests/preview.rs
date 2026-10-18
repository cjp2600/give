use give::{
    git::{LineKind, parse_diff},
    preview::{self, MAX_LINES},
    tree::{self, RowKind},
};
use std::{collections::HashSet, path::Path};

#[test]
fn overlay_keeps_entire_current_file_and_only_marks_additions_by_default() {
    let body = b"before\nfirst\nadded\nthird\nafter\n";
    let patch = "@@ -2,2 +2,3 @@\n first\n-old\n+added\n+third\n";
    let rows = preview::overlay(body, patch, false).unwrap();
    assert_eq!(
        rows.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["before", "first", "added", "third", "after"]
    );
    assert_eq!(
        rows.iter().map(|l| l.kind).collect::<Vec<_>>(),
        [
            LineKind::Context,
            LineKind::Context,
            LineKind::Added,
            LineKind::Added,
            LineKind::Context
        ]
    );
    assert_eq!(
        rows.iter().map(|l| l.new).collect::<Vec<_>>(),
        (1..=5).map(Some).collect::<Vec<_>>()
    );
    assert_eq!(preview::change_blocks(&rows), [2]);
    let rows = preview::overlay(body, patch, true).unwrap();
    assert_eq!(rows[2].kind, LineKind::Removed);
    assert_eq!(rows[2].text, "old");
    assert_eq!(rows[2].old, Some(3));
    assert_eq!(rows[2].new, None);
    assert_eq!(rows.last().unwrap().text, "after");
}

#[test]
fn separated_hunks_keep_all_intervening_lines_and_block_jumps() {
    let body = b"first\nnew second\nthird\nfourth\nfifth\nnew sixth\nlast\n";
    let patch = "@@ -6 +6 @@\n-sixth\n+new sixth\n@@ -2 +2 @@\n-second\n+new second\n";
    let rows = preview::overlay(body, patch, false).unwrap();
    assert_eq!(rows.len(), 7);
    assert_eq!(rows[3].text, "fourth");
    assert_eq!(rows[6].text, "last");
    assert_eq!(preview::change_blocks(&rows), [1, 5]);
    assert_eq!(
        preview::change_blocks(
            &preview::overlay(b"new\n", "@@ -1 +1 @@\n-old\n+new\n", false).unwrap()
        ),
        [0]
    );
}

#[test]
fn overlay_uses_current_file_text_instead_of_patch_context() {
    // git diff -w may preserve a different indentation in patch context.
    let rows = preview::overlay(
        b"    current\nnew\n",
        "@@ -1,2 +1,2 @@\n current\n-old\n+new\n",
        false,
    )
    .unwrap();
    assert_eq!(rows[0].text, "    current");
}

#[test]
fn newline_bom_and_tab_rules_match_giv_without_ghost_rows() {
    assert!(preview::source_lines(b"").is_empty());
    assert_eq!(preview::source_lines(b"\xef\xbb\xbfhi\r\n\r\n"), ["hi", ""]);
    assert_eq!(preview::source_lines(b"one\ntwo"), ["one", "two"]);
    assert_eq!(preview::expand_tabs("a\tb\t"), "a       b       ");
    assert_eq!(preview::expand_tabs("пр\t🙂\t"), "пр      🙂       ");
    let rows = preview::overlay(
        b"new",
        "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        false,
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "new");
}

#[test]
fn deletions_at_start_middle_and_end_preserve_current_content() {
    for (body, patch, expected) in [
        (
            "two\nthree\n",
            "@@ -1,3 +1,2 @@\n-one\n two\n three\n",
            vec!["two", "three"],
        ),
        (
            "one\nthree\n",
            "@@ -1,3 +1,2 @@\n one\n-two\n three\n",
            vec!["one", "three"],
        ),
        (
            "one\ntwo\n",
            "@@ -1,3 +1,2 @@\n one\n two\n-three\n",
            vec!["one", "two"],
        ),
        ("", "@@ -1,2 +0,0 @@\n-one\n-two\n", vec![]),
    ] {
        let rows = preview::overlay(body.as_bytes(), patch, false).unwrap();
        assert_eq!(
            rows.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            expected
        );
        let with_deleted = preview::overlay(body.as_bytes(), patch, true).unwrap();
        assert!(with_deleted.iter().any(|l| l.kind == LineKind::Removed));
    }
}

#[test]
fn invalid_overlay_falls_back_to_patch_and_both_paths_are_bounded() {
    assert!(preview::overlay(b"one\n", "not a patch", false).is_none());
    assert!(preview::overlay(b"one\n", "@@ -20 +20 @@\n-old\n+new\n", false).is_none());
    let patch = parse_diff("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+\tnew\n");
    let rows = preview::fallback(&patch, false);
    assert!(
        rows.iter()
            .all(|l| l.kind != LineKind::Hunk && l.kind != LineKind::Removed)
    );
    assert_eq!(rows.last().unwrap().text, "        new");
    let body = "line\n".repeat(MAX_LINES + 50);
    assert_eq!(preview::full_context(body.as_bytes()).len(), MAX_LINES);
    assert_eq!(
        preview::overlay(body.as_bytes(), "@@ -1 +1 @@\n-old\n+line\n", false)
            .unwrap()
            .len(),
        MAX_LINES
    );
}

#[test]
fn binary_rules_prefer_known_source_extensions_and_match_notice_whole_lines() {
    assert!(preview::known_text(Path::new("generated.PROTO")));
    assert!(!preview::known_text(Path::new("unknown")));
    assert!(preview::binary_path(Path::new("photo.JPEG")));
    assert!(!preview::binary_path(Path::new("icon.svg")));
    assert!(preview::binary_content(b"%PDFrest"));
    assert!(preview::binary_content(b"abc\0def"));
    let mut body = vec![b'x'; 8100];
    body[8050] = 0;
    assert!(!preview::binary_content(&body));
    assert!(preview::binary_notice(
        "Binary files a/image and b/image differ\n"
    ));
    assert!(!preview::binary_notice("+Binary files something differ\n"));
}

#[test]
fn tree_splits_sections_sorts_directories_first_and_compacts_only_directories() {
    let files = give::git::parse_status(
        b" M z.txt\0 M src/deep/a.rs\0 M src/deep/b.rs\0?? src/new.txt\0?? alpha.txt\0",
    )
    .unwrap();
    let rows = tree::build_tree(&files, &HashSet::new());
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        [
            "TRACKED",
            "src/deep",
            "a.rs",
            "b.rs",
            "z.txt",
            "",
            "UNTRACKED",
            "src",
            "new.txt",
            "alpha.txt"
        ]
    );
    assert_eq!(rows[1].path(), Some(Path::new("src/deep")));
    assert_eq!(rows[2].depth, 1);
    assert!(matches!(rows[5].kind, RowKind::Divider));
    let collapsed = HashSet::from(["src/deep".into(), "src".into()]);
    let rows = tree::build_tree(&files, &collapsed);
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        [
            "TRACKED",
            "src/deep",
            "z.txt",
            "",
            "UNTRACKED",
            "src",
            "alpha.txt"
        ]
    );
    assert!(matches!(
        rows[1].kind,
        RowKind::Directory {
            expanded: false,
            ..
        }
    ));
}

#[test]
fn tree_preserves_unusual_filenames_and_prunes_only_missing_directories() {
    let files = give::git::parse_status(b" M parent/ child /one\ntwo.rs\0 M root.txt\0").unwrap();
    let mut collapsed = HashSet::from(["parent/ child ".into(), "gone".into()]);
    tree::prune_collapsed(&mut collapsed, &files);
    assert_eq!(collapsed.len(), 1);
    let rows = tree::build_tree(&files, &HashSet::new());
    assert_eq!(rows[1].label, "parent/ child ");
    assert_eq!(rows[2].label, "one\ntwo.rs");
    assert!(tree::build_tree(&[], &collapsed).is_empty());
}

#[test]
fn syntax_highlights_unicode_with_valid_byte_ranges_and_distinct_colors() {
    let rows = preview::full_context("fn main() { let привет = 42; } // comment".as_bytes());
    let styles = preview::highlight(Path::new("main.rs"), &rows);
    assert_eq!(styles.len(), 1);
    assert!(!styles[0].is_empty());
    for span in &styles[0] {
        assert!(rows[0].text.get(span.range.clone()).is_some());
    }
    assert!(styles[0].iter().any(|s| s.color == 0xb49adb));
    assert!(styles[0].iter().any(|s| s.color == 0x74879c));
}

#[test]
fn overlay_matches_thirty_cases_exported_from_original_go_renderer() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/giv-preview.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = format!("{} (deletions={})", case["name"], case["show"]);
        let rows = preview::overlay(
            case["body"].as_str().unwrap().as_bytes(),
            case["patch"].as_str().unwrap(),
            case["show"].as_bool().unwrap(),
        );
        assert_eq!(rows.is_some(), case["ok"].as_bool().unwrap(), "{name}");
        let Some(rows) = rows else {
            continue;
        };
        let actual: Vec<_> = rows.iter().map(|row| serde_json::json!({
            "old": row.old, "new": row.new, "text": row.text.trim_end_matches(' '), "kind": format!("{:?}", row.kind)
        })).collect();
        assert_eq!(serde_json::json!(actual), case["rows"], "{name}");
    }
}
