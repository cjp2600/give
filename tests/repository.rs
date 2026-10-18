use give::git::{Layer, LineKind, Repository, parse_diff, parse_status};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    repo: Repository,
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--initial-branch=main"]);
        git(dir.path(), &["config", "user.name", "Give tests"]);
        git(
            dir.path(),
            &["config", "user.email", "give@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        git(dir.path(), &["config", "core.hooksPath", "/dev/null"]);
        let repo = Repository::open(dir.path()).unwrap();
        Self { dir, repo }
    }

    fn write(&self, path: &str, text: &str) {
        fs::write(self.dir.path().join(path), text).unwrap();
    }

    fn initial_commit(&self) {
        self.write("file.txt", "one\ntwo\nthree\n");
        self.repo.stage(None).unwrap();
        self.repo.commit("Initial").unwrap();
    }
}

#[test]
fn staged_and_working_previews_are_independent_and_commit_uses_only_index() {
    let f = Fixture::new();
    f.initial_commit();
    f.write("file.txt", "one\nstaged\nthree\n");
    f.repo.stage(None).unwrap();
    f.write("file.txt", "one\nworking\nthree\n");
    let snapshot = f.repo.snapshot().unwrap();
    let file = &snapshot.files[0];
    assert!(file.staged() && file.unstaged());
    assert!(
        f.repo
            .diff(file, Layer::Staged)
            .unwrap()
            .raw
            .contains("+staged")
    );
    assert!(
        f.repo
            .diff(file, Layer::Working)
            .unwrap()
            .raw
            .contains("+working")
    );
    f.repo.commit("Commit staged only").unwrap();
    assert!(git(f.dir.path(), &["show", "HEAD:file.txt"]).contains("staged"));
    assert!(
        fs::read_to_string(f.dir.path().join("file.txt"))
            .unwrap()
            .contains("working")
    );
    let snapshot = f.repo.snapshot().unwrap();
    assert!(!snapshot.files[0].staged());
    assert!(snapshot.files[0].unstaged());
}

#[test]
fn unborn_repository_can_stage_unstage_and_commit_without_removing_files() {
    let f = Fixture::new();
    f.write("new.txt", "hello\n");
    assert_eq!(f.repo.snapshot().unwrap().branch, "main");
    f.repo.stage(None).unwrap();
    f.repo.unstage(None).unwrap();
    assert!(f.dir.path().join("new.txt").exists());
    assert!(f.repo.snapshot().unwrap().files[0].untracked());
    f.repo.stage(None).unwrap();
    f.repo.commit("Первая сборка 🛠️").unwrap();
    assert!(f.repo.snapshot().unwrap().files.is_empty());
}

#[test]
fn null_delimited_status_preserves_whitespace_unicode_and_rename_source() {
    let files =
        parse_status(b" M space name.txt\0?? line\nbreak.txt\0R  new -> name.txt\0old.txt\0")
            .unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().any(|f| f.path == Path::new("line\nbreak.txt")));
    let renamed = files.iter().find(|f| f.index == 'R').unwrap();
    assert_eq!(renamed.path, PathBuf::from("new -> name.txt"));
    assert_eq!(renamed.original_path, Some("old.txt".into()));
    assert!(parse_status(b"R  missing-source\0").is_err());
}

#[test]
fn literal_pathspecs_and_unicode_paths_stage_only_the_selected_file() {
    let f = Fixture::new();
    f.write(":(glob)*.txt", "special\n");
    f.write("привет мир.txt", "unicode\n");
    let snapshot = f.repo.snapshot().unwrap();
    let special = snapshot
        .files
        .iter()
        .find(|file| file.path == Path::new(":(glob)*.txt"))
        .unwrap();
    f.repo.stage(Some(special)).unwrap();
    let snapshot = f.repo.snapshot().unwrap();
    assert_eq!(
        snapshot.files.iter().filter(|file| file.staged()).count(),
        1
    );
    assert!(
        snapshot
            .files
            .iter()
            .any(|file| file.path == Path::new("привет мир.txt") && file.untracked())
    );
}

