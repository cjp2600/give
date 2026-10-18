#!/usr/bin/env python3
"""Run the original giv renderer through a Go overlay; never edit ../giv.

Run on macOS: python3 scripts/generate-giv-preview-fixtures.py [../giv]
Only expected row data is saved; terminal padding and ANSI styling are removed.
"""
import json
import pathlib
import subprocess
import sys
import tempfile

project = pathlib.Path(__file__).resolve().parent.parent
original = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else project.parent / "giv").resolve()
cases = [
    ("replacement", "before\nfirst\nadded\nthird\nafter\n", "@@ -2,2 +2,3 @@\n first\n-old\n+added\n+third\n"),
    ("distant hunks", "one\nnew two\nthree\nfour\nfive\nnew six\nlast\n", "@@ -6 +6 @@\n-six\n+new six\n@@ -2 +2 @@\n-two\n+new two\n"),
    ("delete first", "two\nthree\n", "@@ -1,3 +1,2 @@\n-one\n two\n three\n"),
    ("delete last", "one\ntwo\n", "@@ -1,3 +1,2 @@\n one\n two\n-three\n"),
    ("delete all", "", "@@ -1,2 +0,0 @@\n-one\n-two\n"),
    ("insert first", "new\none\ntwo\n", "@@ -1,2 +1,3 @@\n+new\n one\n two\n"),
    ("insert last", "one\ntwo\nnew\n", "@@ -1,2 +1,3 @@\n one\n two\n+new\n"),
    ("no newline", "new", "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"),
    ("tabs crlf", "a\tb\r\nnew\r\n", "@@ -1,2 +1,2 @@\r\n a\tb\r\n-old\r\n+new\r\n"),
    ("current indentation", "    current\nnew\n", "@@ -1,2 +1,2 @@\n current\n-old\n+new\n"),
    ("blank lines", "\nnew\n\n", "@@ -1,3 +1,3 @@\n \n-old\n+new\n \n"),
    ("header looking code", "++ actual code\n", "--- a/code\n+++ b/code\n@@ -1 +1 @@\n--- old code\n+++ actual code\n"),
    ("new file", "one\ntwo\n", "@@ -0,0 +1,2 @@\n+one\n+two\n"),
    ("missing hunk", "one\n", "not a diff"),
    ("out of bounds", "one\n", "@@ -20 +20 @@\n-old\n+new\n"),
]
inputs = [{"name": name, "body": body, "patch": patch, "show": show}
          for name, body, patch in cases for show in [False, True]]
go_test = r'''package tui
import (
    "encoding/json"
    "os"
    "strconv"
    "strings"
    "testing"
    "github.com/charmbracelet/x/ansi"
)
type desktopRow struct { Old *int `json:"old"`; New *int `json:"new"`; Text string `json:"text"`; Kind string `json:"kind"` }
type desktopCase struct { Name string `json:"name"`; Body string `json:"body"`; Patch string `json:"patch"`; Show bool `json:"show"`; OK bool `json:"ok"`; Rows []desktopRow `json:"rows"` }
func desktopNumber(s string) *int { n, err := strconv.Atoi(strings.TrimSpace(s)); if err != nil { return nil }; return &n }
func TestDesktopParityExport(t *testing.T) {
    input, err := os.ReadFile(os.Getenv("GIV_PARITY_INPUT")); if err != nil { t.Fatal(err) }
    var cases []desktopCase
    if err := json.Unmarshal(input, &cases); err != nil { t.Fatal(err) }
    for i := range cases {
        c := &cases[i]
        rendered, ok := renderFullFileWithDiff("sample.txt", []byte(c.Body), c.Patch, 200, fullFileDiffOptions{showDeletions: c.Show, dualGutter: c.Show})
        c.OK = ok
        c.Rows = []desktopRow{}
        if !ok { continue }
        for _, raw := range strings.Split(rendered, "\n") {
            left, code, found := strings.Cut(ansi.Strip(raw), "│"); if !found { continue }
            r := desktopRow{Kind: "Context", Text: strings.TrimRight(code[3:], " ")}
            if strings.Contains(raw, ansiBgDiffAdd) { r.Kind = "Added" }
            if strings.Contains(raw, ansiBgDiffDel) { r.Kind = "Removed" }
            if c.Show { r.Old = desktopNumber(left[:5]); r.New = desktopNumber(left[5:]) } else {
                r.New = desktopNumber(left)
                if r.Kind == "Context" { r.Old = r.New }
            }
            c.Rows = append(c.Rows, r)
        }
    }
    result, err := json.MarshalIndent(cases, "", "  "); if err != nil { t.Fatal(err) }
    if err := os.WriteFile(os.Getenv("GIV_PARITY_OUTPUT"), append(result, '\n'), 0644); err != nil { t.Fatal(err) }
}
'''
output = project / "tests/fixtures/giv-preview.json"
output.parent.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="give-parity-") as tmp:
    tmp = pathlib.Path(tmp)
    (tmp / "desktop_test.go").write_text(go_test)
    (tmp / "input.json").write_text(json.dumps(inputs))
    (tmp / "overlay.json").write_text(json.dumps({"Replace": {
        str(original / "internal/tui/desktop_parity_export_test.go"): str(tmp / "desktop_test.go")
    }}))
    import os
    env = {**os.environ, "GIV_PARITY_INPUT": str(tmp / "input.json"), "GIV_PARITY_OUTPUT": str(output)}
    subprocess.run(["go", "test", "-overlay", str(tmp / "overlay.json"), "./internal/tui", "-run", "^TestDesktopParityExport$", "-count=1"], cwd=original, env=env, check=True)
print(f"Saved {len(inputs)} original giv renderer cases to {output}")
