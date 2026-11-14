use give::theme::SyntaxColor;
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
    assert!(styles[0].iter().any(|s| s.color == SyntaxColor::Keyword));
    assert!(styles[0].iter().any(|s| s.color == SyntaxColor::Comment));
}

#[test]
fn toml_and_json_highlight_configuration_values_and_comments() {
    for (path, source, cases) in [
        (
            "Cargo.toml",
            "[package]\nname = \"Привет\" # comment\nenabled = true\ncount = 42\nwhen = 2026-11-14T12:30:00Z\ntext = \"\"\"first\nsecond # still a string\n\"\"\"\n[[bin]]\npath = 'src/main.rs'",
            vec![
                (1, "name", SyntaxColor::Variable),
                (1, "Привет", SyntaxColor::String),
                (1, "# comment", SyntaxColor::Comment),
                (2, "true", SyntaxColor::Number),
                (3, "42", SyntaxColor::Number),
                (4, "2026", SyntaxColor::Number),
                (6, "# still", SyntaxColor::String),
                (9, "src/main.rs", SyntaxColor::String),
            ],
        ),
        (
            "package.json",
            "{\n  \"name\": \"Привет\",\n  \"enabled\": true,\n  \"count\": 42,\n  \"value\": null\n}",
            vec![
                (1, "Привет", SyntaxColor::String),
                (2, "true", SyntaxColor::Number),
                (3, "42", SyntaxColor::Number),
                (4, "null", SyntaxColor::Number),
            ],
        ),
        (
            "tsconfig.json",
            "{\n // compiler settings\n \"strict\": true\n}",
            vec![
                (1, "compiler", SyntaxColor::Comment),
                (2, "true", SyntaxColor::Number),
            ],
        ),
        (
            "settings.jsonc",
            "{\n /* multiline\n comment */\n \"count\": 42\n}",
            vec![
                (1, "multiline", SyntaxColor::Comment),
                (2, "comment", SyntaxColor::Comment),
                (3, "42", SyntaxColor::Number),
            ],
        ),
    ] {
        let rows = preview::full_context(source.as_bytes());
        let styles = preview::highlight(Path::new(path), &rows);
        for (row, token, expected) in cases {
            let start = rows[row].text.find(token).unwrap();
            let color = styles[row]
                .iter()
                .find(|span| span.range.contains(&start))
                .map(|span| span.color);
            assert_eq!(color, Some(expected), "{path}:{row}: {token}");
        }
        for (line, spans) in rows.iter().zip(&styles) {
            for span in spans {
                assert!(line.text.get(span.range.clone()).is_some());
            }
        }
    }
}

#[test]
fn web_languages_highlight_modern_script_and_component_syntax() {
    let cases = [
        (
            "app.ts",
            "export interface Props { title: string; }",
            "interface",
            SyntaxColor::Keyword,
        ),
        (
            "app.mts",
            "const title: string = 'Привет';",
            "const",
            SyntaxColor::Keyword,
        ),
        (
            "app.cts",
            "export type Count = number;",
            "type",
            SyntaxColor::Keyword,
        ),
        (
            "app.js",
            "export const greet = async () => 'hello';",
            "async",
            SyntaxColor::Keyword,
        ),
        (
            "app.mjs",
            "export const count = 42;",
            "const",
            SyntaxColor::Keyword,
        ),
        ("app.cjs", "const count = 42;", "42", SyntaxColor::Number),
        (
            "App.tsx",
            "const view = <Card title=\"Привет\" />;",
            "Привет",
            SyntaxColor::String,
        ),
        (
            "App.jsx",
            "const view = <Card title=\"Привет\" />;",
            "Привет",
            SyntaxColor::String,
        ),
        (
            "index.astro",
            "<h1 class=\"title\">Привет</h1>",
            "title",
            SyntaxColor::String,
        ),
    ];
    for (path, source, token, expected) in cases {
        let rows = preview::full_context(source.as_bytes());
        let styles = preview::highlight(Path::new(path), &rows);
        let start = source.find(token).unwrap();
        let color = styles[0]
            .iter()
            .find(|span| span.range.contains(&start))
            .map(|span| span.color);
        assert_eq!(color, Some(expected), "{path}: {token}");
        for span in &styles[0] {
            assert!(
                source.get(span.range.clone()).is_some(),
                "{path}: invalid UTF-8 range"
            );
        }
    }
}

