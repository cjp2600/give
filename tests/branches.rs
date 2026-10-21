use give::{
    branches,
    git::Repository,
    review::{self, Branch},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn configure(path: &Path) {
    for (key, value) in [
        ("user.name", "Give tests"),
        ("user.email", "give@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(path, &["config", key, value]);
    }
}

fn commit(path: &Path, text: &str) {
    fs::write(path.join("file.txt"), text).unwrap();
    git(path, &["add", "file.txt"]);
    git(path, &["commit", "--quiet", "-m", text]);
}

struct Fixture {
    _dir: TempDir,
    publisher: PathBuf,
    remote: PathBuf,
    repo: Repository,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let publisher = dir.path().join("publisher");
        let remote = dir.path().join("remote.git");
        let client = dir.path().join("client with spaces");
        fs::create_dir(&publisher).unwrap();
        git(
            dir.path(),
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch=main",
                remote.to_str().unwrap(),
            ],
        );
        git(&publisher, &["init", "--quiet", "--initial-branch=main"]);
        configure(&publisher);
        commit(&publisher, "initial\n");
        git(
            &publisher,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&publisher, &["push", "--quiet", "-u", "origin", "main"]);
        git(&publisher, &["switch", "--quiet", "-c", "feature/nested"]);
        commit(&publisher, "feature\n");
        git(&publisher, &["push", "--quiet", "origin", "feature/nested"]);
        git(&publisher, &["switch", "--quiet", "main"]);
        git(
            dir.path(),
            &[
                "clone",
                "--quiet",
                remote.to_str().unwrap(),
                client.to_str().unwrap(),
            ],
        );
        configure(&client);
        let repo = Repository::open(&client).unwrap();
        Self {
            _dir: dir,
            publisher,
            remote,
            repo,
        }
    }

    fn branch(&self, reference: &str) -> Branch {
        review::branches(&self.repo)
            .unwrap()
            .into_iter()
            .find(|b| b.reference == reference)
            .unwrap()
    }

    fn head(&self) -> String {
        git(&self.repo.root, &["rev-parse", "HEAD"])
    }
}

#[test]
fn remote_checkout_creates_tracking_branch_and_reuses_it_without_resetting() {
    let f = Fixture::new();
    let branch = f.branch("refs/remotes/origin/feature/nested");
    assert_eq!(
        branches::switch(&f.repo, &branch).unwrap(),
        "feature/nested"
    );
    assert_eq!(
        git(
            &f.repo.root,
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        branch.reference
    );
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "feature\n"
    );
    commit(&f.repo.root, "local commit\n");
    let local_head = f.head();
    branches::switch(&f.repo, &f.branch("refs/heads/main")).unwrap();
    branches::switch(&f.repo, &branch).unwrap();
    assert_eq!(
        f.head(),
        local_head,
        "reusing a tracking branch preserves local commits"
    );
    assert_eq!(f.repo.snapshot().unwrap().branch, "feature/nested");
}

#[test]
fn remote_checkout_refuses_a_same_named_local_branch_with_another_upstream() {
    let f = Fixture::new();
    git(&f.repo.root, &["branch", "feature/nested", "main"]);
    git(
        &f.repo.root,
        &["branch", "--set-upstream-to=origin/main", "feature/nested"],
    );
    let before = f.head();
    let error =
        branches::switch(&f.repo, &f.branch("refs/remotes/origin/feature/nested")).unwrap_err();
    assert!(error.to_string().contains("does not track"));
    assert_eq!(f.repo.snapshot().unwrap().branch, "main");
    assert_eq!(f.head(), before);
}

#[test]
fn local_checkout_rejects_overwriting_dirty_files_and_preserves_the_index() {
    let f = Fixture::new();
    git(&f.repo.root, &["branch", "other", "origin/feature/nested"]);
    fs::write(f.repo.root.join("file.txt"), "staged\n").unwrap();
    git(&f.repo.root, &["add", "file.txt"]);
    fs::write(f.repo.root.join("file.txt"), "unstaged\n").unwrap();
    fs::write(f.repo.root.join("untracked.txt"), "untracked\n").unwrap();
    let before = f.head();
    assert!(branches::switch(&f.repo, &f.branch("refs/heads/other")).is_err());
    assert_eq!(f.head(), before);
    assert_eq!(f.repo.snapshot().unwrap().branch, "main");
    assert_eq!(git(&f.repo.root, &["show", ":file.txt"]), "staged");
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(f.repo.root.join("untracked.txt")).unwrap(),
        "untracked\n"
    );
}