#[test]
fn rename_can_be_unstaged_without_changing_worktree() {
    let f = Fixture::new();
    f.initial_commit();
    fs::rename(
        f.dir.path().join("file.txt"),
        f.dir.path().join("renamed.txt"),
    )
    .unwrap();
    f.repo.stage(None).unwrap();
    let snapshot = f.repo.snapshot().unwrap();
    assert_eq!(snapshot.files[0].index, 'R');
    f.repo.unstage(Some(&snapshot.files[0])).unwrap();
    assert!(f.dir.path().join("renamed.txt").exists());
    assert!(!f.dir.path().join("file.txt").exists());
    assert!(
        f.repo
            .snapshot()
            .unwrap()
            .files
            .iter()
            .all(|file| !file.staged())
    );
}

#[test]
fn restaging_an_edited_rename_works() {
    let f = Fixture::new();
    f.initial_commit();
    fs::rename(
        f.dir.path().join("file.txt"),
        f.dir.path().join("renamed.txt"),
    )
    .unwrap();
    f.repo.stage(None).unwrap();
    f.write("renamed.txt", "one\ntwo\nchanged\n");
    let snapshot = f.repo.snapshot().unwrap();
    f.repo.stage(Some(&snapshot.files[0])).unwrap();
    assert!(
        f.repo
            .snapshot()
            .unwrap()
            .files
            .iter()
            .all(|file| !file.unstaged())
    );
}

#[test]
fn binary_large_and_empty_untracked_files_have_explicit_previews() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("binary.dat"), [1, 0, 2]).unwrap();
    fs::write(f.dir.path().join("large.dat"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    f.write("empty.txt", "");
    let snapshot = f.repo.snapshot().unwrap();
    for file in snapshot.files {
        assert!(f.repo.diff(&file, Layer::Working).unwrap().notice.is_some());
    }
}

#[test]
fn hunk_line_numbers_and_header_looking_content_are_correct() {
    let diff =
        parse_diff("--- a/file\n+++ b/file\n@@ -3,2 +3,2 @@\n old\n-gone\n+++ actual code\n");
    let added = diff
        .lines
        .iter()
        .find(|line| line.kind == LineKind::Added)
        .unwrap();
    assert_eq!(added.new, Some(4));
    assert_eq!(diff.additions, 1);
    assert_eq!(diff.deletions, 1);
    let f = Fixture::new();
    f.write("file.txt", "++ actual code\n");
    let file = &f.repo.snapshot().unwrap().files[0];
    assert_eq!(f.repo.diff(file, Layer::Working).unwrap().additions, 1);
}

#[test]
fn commit_rejects_empty_message_and_empty_index() {
    let f = Fixture::new();
    assert!(f.repo.commit(" ").is_err());
    assert!(f.repo.commit("Nothing staged").is_err());
}

#[test]
fn live_preview_includes_staged_and_unstaged_changes_over_full_worktree() {
    let f = Fixture::new();
    let original = (1..=30).map(|i| format!("line {i}\n")).collect::<String>();
    f.write("file.txt", &original);
    f.repo.stage(None).unwrap();
    f.repo.commit("Initial").unwrap();
    let staged = original.replace("line 5\n", "staged five\n");
    f.write("file.txt", &staged);
    f.repo.stage(None).unwrap();
    f.write(
        "file.txt",
        &staged.replace("line 25\n", "working twenty five\n"),
    );
    let file = &f.repo.snapshot().unwrap().files[0];
    let preview = f.repo.preview(file, false).unwrap();
    assert_eq!(preview.lines.len(), 30);
    assert_eq!(preview.lines[0].text, "line 1");
    assert_eq!(preview.lines[14].text, "line 15");
    assert_eq!(preview.lines[29].text, "line 30");
    assert_eq!(preview.lines[4].kind, LineKind::Added);
    assert_eq!(preview.lines[24].kind, LineKind::Added);
    assert!(!preview.lines.iter().any(|l| l.kind == LineKind::Removed
        || l.kind == LineKind::Hunk
        || l.kind == LineKind::Header));
    let preview = f.repo.preview(file, true).unwrap();
    assert_eq!(preview.lines.len(), 32);
    assert_eq!(
        preview
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Removed)
            .count(),
        2
    );
}