#[test]
fn astro_preserves_frontmatter_templates_scripts_and_styles_across_rows() {
    let source = "---\nimport Layout from './Layout.astro';\ninterface Props { title: string; }\nconst greeting = `Привет\nworld ${42}`;\n---\nplain const return\n<Layout title={greeting}>\n<h1 class=\"title\">{true ? greeting : 'fallback'}</h1>\n<script>\nconst enabled = true;\n/* multiline\nreturn is a comment\n*/\nconst count = 12;\n</script>\n<style>\n.title { color: red; }\n</style>\n</Layout>";
    let rows = preview::full_context(source.as_bytes());
    let styles = preview::highlight(Path::new("src/pages/index.astro"), &rows);
    for (row, token, expected) in [
        (1, "import", SyntaxColor::Keyword),
        (2, "interface", SyntaxColor::Keyword),
        (4, "world", SyntaxColor::String),
        (4, "42", SyntaxColor::Number),
        (6, "const", SyntaxColor::Text),
        (8, "title", SyntaxColor::String),
        (8, "true", SyntaxColor::Number),
        (10, "const", SyntaxColor::Keyword),
        (12, "return", SyntaxColor::Comment),
        (14, "12", SyntaxColor::Number),
    ] {
        let start = rows[row].text.find(token).unwrap();
        let color = styles[row]
            .iter()
            .find(|span| span.range.contains(&start))
            .map(|span| span.color);
        assert_eq!(color, Some(expected), "line {row}: {token}");
    }
    assert!(
        styles[17]
            .iter()
            .any(|span| span.color != SyntaxColor::Text),
        "embedded CSS must be highlighted"
    );
}

#[test]
fn web_syntax_cache_keeps_multiline_context_when_scrolling_and_toggling_deletions() {
    use give::{
        git::Diff,
        preview_cache::{Key, PreviewCache},
    };
    let source = format!(
        "/*\n{}\n*/\nconst count = 42;",
        "comment const value\n".repeat(100) + "comment const value"
    );
    let rows = preview::full_context(source.as_bytes());
    let slot = PreviewCache::default().slot(Key {
        path: "app.ts".into(),
        version: 1,
    });
    slot.load_with(|| {
        let diff = Diff {
            lines: rows.clone(),
            ..Diff::default()
        };
        Ok((diff.clone(), diff))
    })
    .unwrap();
    // Start in the middle, then go backwards and forwards across checkpoints.
    for range in [96..103, 48..55, 100..104, 0..4] {
        slot.highlight_range(false, range.clone());
        assert!(slot.range_ready(false, range));
    }
    let syntax = slot.styles(false).unwrap();
    assert!(syntax[20].get().is_none(), "offscreen spans stay lazy");
    let start = rows[100].text.find("const").unwrap();
    assert_eq!(
        syntax[100]
            .get()
            .unwrap()
            .iter()
            .find(|span| span.range.contains(&start))
            .unwrap()
            .color,
        SyntaxColor::Comment
    );
    let last = rows.len() - 1;
    assert!(
        syntax[last]
            .get()
            .unwrap()
            .iter()
            .any(|span| span.color == SyntaxColor::Keyword)
    );

    // A deleted comment opener affects the old stream, but must not recolor the
    // current file's shared rows as comments when deletion mode is opened first.
    let body = b"const current = 42;\n*/\n";
    let normal = preview::overlay(
        body,
        "@@ -1,3 +1,2 @@\n-/*\n const current = 42;\n */\n",
        false,
    )
    .unwrap();
    let deleted = preview::overlay(
        body,
        "@@ -1,3 +1,2 @@\n-/*\n const current = 42;\n */\n",
        true,
    )
    .unwrap();
    let slot = PreviewCache::default().slot(Key {
        path: "removed.ts".into(),
        version: 1,
    });
    slot.load_with(|| {
        Ok((
            Diff {
                lines: normal,
                ..Diff::default()
            },
            Diff {
                lines: deleted,
                ..Diff::default()
            },
        ))
    })
    .unwrap();
    slot.highlight_range(true, 0..3);
    let normal = slot.styles(false).unwrap();
    let deleted = slot.styles(true).unwrap();
    assert!(std::sync::Arc::ptr_eq(&normal[0], &deleted[1]));
    assert_eq!(normal[0].get().unwrap()[0].color, SyntaxColor::Keyword);
    assert!(deleted[0].get().is_some());
}