#[test]
fn branch_listing_and_switch_are_unambiguous_with_colliding_local_and_remote_names() {
    let f = Fixture::new();
    git(&f.repo.root, &["branch", "origin/feature/nested", "main"]);
    git(&f.repo.root, &["tag", "feature/nested", "main"]);
    let local = f.branch("refs/heads/origin/feature/nested");
    assert_eq!(local.name, "origin/feature/nested");
    let remote = f.branch("refs/remotes/origin/feature/nested");
    assert_eq!(remote.name, local.name);
    branches::switch(&f.repo, &remote).unwrap();
    assert_eq!(
        f.head(),
        git(&f.repo.root, &["rev-parse", &remote.reference])
    );
    branches::switch(&f.repo, &local).unwrap();
    assert_eq!(
        f.head(),
        git(&f.repo.root, &["rev-parse", &local.reference])
    );
}

#[test]
fn fetch_updates_and_prunes_remote_refs_without_touching_the_working_tree_then_pull_advances() {
    let f = Fixture::new();
    let before = f.head();
    commit(&f.publisher, "remote update\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    git(
        &f.publisher,
        &["push", "--quiet", "origin", "--delete", "feature/nested"],
    );
    git(
        &f.publisher,
        &["push", "--quiet", "origin", "main:brand-new"],
    );
    review::fetch(&f.repo).unwrap();
    assert_eq!(f.head(), before);
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "initial\n"
    );
    let refs = review::branches(&f.repo).unwrap();
    assert!(
        refs.iter()
            .any(|b| b.reference == "refs/remotes/origin/brand-new")
    );
    assert!(
        !refs
            .iter()
            .any(|b| b.reference == "refs/remotes/origin/feature/nested")
    );
    branches::pull(&f.repo).unwrap();
    assert_eq!(f.head(), git(&f.publisher, &["rev-parse", "HEAD"]));
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "remote update\n"
    );
    branches::pull(&f.repo).unwrap(); // Already up to date is also success.
}

