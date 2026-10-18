use give::{
    git::{LineKind, Repository},
    review::{self, Forge, Review},
};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

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

fn init(path: &Path) {
    git(path, &["init", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Give tests"]);
    git(path, &["config", "user.email", "give@example.invalid"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    git(path, &["config", "core.hooksPath", "/dev/null"]);
}

fn commit(path: &Path, message: &str) {
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "-m", message]);
}

/// main: file.txt, old.txt. feature edits file.txt, adds new.rs, deletes old.txt.
/// main then moves on with main-only.txt, which the review must not include.
fn branch_fixture() -> (TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init(root);
    fs::write(root.join("file.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(root.join("old.txt"), "gone\n").unwrap();
    commit(root, "Initial");
    git(root, &["checkout", "--quiet", "-b", "feature"]);
    fs::write(root.join("file.txt"), "one\nTWO\nthree\n").unwrap();
    fs::write(root.join("new.rs"), "fn main() {}\n// done\n").unwrap();
    fs::remove_file(root.join("old.txt")).unwrap();
    commit(root, "Feature");
    git(root, &["checkout", "--quiet", "main"]);
    fs::write(root.join("main-only.txt"), "later\n").unwrap();
    commit(root, "Main moves on");
    let repo = Repository::open(root).unwrap();
    (dir, repo)
}

#[test]
fn review_lists_branch_changes_since_the_merge_base_with_main() {
    let (_dir, repo) = branch_fixture();
    let review = Review::load(&repo, "feature").unwrap();
    assert_eq!(review.main, "main");
    assert_eq!(review.forge, Forge::Unknown);
    assert_eq!(
        review.integration_error.as_deref(),
        Some("origin remote is not configured")
    );
    assert_eq!(review.status(), "origin remote is not configured");
    assert_eq!(
        review.base,
        git(&repo.root, &["merge-base", "main", "feature"]).trim()
    );
    assert_eq!(review.head, "feature");
    let files: Vec<_> = review
        .files
        .iter()
        .map(|file| (file.path.to_string_lossy().into_owned(), file.index))
        .collect();
    assert_eq!(
        files,
        [
            ("file.txt".to_owned(), 'M'),
            ("new.rs".to_owned(), 'A'),
            ("old.txt".to_owned(), 'D'),
        ]
    );
    let snapshot = review.snapshot();
    assert_eq!(snapshot.branch, "feature");
    assert_eq!(snapshot.versions.len(), 3);
}

#[test]
fn review_previews_read_the_branch_not_the_working_tree() {
    let (dir, repo) = branch_fixture();
    // Local edits on the checked-out main branch must not leak into the review.
    fs::write(dir.path().join("file.txt"), "dirty\n").unwrap();
    let review = Review::load(&repo, "feature").unwrap();
    let file = |name: &str| {
        review
            .files
            .iter()
            .find(|file| file.path == Path::new(name))
            .unwrap()
            .clone()
    };

    let (normal, deletions) = review.preview_pair(&repo, &file("file.txt")).unwrap();
    let rows = |diff: &give::git::Diff| {
        diff.lines
            .iter()
            .map(|line| (line.kind, line.old, line.new, line.text.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(&normal),
        [
            (LineKind::Context, Some(1), Some(1), "one".to_owned()),
            (LineKind::Added, None, Some(2), "TWO".to_owned()),
            (LineKind::Context, Some(3), Some(3), "three".to_owned()),
        ]
    );
    assert!(
        rows(&deletions).contains(&(LineKind::Removed, Some(2), None, "two".to_owned())),
        "{deletions:?}"
    );
    assert_eq!((normal.additions, normal.deletions), (1, 1));
    assert_eq!(normal.notice, None);

    let (added, _) = review.preview_pair(&repo, &file("new.rs")).unwrap();
    assert_eq!(
        added.notice.as_deref(),
        Some("New file — showing full content")
    );
    assert_eq!(added.additions, 2);
    assert!(
        added
            .lines
            .iter()
            .all(|line| line.kind == LineKind::Added && line.old.is_none())
    );
    assert_eq!(added.lines[1].new, Some(2));

    let (deleted, _) = review.preview_pair(&repo, &file("old.txt")).unwrap();
    assert_eq!(
        deleted.notice.as_deref(),
        Some("File deleted in this branch.")
    );
    assert!(deleted.lines.is_empty());
}

#[test]
fn renames_are_reviewed_as_modified_destinations() {
    let (dir, repo) = branch_fixture();
    git(dir.path(), &["checkout", "--quiet", "-b", "rename"]);
    git(dir.path(), &["mv", "file.txt", "moved.txt"]);
    commit(dir.path(), "Rename");
    let review = Review::load(&repo, "rename").unwrap();
    let files: Vec<_> = review
        .files
        .iter()
        .map(|file| (file.path.to_string_lossy().into_owned(), file.index))
        .collect();
    assert_eq!(files, [("moved.txt".to_owned(), 'M')]);
}

#[test]
fn branches_and_reviews_include_remote_only_branches() {
    let (dir, repo) = branch_fixture();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--quiet", "--bare"]);
    let url = remote.path().to_string_lossy().into_owned();
    git(dir.path(), &["remote", "add", "origin", &url]);
    git(
        dir.path(),
        &["push", "--quiet", "origin", "main", "feature"],
    );
    git(
        dir.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    // A branch that exists only on the remote.
    git(
        dir.path(),
        &["checkout", "--quiet", "-b", "shared", "feature"],
    );
    fs::write(dir.path().join("shared.txt"), "shared\n").unwrap();
    commit(dir.path(), "Shared");
    git(dir.path(), &["push", "--quiet", "origin", "shared"]);
    git(dir.path(), &["checkout", "--quiet", "main"]);
    git(dir.path(), &["branch", "--quiet", "-D", "shared"]);
    git(
        dir.path(),
        &["update-ref", "-d", "refs/remotes/origin/shared"],
    );

    review::fetch(&repo).unwrap();
    let branches = review::branches(&repo).unwrap();
    let names: Vec<_> = branches
        .iter()
        .map(|branch| {
            (
                branch.name.as_str(),
                branch.remote.as_deref(),
                branch.current,
            )
        })
        .collect();
    assert_eq!(names[0], ("main", None, true));
    assert!(names.contains(&("feature", None, false)));
    assert!(names.contains(&("origin/shared", Some("origin"), false)));
    assert!(names.iter().all(|(name, _, _)| *name != "origin"));
    let first_remote = names.iter().position(|(_, remote, _)| remote.is_some());
    assert!(
        names[first_remote.unwrap()..]
            .iter()
            .all(|(_, remote, _)| remote.is_some())
    );

    for name in ["origin/shared", "shared"] {
        let review = Review::load(&repo, name).unwrap();
        assert_eq!(review.branch, name);
        assert_eq!(review.head, "origin/shared");
        assert_eq!(review.main, "origin/main");
        assert_eq!(
            review.integration_error,
            Some(format!("unsupported origin remote: {url}"))
        );
        assert!(
            review
                .files
                .iter()
                .any(|file| file.path == Path::new("shared.txt"))
        );
    }
}

#[test]
fn unknown_branches_fail_without_a_merge_base() {
    let (_dir, repo) = branch_fixture();
    let error = Review::load(&repo, "missing").unwrap_err();
    assert_eq!(
        error.to_string(),
        "cannot find merge-base for branch missing against main/master"
    );
    assert!(Review::load(&repo, "  ").is_err());
}