#[test]
fn syntax_cache_finishes_fallback_headers_without_parsing_them_as_source() {
    use give::{
        git::Diff,
        preview_cache::{Key, PreviewCache},
    };
    let diff = parse_diff(
        "diff --git a/file.ts b/file.ts\n--- a/file.ts\n+++ b/file.ts\n@@ -1 +1 @@\n-const old = 1;\n+const current = 2;\n",
    );
    let normal = preview::fallback(&diff, false);
    let deleted = preview::fallback(&diff, true);
    let slot = PreviewCache::default().slot(Key {
        path: "file.ts".into(),
        version: 1,
    });
    slot.load_with(|| {
        Ok((
            Diff {
                lines: normal,
                ..Diff::default()
            },
            Diff {
                lines: deleted,
                ..Diff::default()
            },
        ))
    })
    .unwrap();
    slot.highlight_range(true, 0..usize::MAX);
    assert!(slot.range_ready(true, 0..usize::MAX));
}

#[test]
fn dart_has_its_own_grammar() {
    let source = "@override Future<void> main() async { final s = 'hi \\$name ${a.b}'; return 0x1F; } // done";
    let rows = preview::full_context(source.as_bytes());
    let styles = preview::highlight(Path::new("lib/main.dart"), &rows);
    let color = |token: &str| {
        let start = source.find(token).unwrap();
        styles[0]
            .iter()
            .find(|span| span.range.contains(&start))
            .unwrap()
            .color
    };
    assert_eq!(color("async"), SyntaxColor::Keyword);
    assert_eq!(color("return"), SyntaxColor::Keyword);
    assert_eq!(color("'hi"), SyntaxColor::String);
    assert_eq!(color("name"), SyntaxColor::String);
    assert_eq!(color("0x1F"), SyntaxColor::Number);
    assert_eq!(color("// done"), SyntaxColor::Comment);
    assert_eq!(color("Future"), SyntaxColor::Function);
    assert_ne!(color("main"), SyntaxColor::Text);
}