#[test]
fn pull_fetches_before_updating_and_refuses_diverged_history_without_merging() {
    let f = Fixture::new();
    commit(&f.publisher, "first remote update\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    branches::pull(&f.repo).unwrap();
    assert_eq!(f.head(), git(&f.publisher, &["rev-parse", "HEAD"]));
    commit(&f.repo.root, "client diverges\n");
    let before = f.head();
    commit(&f.publisher, "publisher diverges\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    assert!(branches::pull(&f.repo).is_err());
    assert_eq!(f.head(), before);
    assert_eq!(
        git(&f.repo.root, &["rev-parse", "origin/main"]),
        git(&f.publisher, &["rev-parse", "HEAD"])
    );
    assert!(!f.repo.root.join(".git/MERGE_HEAD").exists());
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "client diverges\n"
    );
}

#[test]
fn pull_keeps_dirty_files_and_does_not_autostash_even_when_configured() {
    let f = Fixture::new();
    commit(&f.publisher, "remote update\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    git(&f.repo.root, &["config", "pull.autostash", "true"]);
    git(&f.repo.root, &["config", "merge.autostash", "true"]);
    fs::write(f.repo.root.join("file.txt"), "my work\n").unwrap();
    let before = f.head();
    assert!(branches::pull(&f.repo).is_err());
    assert_eq!(f.head(), before);
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "my work\n"
    );
    assert_eq!(git(&f.repo.root, &["stash", "list"]), "");
}

#[test]
fn pull_without_upstream_finds_the_unique_same_named_remote_branch() {
    let f = Fixture::new();
    git(&f.repo.root, &["branch", "--unset-upstream"]);
    commit(&f.publisher, "remote update\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    branches::pull(&f.repo).unwrap();
    assert_eq!(f.head(), git(&f.publisher, &["rev-parse", "HEAD"]));
    assert_eq!(
        git(
            &f.repo.root,
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        "refs/remotes/origin/main"
    );
}

#[test]
fn pull_reports_missing_upstream_detached_head_and_network_failures() {
    let f = Fixture::new();
    let before = f.head();
    git(&f.repo.root, &["switch", "-c", "unpublished"]);
    assert!(
        branches::pull(&f.repo)
            .unwrap_err()
            .to_string()
            .contains("tracking")
    );
    git(&f.repo.root, &["switch", "--detach", "HEAD"]);
    assert!(branches::pull(&f.repo).is_err());
    git(&f.repo.root, &["switch", "main"]);
    git(&f.repo.root, &["branch", "--set-upstream-to=origin/main"]);
    fs::rename(&f.remote, f.remote.with_extension("gone")).unwrap();
    assert!(review::fetch(&f.repo).is_err());
    assert!(branches::pull(&f.repo).is_err());
    assert_eq!(f.head(), before);
}

#[test]
fn pull_requires_a_choice_for_multiple_remotes_and_remembers_explicit_choice() {
    let f = Fixture::new();
    git(
        &f.repo.root,
        &["remote", "add", "second", f.remote.to_str().unwrap()],
    );
    git(&f.repo.root, &["branch", "--unset-upstream"]);
    let before = f.head();
    let error = branches::pull(&f.repo).unwrap_err().to_string();
    assert!(error.contains("Pull from"), "{error}");
    assert_eq!(f.head(), before);
    branches::pull_from(&f.repo, &f.branch("refs/remotes/second/feature/nested")).unwrap();
    assert_eq!(
        git(
            &f.repo.root,
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        "refs/remotes/second/feature/nested"
    );
    assert_eq!(f.repo.snapshot().unwrap().branch, "main");
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "feature\n"
    );
}

#[test]
fn failed_explicit_pull_preserves_upstream_and_local_history() {
    let f = Fixture::new();
    commit(&f.repo.root, "local divergence\n");
    let before = f.head();
    assert!(branches::pull_from(&f.repo, &f.branch("refs/remotes/origin/feature/nested")).is_err());
    assert_eq!(f.head(), before);
    assert_eq!(
        git(
            &f.repo.root,
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        "refs/remotes/origin/main"
    );
    assert!(!f.repo.root.join(".git/MERGE_HEAD").exists());
}

#[test]
fn push_publishes_new_branch_sets_tracking_and_can_publish_main_after_merge() {
    let f = Fixture::new();
    git(&f.repo.root, &["switch", "-c", "new-feature"]);
    commit(&f.repo.root, "published feature\n");
    let feature = f.branch("refs/heads/new-feature");
    branches::push(&f.repo, "origin").unwrap();
    assert_eq!(
        git(&f.remote, &["rev-parse", "refs/heads/new-feature"]),
        f.head()
    );
    assert_eq!(
        git(
            &f.repo.root,
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        "refs/remotes/origin/new-feature"
    );
    branches::switch(&f.repo, &f.branch("refs/heads/main")).unwrap();
    branches::merge(&f.repo, &feature).unwrap();
    branches::push(&f.repo, "origin").unwrap();
    assert_eq!(git(&f.remote, &["rev-parse", "refs/heads/main"]), f.head());
}

#[test]
fn push_rejection_preserves_remote_history_even_with_force_push_refspec_configured() {
    let f = Fixture::new();
    commit(&f.publisher, "remote advance\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    let remote_head = git(&f.remote, &["rev-parse", "main"]);
    commit(&f.repo.root, "local divergence\n");
    git(
        &f.repo.root,
        &["config", "remote.origin.push", "+refs/heads/*:refs/heads/*"],
    );
    assert!(branches::push(&f.repo, "origin").is_err());
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), remote_head);
    assert!(branches::push(&f.repo, "missing").is_err());
    git(&f.repo.root, &["switch", "--detach", "HEAD"]);
    assert!(branches::push(&f.repo, "origin").is_err());
}

#[test]
fn commit_and_push_publishes_only_the_index_on_the_current_branch() {
    let f = Fixture::new();
    git(&f.repo.root, &["switch", "-c", "quick-commit"]);
    fs::write(f.repo.root.join("file.txt"), "staged content\n").unwrap();
    git(&f.repo.root, &["add", "file.txt"]);
    fs::write(f.repo.root.join("file.txt"), "unstaged content\n").unwrap();
    fs::write(f.repo.root.join("untracked.txt"), "leave me local\n").unwrap();
    let outcome = branches::commit_and_push(&f.repo, "Ship staged changes").unwrap();
    assert!(matches!(outcome, branches::CommitPushOutcome::Pushed(_)));
    assert_eq!(
        git(&f.remote, &["rev-parse", "refs/heads/quick-commit"]),
        f.head()
    );
    assert_eq!(
        git(&f.remote, &["show", "quick-commit:file.txt"]),
        "staged content"
    );
    assert_eq!(
        git(&f.repo.root, &["log", "-1", "--format=%s"]),
        "Ship staged changes"
    );
    assert_eq!(
        git(&f.repo.root, &["config", "branch.quick-commit.remote"]),
        "origin"
    );
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "unstaged content\n"
    );
    assert_eq!(f.repo.snapshot().unwrap().files.len(), 2);
}

#[test]
fn commit_and_push_uses_configured_push_remote_before_tracking_remote() {
    let f = Fixture::new();
    for remote in ["default-push", "branch-push"] {
        git(
            &f.repo.root,
            &["remote", "add", remote, f.remote.to_str().unwrap()],
        );
    }
    git(
        &f.repo.root,
        &["config", "remote.pushDefault", "default-push"],
    );
    fs::write(f.repo.root.join("file.txt"), "default remote\n").unwrap();
    git(&f.repo.root, &["add", "file.txt"]);
    assert!(matches!(
        branches::commit_and_push(&f.repo, "Use default remote").unwrap(),
        branches::CommitPushOutcome::Pushed(_)
    ));
    assert_eq!(
        git(&f.repo.root, &["config", "branch.main.remote"]),
        "default-push"
    );
    git(
        &f.repo.root,
        &["config", "branch.main.pushRemote", "branch-push"],
    );
    fs::write(f.repo.root.join("file.txt"), "branch remote\n").unwrap();
    git(&f.repo.root, &["add", "file.txt"]);
    assert!(matches!(
        branches::commit_and_push(&f.repo, "Use branch remote").unwrap(),
        branches::CommitPushOutcome::Pushed(_)
    ));
    assert_eq!(
        git(&f.repo.root, &["config", "branch.main.remote"]),
        "branch-push"
    );
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), f.head());
}

#[test]
fn commit_and_push_reports_rejected_push_as_a_successful_local_commit() {
    let f = Fixture::new();
    commit(&f.publisher, "remote advance\n");
    git(&f.publisher, &["push", "--quiet", "origin", "main"]);
    let remote_head = git(&f.remote, &["rev-parse", "main"]);
    let before = f.head();
    fs::write(f.repo.root.join("file.txt"), "local change\n").unwrap();
    git(&f.repo.root, &["add", "file.txt"]);
    let outcome = branches::commit_and_push(&f.repo, "Keep local commit").unwrap();
    assert!(matches!(
        outcome,
        branches::CommitPushOutcome::PushFailed(_)
    ));
    assert_ne!(f.head(), before);
    assert_eq!(git(&f.repo.root, &["rev-parse", "HEAD^"]), before);
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), remote_head);
    assert!(f.repo.snapshot().unwrap().files.is_empty());
}

