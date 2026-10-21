use give::{git::Repository, history};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned()
}

fn fixture() -> (TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--initial-branch=main"]);
    git(dir.path(), &["config", "user.name", "Автор Тест"]);
    git(
        dir.path(),
        &["config", "user.email", "author@example.invalid"],
    );
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    git(dir.path(), &["config", "core.hooksPath", "/dev/null"]);
    let repo = Repository::open(dir.path()).unwrap();
    (dir, repo)
}

fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "--allow-empty", "-m", message]);
    git(root, &["rev-parse", "HEAD"])
}

#[test]
fn unborn_history_is_empty_but_invalid_refs_are_errors() {
    let (_dir, repo) = fixture();
    assert_eq!(
        history::load(&repo, "HEAD", 100).unwrap(),
        history::Page::default()
    );
    assert!(history::load(&repo, "missing", 100).is_err());
    assert!(history::load(&repo, "--all", 100).is_err());
}

#[test]
fn branch_history_is_bounded_and_excludes_unmerged_commits() {
    let (_dir, repo) = fixture();
    let root = &repo.root;
    let initial = commit(root, "Initial");
    git(root, &["checkout", "-b", "feature"]);
    let feature = commit(root, "Функция\n\nFull message body.");
    git(root, &["checkout", "main"]);
    let main = commit(root, "Main only");
    let page = history::load(&repo, "feature", 1).unwrap();
    assert!(page.has_more);
    assert_eq!(page.commits.len(), 1);
    let entry = &page.commits[0];
    assert_eq!(entry.id, feature);
    assert_eq!(entry.parents.as_slice(), std::slice::from_ref(&initial));
    assert_eq!(entry.author, "Автор Тест");
    assert_eq!(entry.email, "author@example.invalid");
    assert_eq!(entry.subject, "Функция");
    assert!(entry.references.contains("feature"));
    assert!(entry.date.contains(':'));
    let details = history::details(&repo, entry).unwrap();
    assert_eq!(details.message, "Функция\n\nFull message body.");
    assert!(details.files.is_empty());
    let full = history::load(&repo, "feature", 2).unwrap();
    assert!(!full.has_more);
    assert_eq!(
        full.commits
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        [feature.as_str(), initial.as_str()]
    );
    assert_eq!(
        history::load(&repo, "HEAD", 10).unwrap().commits[0].id,
        main
    );
    git(root, &["checkout", "--detach", &feature]);
    let detached = history::load(&repo, "HEAD", 10).unwrap();
    assert_eq!(
        detached.commits.iter().map(|c| &c.id).collect::<Vec<_>>(),
        full.commits.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
    assert!(detached.commits[0].references.contains("HEAD"));
}

#[test]
fn details_preserve_paths_and_read_immutable_commits() {
    let (_dir, repo) = fixture();
    let root = &repo.root;
    let odd = "Юникод\tс пробелом\nfile.txt";
    fs::write(root.join(odd), "same content\n").unwrap();
    fs::write(root.join("delete.txt"), "remove\n").unwrap();
    fs::write(root.join("modify.txt"), "old\n").unwrap();
    commit(root, "Initial");
    let first = history::load(&repo, "HEAD", 10).unwrap().commits.remove(0);
    let initial = history::details(&repo, &first).unwrap();
    assert_eq!(initial.files.len(), 3);
    assert!(initial.files.iter().all(|file| file.status == 'A'));
    fs::rename(root.join(odd), root.join("renamed.txt")).unwrap();
    fs::remove_file(root.join("delete.txt")).unwrap();
    fs::write(root.join("modify.txt"), "new\n").unwrap();
    fs::write(root.join("added.txt"), "added\n").unwrap();
    commit(root, "Changes");
    let changed = history::load(&repo, "HEAD", 10).unwrap().commits.remove(0);
    // Neither the dirty worktree nor subsequent commits may affect these details.
    fs::write(root.join("dirty.txt"), "not committed\n").unwrap();
    let details = history::details(&repo, &changed).unwrap();
    assert_eq!(details.files.len(), 4);
    for (name, status) in [
        ("added.txt", 'A'),
        ("delete.txt", 'D'),
        ("modify.txt", 'M'),
        ("renamed.txt", 'R'),
    ] {
        assert_eq!(
            details
                .files
                .iter()
                .find(|f| f.path == Path::new(name))
                .unwrap()
                .status,
            status
        );
    }
    assert_eq!(
        details
            .files
            .iter()
            .find(|f| f.status == 'R')
            .unwrap()
            .original_path
            .as_deref(),
        Some(Path::new(odd))
    );
    assert_eq!(
        history::details(&repo, &first).unwrap().files,
        initial.files
    );
}

#[test]
fn merge_history_includes_merged_authors_and_files_against_first_parent() {
    let (_dir, repo) = fixture();
    let root = &repo.root;
    commit(root, "Initial");
    git(root, &["checkout", "-b", "feature"]);
    fs::write(root.join("feature.txt"), "feature\n").unwrap();
    git(root, &["config", "user.name", "Feature author"]);
    let feature = commit(root, "Feature change");
    git(root, &["checkout", "main"]);
    fs::write(root.join("main.txt"), "main\n").unwrap();
    let parent = commit(root, "Main change");
    git(
        root,
        &["merge", "--no-ff", "feature", "-m", "Merge feature"],
    );
    let page = history::load(&repo, "HEAD", 100).unwrap();
    let merge = &page.commits[0];
    assert_eq!(merge.parents, [parent, feature.clone()]);
    assert!(
        page.commits
            .iter()
            .any(|c| c.id == feature && c.author == "Feature author")
    );
    let details = history::details(&repo, merge).unwrap();
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].path, Path::new("feature.txt"));
    assert_eq!(details.files[0].status, 'A');
}