#[test]
fn proto3_highlights_declarations_fields_rpcs_and_literals() {
    let cases = [
        (
            "syntax = \"proto3\";",
            vec![
                ("syntax", SyntaxColor::Keyword),
                ("proto3", SyntaxColor::String),
            ],
        ),
        (
            "import public 'common.proto';",
            vec![
                ("import", SyntaxColor::Keyword),
                ("public", SyntaxColor::Keyword),
                ("common.proto", SyntaxColor::String),
            ],
        ),
        (
            "package example.v1;",
            vec![("package", SyntaxColor::Keyword)],
        ),
        (
            "message Request {",
            vec![
                ("message", SyntaxColor::Keyword),
                ("Request", SyntaxColor::Function),
            ],
        ),
        (
            "  optional string name = 1; // имя",
            vec![
                ("optional", SyntaxColor::Keyword),
                ("string", SyntaxColor::Keyword),
                ("1", SyntaxColor::Number),
                ("имя", SyntaxColor::Comment),
            ],
        ),
        (
            "  repeated .example.v1.Item items = 2;",
            vec![
                ("repeated", SyntaxColor::Keyword),
                ("example.v1.Item", SyntaxColor::Function),
                ("2", SyntaxColor::Number),
            ],
        ),
        (
            "  map<string, bytes> labels = 3;",
            vec![
                ("map", SyntaxColor::Keyword),
                ("string", SyntaxColor::Keyword),
                ("bytes", SyntaxColor::Keyword),
                ("3", SyntaxColor::Number),
            ],
        ),
        ("  oneof payload {", vec![("oneof", SyntaxColor::Keyword)]),
        (
            "  reserved 4 to 9;",
            vec![
                ("reserved", SyntaxColor::Keyword),
                ("to", SyntaxColor::Keyword),
                ("9", SyntaxColor::Number),
            ],
        ),
        (
            "enum State { UNKNOWN = 0; }",
            vec![("enum", SyntaxColor::Keyword), ("0", SyntaxColor::Number)],
        ),
        ("service API {", vec![("service", SyntaxColor::Keyword)]),
        (
            "  rpc Watch(stream Request) returns (stream Reply);",
            vec![
                ("rpc", SyntaxColor::Keyword),
                ("Watch", SyntaxColor::Type),
                ("stream", SyntaxColor::Keyword),
                ("returns", SyntaxColor::Keyword),
            ],
        ),
        (
            "option deprecated = true;",
            vec![
                ("option", SyntaxColor::Keyword),
                ("true", SyntaxColor::Number),
            ],
        ),
        (
            "option (custom.ratio) = -1.5e+2;",
            vec![("1.5e+2", SyntaxColor::Number)],
        ),
        (
            "/* message hidden = 42; */ int32 count = 5;",
            vec![
                ("hidden", SyntaxColor::Comment),
                ("42", SyntaxColor::Comment),
                ("int32", SyntaxColor::Keyword),
                ("5;", SyntaxColor::Number),
            ],
        ),
        (
            r#"option (custom.text) = "say \"hi\" // message";"#,
            vec![
                ("say", SyntaxColor::String),
                ("// message", SyntaxColor::String),
            ],
        ),
    ];
    let source = cases
        .iter()
        .map(|(line, _)| *line)
        .collect::<Vec<_>>()
        .join("\n");
    let rows = preview::full_context(source.as_bytes());
    let styles = preview::highlight(Path::new("api/service.proto"), &rows);
    assert_eq!(styles.len(), cases.len());
    for ((line, tokens), spans) in cases.iter().zip(&styles) {
        for span in spans {
            assert!(line.get(span.range.clone()).is_some(), "{line}");
        }
        for (token, expected) in tokens {
            let start = line.find(token).unwrap();
            let span = spans
                .iter()
                .find(|span| span.range.contains(&start))
                .unwrap();
            assert_eq!(span.color, *expected, "{token} in {line}");
        }
    }
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

#[test]
fn catalog_lists_every_file_in_one_tree_with_changes_kept() {
    let files =
        give::git::parse_status(b" M src/lib.rs\0A  src/new.rs\0D  gone.txt\0?? notes.md\0")
            .unwrap();
    let catalog = give::git::merge_catalog(
        &files,
        b"Cargo.toml\0src/lib.rs\0src/new.rs\0src/ui/view.rs\0notes.md\0",
    )
    .unwrap();
    assert_eq!(catalog.len(), 6);
    assert!(
        catalog
            .iter()
            .any(|f| f.path == Path::new("gone.txt") && f.index == 'D')
    );
    assert!(
        catalog
            .iter()
            .any(|f| f.path == Path::new("Cargo.toml") && f.clean())
    );
    assert!(
        catalog
            .iter()
            .any(|f| f.path == Path::new("notes.md") && f.untracked())
    );

    let rows = tree::build_catalog(&catalog, &HashSet::new(), false);
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        ["src", "Cargo.toml", "gone.txt", "notes.md"]
    );
    assert!(rows[0].changed);
    assert!(!rows[1].changed);

    let expanded = tree::changed_directories(&catalog);
    assert_eq!(expanded, HashSet::from(["src".into()]));
    let rows = tree::build_catalog(&catalog, &expanded, false);
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        [
            "src",
            "ui",
            "lib.rs",
            "new.rs",
            "Cargo.toml",
            "gone.txt",
            "notes.md"
        ]
    );
    assert!(!rows[1].changed);
    let rows = tree::build_catalog(&catalog, &HashSet::new(), true);
    assert_eq!(rows.len(), 8);
}