#[test]
fn commit_and_push_checks_destination_before_committing_and_skips_push_if_commit_fails() {
    let f = Fixture::new();
    let before = f.head();
    fs::write(f.repo.root.join("file.txt"), "pending commit\n").unwrap();
    git(&f.repo.root, &["add", "file.txt"]);
    assert!(branches::commit_and_push(&f.repo, " ").is_err());
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), before);
    git(
        &f.repo.root,
        &["config", "branch.main.pushRemote", "missing"],
    );
    assert!(branches::commit_and_push(&f.repo, "Invalid remote").is_err());
    git(
        &f.repo.root,
        &["config", "--unset", "branch.main.pushRemote"],
    );
    git(&f.repo.root, &["remote", "remove", "origin"]);
    assert!(branches::commit_and_push(&f.repo, "No remote").is_err());
    for remote in ["one", "two"] {
        git(
            &f.repo.root,
            &["remote", "add", remote, f.remote.to_str().unwrap()],
        );
    }
    assert!(branches::commit_and_push(&f.repo, "Ambiguous remote").is_err());
    git(&f.repo.root, &["switch", "--detach", "HEAD"]);
    assert!(branches::commit_and_push(&f.repo, "Detached HEAD").is_err());
    assert_eq!(f.head(), before);
    assert!(
        f.repo
            .snapshot()
            .unwrap()
            .files
            .iter()
            .any(|file| file.staged())
    );
}

