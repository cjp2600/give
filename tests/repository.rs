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
