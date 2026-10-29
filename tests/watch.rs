use give::{git::Repository, watch::ProjectWatcher};
use std::{
    fs,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn wait_for_change(watcher: &ProjectWatcher) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !watcher.take_changed() {
        assert!(
            Instant::now() < deadline,
            "filesystem change was not delivered"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn watcher_delivers_ignored_file_and_directory_changes_for_catalog_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    fs::write(dir.path().join(".gitignore"), "cache/\n").unwrap();
    let repo = Repository::open(dir.path()).unwrap();
    let watcher = ProjectWatcher::new(&repo.root).unwrap();
    fs::create_dir(dir.path().join("cache")).unwrap();
    fs::write(dir.path().join("cache/new.txt"), "new\n").unwrap();
    wait_for_change(&watcher);
    let snapshot = repo.snapshot_with(true).unwrap();
    assert!(snapshot.file(Path::new("cache/new.txt")).unwrap().ignored());
    fs::rename(
        dir.path().join("cache/new.txt"),
        dir.path().join("cache/renamed.txt"),
    )
    .unwrap();
    wait_for_change(&watcher);
    let snapshot = repo.snapshot_with(true).unwrap();
    assert!(snapshot.file(Path::new("cache/new.txt")).is_none());
    assert!(
        snapshot
            .file(Path::new("cache/renamed.txt"))
            .unwrap()
            .ignored()
    );
    fs::remove_dir_all(dir.path().join("cache")).unwrap();
    wait_for_change(&watcher);
    assert!(
        repo.snapshot_with(true)
            .unwrap()
            .file(Path::new("cache"))
            .is_none()
    );
}