#[test]
fn merge_creates_a_commit_for_clean_divergence_without_opening_an_editor() {
    let f = Fixture::new();
    git(&f.repo.root, &["switch", "-c", "topic"]);
    fs::write(f.repo.root.join("topic.txt"), "topic\n").unwrap();
    git(&f.repo.root, &["add", "topic.txt"]);
    git(&f.repo.root, &["commit", "--quiet", "-m", "topic"]);
    let branch = f.branch("refs/heads/topic");
    let source = f.head();
    branches::switch(&f.repo, &f.branch("refs/heads/main")).unwrap();
    commit(&f.repo.root, "main advance\n");
    let target = f.head();
    git(&f.repo.root, &["config", "merge.ff", "only"]);
    git(&f.repo.root, &["config", "merge.autoStash", "true"]);
    branches::merge(&f.repo, &branch).unwrap();
    assert_eq!(git(&f.repo.root, &["rev-parse", "HEAD^1"]), target);
    assert_eq!(git(&f.repo.root, &["rev-parse", "HEAD^2"]), source);
    assert!(f.repo.snapshot().unwrap().files.is_empty());
    assert_eq!(git(&f.repo.root, &["stash", "list"]), "");
}

#[test]
fn merge_reports_conflicting_files_and_abort_restores_target() {
    let f = Fixture::new();
    commit(&f.repo.root, "main change\n");
    let before = f.head();
    let error = branches::merge(&f.repo, &f.branch("refs/remotes/origin/feature/nested"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("Merge conflict"), "{error}");
    assert!(error.contains("file.txt"), "{error}");
    assert!(error.contains("origin/feature/nested → main"), "{error}");
    assert_eq!(f.head(), before);
    assert!(
        f.repo
            .snapshot()
            .unwrap()
            .files
            .iter()
            .any(|file| file.conflicted())
    );
    assert!(f.repo.root.join(".git/MERGE_HEAD").exists());
    branches::abort_merge(&f.repo).unwrap();
    assert_eq!(f.head(), before);
    assert!(f.repo.snapshot().unwrap().files.is_empty());
    assert_eq!(
        fs::read_to_string(f.repo.root.join("file.txt")).unwrap(),
        "main change\n"
    );
}

#[test]
fn merge_requires_clean_worktree_and_uses_full_ref_despite_a_colliding_tag() {
    let f = Fixture::new();
    let branch = f.branch("refs/remotes/origin/feature/nested");
    git(&f.repo.root, &["tag", "origin/feature/nested", "main"]);
    fs::write(f.repo.root.join("untracked.txt"), "keep me\n").unwrap();
    let before = f.head();
    assert!(
        branches::merge(&f.repo, &branch)
            .unwrap_err()
            .to_string()
            .contains("Commit or stash")
    );
    assert_eq!(f.head(), before);
    assert_eq!(
        fs::read_to_string(f.repo.root.join("untracked.txt")).unwrap(),
        "keep me\n"
    );
    fs::remove_file(f.repo.root.join("untracked.txt")).unwrap();
    branches::merge(&f.repo, &branch).unwrap();
    assert_eq!(
        f.head(),
        git(&f.repo.root, &["rev-parse", &branch.reference])
    );
    assert_eq!(f.repo.snapshot().unwrap().branch, "main");
}