#[test]
fn live_preview_ignores_whitespace_noise_and_new_files_are_plain_full_context() {
    let f = Fixture::new();
    f.initial_commit();
    f.write("file.txt", "  one\ntwo  \nthree\n");
    f.write("new.rs", "fn main() {}\n");
    for file in &f.repo.snapshot().unwrap().files {
        let preview = f.repo.preview(file, false).unwrap();
        assert!(preview.lines.iter().all(|l| l.kind == LineKind::Context));
        assert!(preview.notice.is_some());
    }
    f.repo.stage(None).unwrap();
    let file = f
        .repo
        .snapshot()
        .unwrap()
        .files
        .into_iter()
        .find(|f| f.path == Path::new("new.rs"))
        .unwrap();
    assert_eq!(
        f.repo.preview(&file, false).unwrap().lines[0].kind,
        LineKind::Context
    );
}

#[test]
fn live_preview_forces_source_text_despite_gitattributes_and_falls_back_for_deletion() {
    let f = Fixture::new();
    f.write(".gitattributes", "*.rs -diff\n");
    f.write("file.rs", "fn before() {}\n");
    f.repo.stage(None).unwrap();
    f.repo.commit("Initial").unwrap();
    f.write("file.rs", "fn after() {}\n");
    let file = f
        .repo
        .snapshot()
        .unwrap()
        .files
        .into_iter()
        .find(|f| f.path == Path::new("file.rs"))
        .unwrap();
    let preview = f.repo.preview(&file, false).unwrap();
    assert_eq!(preview.lines.len(), 1);
    assert_eq!(preview.lines[0].kind, LineKind::Added);
    fs::remove_file(f.dir.path().join("file.rs")).unwrap();
    let file = f
        .repo
        .snapshot()
        .unwrap()
        .files
        .into_iter()
        .find(|f| f.path == Path::new("file.rs"))
        .unwrap();
    assert!(
        !f.repo
            .preview(&file, false)
            .unwrap()
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Removed)
    );
    assert!(
        f.repo
            .preview(&file, true)
            .unwrap()
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Removed)
    );
}

#[test]
fn snapshot_versions_detect_same_status_edits_without_invalidating_other_files() {
    use std::time::{Duration, UNIX_EPOCH};
    let f = Fixture::new();
    f.initial_commit();
    f.write("file.txt", "new one\n");
    f.write("other.txt", "unchanged\n");
    let path = f.dir.path().join("file.txt");
    fs::File::open(&path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(100))
        .unwrap();
    let first = f.repo.snapshot().unwrap();
    f.write("file.txt", "new two\n");
    fs::File::open(&path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(101))
        .unwrap();
    let second = f.repo.snapshot().unwrap();
    assert_eq!(first.files, second.files);
    assert_ne!(
        first.versions[Path::new("file.txt")],
        second.versions[Path::new("file.txt")]
    );
    assert_eq!(
        first.versions[Path::new("other.txt")],
        second.versions[Path::new("other.txt")]
    );
}

#[test]
fn preview_cache_coalesces_reads_and_shares_both_modes_without_copying_rows() {
    use give::preview_cache::{Key, PreviewCache};
    use std::sync::Arc;
    let f = Fixture::new();
    f.initial_commit();
    f.write("file.txt", "one\nchanged\nthree\n");
    let snapshot = f.repo.snapshot().unwrap();
    let file = snapshot.files[0].clone();
    let key = Key {
        path: file.path.clone(),
        version: snapshot.versions[&file.path],
    };
    let mut cache = PreviewCache::default();
    let slot = cache.slot(key.clone());
    assert!(slot.ready().is_none());
    let concurrent = cache.slot(key.clone());
    assert!(Arc::ptr_eq(&slot, &concurrent));
    let (first, second) = std::thread::scope(|scope| {
        let other = scope.spawn(|| concurrent.load(&f.repo, &file).unwrap());
        let first = slot.load(&f.repo, &file).unwrap();
        (first, other.join().unwrap())
    });
    assert!(Arc::ptr_eq(&first.normal, &second.normal));
    assert_eq!(first.normal.lines.len(), 3);
    assert_eq!(first.deletions.lines.len(), 4);
    // A cache hit needs neither a readable file nor a working Git subprocess.
    fs::remove_file(f.dir.path().join(&file.path)).unwrap();
    let repeated = slot.load(&f.repo, &file).unwrap();
    assert!(Arc::ptr_eq(&first.normal, &repeated.normal));
    slot.highlight();
    assert!(slot.styles(false).is_some());
    assert!(slot.styles(true).is_some());
    assert!(cache.bytes() > 0);
    let changed = cache.slot(Key {
        path: file.path.clone(),
        version: key.version + 1,
    });
    assert!(changed.ready().is_none());
    assert_eq!(cache.len(), 1);
    cache.retain_versions(&Default::default());
    assert!(cache.is_empty());
}

#[test]
fn preview_cache_enforces_lru_capacity_and_memory_budget() {
    use give::preview_cache::{Key, PreviewCache};
    use std::sync::Arc;
    let mut cache = PreviewCache::new(2, usize::MAX);
    let key = |name: &str| Key {
        path: name.into(),
        version: 1,
    };
    let first = cache.slot(key("first"));
    let second = cache.slot(key("second"));
    assert!(Arc::ptr_eq(&first, &cache.slot(key("first"))));
    cache.slot(key("third"));
    assert_eq!(cache.len(), 2);
    assert!(!Arc::ptr_eq(&second, &cache.slot(key("second"))));
    let f = Fixture::new();
    f.write("file.txt", "line\n");
    let file = f.repo.snapshot().unwrap().files[0].clone();
    let mut cache = PreviewCache::new(12, 1);
    cache.slot(key("empty"));
    let loaded = cache.slot(key("file.txt"));
    loaded.load(&f.repo, &file).unwrap();
    cache.trim();
    assert_eq!(cache.len(), 1); // Always retain the current file, even above budget.
}

#[test]
fn syntax_is_lazy_and_unchanged_rows_are_shared_between_modes() {
    use give::preview_cache::{Key, PreviewCache};
    use std::sync::Arc;
    let f = Fixture::new();
    f.write("file.rs", "let value = 1;\n".repeat(100).as_str());
    f.repo.stage(None).unwrap();
    f.repo.commit("Initial").unwrap();
    f.write(
        "file.rs",
        "let value = 1;\n"
            .repeat(100)
            .replacen("= 1", "= 2", 1)
            .as_str(),
    );
    let snapshot = f.repo.snapshot().unwrap();
    let file = &snapshot.files[0];
    let slot = PreviewCache::default().slot(Key {
        path: file.path.clone(),
        version: snapshot.versions[&file.path],
    });
    slot.load(&f.repo, file).unwrap();
    let normal = slot.styles(false).unwrap();
    let deleted = slot.styles(true).unwrap();
    assert!(normal.iter().all(|row| row.get().is_none()));
    slot.highlight_range(false, 0..4);
    assert!(slot.range_ready(false, 0..4));
    assert!(!slot.range_ready(false, 4..8));
    assert!(normal[99].get().is_none());
    assert!(Arc::ptr_eq(&normal[1], &deleted[2]));
    assert!(deleted[2].get().is_some());
    assert!(deleted[0].get().is_none()); // Deleted old text has its own lazy cell.
}
