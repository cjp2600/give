use anyhow::{Context, Result, bail};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};
use wait_timeout::ChildExt;

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const PREVIEW_LIMIT: u64 = 1024 * 1024;
const LINE_LIMIT: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    /// Explicit disk directories, including empty ones, appear only in the catalog.
    pub directory: bool,
    pub original_path: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
}

impl ChangedFile {
    pub fn staged(&self) -> bool {
        self.index != ' ' && self.index != '?' && !self.ignored()
    }

    pub fn unstaged(&self) -> bool {
        self.worktree != ' ' && !self.ignored()
    }

    pub fn untracked(&self) -> bool {
        self.index == '?'
    }

    pub fn ignored(&self) -> bool {
        self.index == '!' && self.worktree == '!'
    }

    pub fn changed(&self) -> bool {
        !self.clean() && !self.ignored() && !self.directory
    }

    /// A catalog file without any change.
    pub fn clean(&self) -> bool {
        self.index == ' ' && self.worktree == ' '
    }

    pub fn conflicted(&self) -> bool {
        matches!(
            (self.index, self.worktree),
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D')
        )
    }

    pub fn status(&self, staged: bool) -> char {
        if self.untracked() {
            '+'
        } else if staged {
            self.index
        } else {
            self.worktree
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub branch: String,
    pub files: Vec<ChangedFile>,
    /// Disk files and directories, sorted by path. Ignored entries use `'!!'`;
    /// changed files retain their Git status. Empty unless requested.
    pub catalog: Vec<ChangedFile>,
    pub versions: HashMap<PathBuf, u64>,
}

impl Snapshot {
    /// A changed file, or a clean catalog file.
    pub fn file(&self, path: &Path) -> Option<&ChangedFile> {
        self.files
            .iter()
            .find(|file| file.path == path)
            .or_else(|| {
                self.catalog
                    .binary_search_by(|file| file.path.as_path().cmp(path))
                    .ok()
                    .map(|i| &self.catalog[i])
            })
    }

    /// Show a pasted file at once; the next scan confirms its Git status.
    pub fn insert_new_file(&mut self, path: PathBuf) {
        let ignored = path.ancestors().skip(1).any(|parent| {
            self.catalog
                .binary_search_by(|file| file.path.as_path().cmp(parent))
                .is_ok_and(|i| self.catalog[i].ignored())
        });
        let status = if ignored { '!' } else { '?' };
        let file = ChangedFile {
            path,
            directory: false,
            original_path: None,
            index: status,
            worktree: status,
        };
        let insert = |files: &mut Vec<ChangedFile>, file: ChangedFile| match files
            .binary_search_by(|other| other.path.cmp(&file.path))
        {
            Ok(i) => files[i] = file,
            Err(i) => files.insert(i, file),
        };
        if !ignored {
            insert(&mut self.files, file.clone());
        }
        insert(&mut self.catalog, file);
    }

    /// Hide a deleted file at once; the next scan confirms its Git status.
    pub fn remove_file(&mut self, path: &Path) {
        self.files.retain(|file| file.path != path);
        self.catalog.retain(|file| file.path != path);
        self.versions.remove(path);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Working,
    Staged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Header,
    Hunk,
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    pub kind: LineKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
    pub raw: String,
    pub additions: usize,
    pub deletions: usize,
    pub notice: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
}

impl Repository {
    pub fn open(path: &Path) -> Result<Self> {
        let path = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let root = run(path, ["rev-parse", "--show-toplevel"], None, false)?;
        // Strip Git's final newline, preserving spaces in the repository name.
        let root = root.strip_suffix(b"\n").unwrap_or(&root);
        Ok(Self {
            root: path_from_bytes(root)?,
        })
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.snapshot_with(false)
    }

    /// With `catalog`, also reads disk entries, including ignored and hidden paths.
    /// Git queries and the disk walk run concurrently; only ignore rules wait for both.
    pub fn snapshot_with(&self, catalog: bool) -> Result<Snapshot> {
        thread::scope(|scope| {
            let walk = catalog.then(|| scope.spawn(|| walk_disk(&self.root)));
            let tracked = catalog.then(|| {
                scope.spawn(|| run(&self.root, ["ls-files", "--cached", "-z"], None, false))
            });
            let branch = scope.spawn(|| self.branch());
            let head = scope.spawn(|| {
                run(&self.root, ["rev-parse", "--verify", "HEAD"], None, false).unwrap_or_default()
            });
            let raw = run(
                &self.root,
                ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                None,
                false,
            )?;
            let files = parse_status(&raw)?;
            let catalog = match (walk, tracked) {
                (Some(walk), Some(tracked)) => {
                    self.disk_catalog(&files, joined(walk)?, &joined(tracked)?)?
                }
                _ => Vec::new(),
            };
            let branch = joined(branch)?;
            let head = joined(head);
            // A porcelain status can stay " M" while the file changes repeatedly.
            // Per-file versions invalidate only that file, also when HEAD changes.
            let versions = files
                .iter()
                .chain(catalog.iter().filter(|file| !file.changed()))
                .map(|file| (file.path.clone(), self.version(&head, file)))
                .collect();
            Ok(Snapshot {
                branch,
                files,
                catalog,
                versions,
            })
        })
    }

    fn branch(&self) -> Result<String> {
        Ok(
            match run(
                &self.root,
                ["symbolic-ref", "--quiet", "--short", "HEAD"],
                None,
                false,
            ) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).trim_end().to_owned(),
                Err(_) => {
                    let bytes = run(&self.root, ["rev-parse", "--short", "HEAD"], None, false)?;
                    format!("detached · {}", String::from_utf8_lossy(&bytes).trim_end())
                }
            },
        )
    }

    fn version(&self, head: &[u8], file: &ChangedFile) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        head.hash(&mut hash);
        file.index.hash(&mut hash);
        file.worktree.hash(&mut hash);
        if let Ok(metadata) = fs::metadata(self.root.join(&file.path)) {
            metadata.len().hash(&mut hash);
            metadata.modified().ok().hash(&mut hash);
        }
        hash.finish()
    }

    fn disk_catalog(
        &self,
        files: &[ChangedFile],
        entries: Vec<(PathBuf, bool)>,
        tracked: &[u8],
    ) -> Result<Vec<ChangedFile>> {
        let mut catalog: HashMap<_, _> = files
            .iter()
            .map(|file| (file.path.clone(), file.clone()))
            .collect();
        for (path, directory) in entries {
            catalog
                .entry(path.clone())
                .and_modify(|file| file.directory = directory)
                .or_insert(ChangedFile {
                    path,
                    directory,
                    original_path: None,
                    index: ' ',
                    worktree: ' ',
                });
        }
        if !catalog.is_empty() {
            let tracked: HashSet<_> = tracked
                .split(|b| *b == 0)
                .filter(|path| !path.is_empty())
                .map(path_from_bytes)
                .collect::<Result<_>>()?;
            // Bound each response: ignored build/dependency trees can have many paths.
            let mut paths: Vec<_> = catalog.keys().cloned().collect();
            paths.sort();
            let mut ignored_directories = HashSet::new();
            for batch in paths.chunks(1024) {
                let mut input = Vec::new();
                for path in batch {
                    let file = catalog.get_mut(path).unwrap();
                    if tracked.contains(path) || file.changed() {
                        continue;
                    }
                    // Git cannot re-include a file below an excluded directory.
                    // Reuse that classification for large build/dependency trees.
                    if path
                        .ancestors()
                        .skip(1)
                        .any(|parent| ignored_directories.contains(parent))
                    {
                        file.index = '!';
                        file.worktree = '!';
                        continue;
                    }
                    // Leading ./ prevents names such as :(glob)* from becoming pathspecs.
                    input.extend_from_slice(b"./");
                    input.extend_from_slice(path.as_os_str().as_encoded_bytes());
                    input.push(0);
                }
                if input.is_empty() {
                    continue;
                }
                // Verbose records distinguish negations and preserve unusual names.
                // Exit 1 means no paths matched, a successful classification.
                let mut command = git_command(&self.root);
                // check-ignore accepts literal stdin paths, but rejects this pathspec flag.
                command.env_remove("GIT_LITERAL_PATHSPECS");
                command.args([
                    "check-ignore",
                    // Permit paths inside checked-out submodules; protect tracked paths below.
                    "--no-index",
                    "--stdin",
                    "-z",
                    "--verbose",
                    "--non-matching",
                ]);
                let ignored = execute_with_no_match(
                    command,
                    "Git",
                    Some(&input),
                    Duration::from_secs(15),
                    true,
                )?;
                let fields: Vec<_> = ignored.split(|b| *b == 0).collect();
                for record in fields.chunks_exact(4) {
                    let pattern = record[2];
                    if !pattern.is_empty() && !pattern.starts_with(b"!") {
                        let path =
                            path_from_bytes(record[3].strip_prefix(b"./").unwrap_or(record[3]))?;
                        if !tracked.contains(&path)
                            && let Some(file) = catalog.get_mut(&path)
                            && !file.changed()
                        {
                            file.index = '!';
                            file.worktree = '!';
                            if file.directory {
                                ignored_directories.insert(path);
                            }
                        }
                    }
                }
            }
        }
        let mut catalog: Vec<_> = catalog.into_values().collect();
        catalog.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(catalog)
    }

    pub fn diff(&self, file: &ChangedFile, layer: Layer) -> Result<Diff> {
        if file.ignored() {
            return self.preview(file, false);
        }
        if file.untracked() {
            return self.untracked_diff(&file.path);
        }
        let mut args: Vec<OsString> = [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=4",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if layer == Layer::Staged {
            args.push("--cached".into());
        }
        args.push("--".into());
        args.push(file.path.as_os_str().into());
        if let Some(original) = &file.original_path {
            args.push(original.as_os_str().into());
        }
        let raw = run(&self.root, args, None, false)?;
        Ok(parse_diff(&String::from_utf8_lossy(&raw)))
    }

    fn untracked_diff(&self, path: &Path) -> Result<Diff> {
        let full = self.root.join(path);
        let metadata = fs::symlink_metadata(&full)?;
        let bytes = if metadata.file_type().is_symlink() {
            fs::read_link(&full)?.to_string_lossy().as_bytes().to_vec()
        } else if !metadata.is_file() {
            return Ok(Diff {
                notice: Some("Preview is unavailable for this file type.".into()),
                ..Diff::default()
            });
        } else {
            let mut bytes = Vec::new();
            fs::File::open(&full)?
                .take(PREVIEW_LIMIT + 1)
                .read_to_end(&mut bytes)?;
            bytes
        };
        if bytes.len() as u64 > PREVIEW_LIMIT {
            return Ok(Diff {
                notice: Some(
                    "File is larger than 1 MiB. Open it in your editor to inspect it.".into(),
                ),
                ..Diff::default()
            });
        }
        if bytes.contains(&0) {
            return Ok(Diff {
                notice: Some("Binary file · text preview unavailable.".into()),
                ..Diff::default()
            });
        }
        let text = String::from_utf8_lossy(&bytes);
        let raw = format!(
            "--- /dev/null\n+++ {}\n@@ -0,0 +1,{} @@\n{}",
            path.display(),
            text.lines().count(),
            text.lines()
                .map(|line| format!("+{line}\n"))
                .collect::<String>()
        );
        let mut result = parse_diff(&raw);
        for (i, line) in result
            .lines
            .iter_mut()
            .filter(|line| line.kind == LineKind::Added)
            .enumerate()
        {
            line.new = Some(i + 1);
        }
        if bytes.is_empty() {
            result.notice = Some("New empty file. Ready to stage.".into());
        }
        Ok(result)
    }

    /// Live giv preview: all uncommitted changes vs HEAD, over the full file.
    /// Keep `diff` separate for callers that explicitly need index/worktree patches.
    pub fn preview(&self, file: &ChangedFile, show_deletions: bool) -> Result<Diff> {
        let (normal, deletions) = self.preview_pair(file)?;
        Ok(if show_deletions { deletions } else { normal })
    }

    /// Both modes share one Git subprocess result and one full-file read.
    pub fn preview_pair(&self, file: &ChangedFile) -> Result<(Diff, Diff)> {
        use crate::preview;
        let mut diff = Diff::default();
        let new_file = file.untracked() || file.ignored() || file.index == 'A';
        if !new_file {
            let has_head = run(&self.root, ["rev-parse", "--verify", "HEAD"], None, false).is_ok();
            let mut args: Vec<OsString> =
                ["diff", "-w", "--no-ext-diff", "--no-textconv", "--no-color"]
                    .into_iter()
                    .map(Into::into)
                    .collect();
            if preview::known_text(&file.path) {
                args.push("--text".into());
            }
            if has_head {
                args.push("HEAD".into());
            } else if file.staged() {
                args.push("--cached".into());
            }
            args.push("--".into());
            args.push(file.path.as_os_str().into());
            // The path selected by giv is the destination of a rename.
            let raw = run(&self.root, args, None, false)?;
            diff = parse_diff(&String::from_utf8_lossy(&raw));
        }
        if !preview::known_text(&file.path)
            && (preview::binary_path(&file.path) || preview::binary_notice(&diff.raw))
        {
            diff.lines.clear();
            diff.notice = Some("Binary file — content not shown.".into());
            return Ok((diff.clone(), diff));
        }
        let full = self.root.join(&file.path);
        let body = (|| -> Result<Vec<u8>> {
            if !fs::metadata(&full)?.is_file() {
                bail!("Preview is unavailable for this file type.");
            }
            Ok(fs::read(&full)?)
        })();
        let build = |show_deletions| {
            let mut result = diff.clone();
            match &body {
                Err(error) => {
                    result.lines = preview::fallback(&diff, show_deletions);
                    result.notice = Some(format!("{error:#}"));
                }
                Ok(body) => {
                    if !preview::known_text(&file.path) && preview::binary_content(body) {
                        result.lines.clear();
                        result.notice = Some("Binary file — content not shown.".into());
                        return result;
                    }
                    if new_file || diff.raw.trim().is_empty() {
                        result.lines = preview::full_context(body);
                        result.notice = Some(
                            if file.ignored() {
                                "Ignored by Git — showing file from disk"
                            } else if file.untracked() {
                                "Untracked file — showing full content"
                            } else if new_file {
                                "New file — showing full content"
                            } else {
                                "No uncommitted changes vs HEAD — showing file from disk"
                            }
                            .into(),
                        );
                    } else {
                        result.lines = preview::overlay(body, &diff.raw, show_deletions)
                            .unwrap_or_else(|| preview::fallback(&diff, show_deletions));
                        result.notice = None;
                    }
                    if result.lines.len() >= preview::MAX_LINES {
                        result.notice = Some("Preview limited to 6,000 lines.".into());
                    }
                }
            }
            result
        };
        Ok((build(false), build(true)))
    }

    pub fn stage(&self, file: Option<&ChangedFile>) -> Result<()> {
        let mut args: Vec<OsString> = vec!["add".into(), "--all".into(), "--".into()];
        match file {
            Some(file) => {
                args.push(file.path.as_os_str().into());
                if file.worktree == 'R'
                    && let Some(original) = &file.original_path
                {
                    args.push(original.as_os_str().into());
                }
            }
            None => args.push(".".into()),
        }
        run(&self.root, args, None, false)?;
        Ok(())
    }

    pub fn unstage(&self, file: Option<&ChangedFile>) -> Result<()> {
        let has_head = run(&self.root, ["rev-parse", "--verify", "HEAD"], None, false).is_ok();
        let mut args: Vec<OsString> = if has_head {
            vec!["reset".into(), "--quiet".into(), "HEAD".into(), "--".into()]
        } else {
            vec![
                "rm".into(),
                "--cached".into(),
                "--quiet".into(),
                "-r".into(),
                "--force".into(),
                "--".into(),
            ]
        };
        match file {
            Some(file) => {
                args.push(file.path.as_os_str().into());
                if let Some(original) = &file.original_path {
                    args.push(original.as_os_str().into());
                }
            }
            None => args.push(".".into()),
        }
        run(&self.root, args, None, false)?;
        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<()> {
        if message.trim().is_empty() {
            bail!("Write a commit message first.");
        }
        if !self.snapshot()?.files.iter().any(ChangedFile::staged) {
            bail!("Stage changes before committing.");
        }
        // Only the existing index is committed. Message is stdin, never shell code.
        run(
            &self.root,
            ["commit", "--file=-"],
            Some(message.as_bytes()),
            true,
        )?;
        Ok(())
    }
}

fn joined<T>(handle: thread::ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Disk entries below `root` except `.git`, as (relative path, is directory).
/// Workers share one queue of directories: build trees dominate large projects,
/// so splitting by top-level folder would leave most threads idle.
fn walk_disk(root: &Path) -> Result<Vec<(PathBuf, bool)>> {
    struct Queue {
        pending: Vec<PathBuf>,
        active: usize,
        error: Option<anyhow::Error>,
    }
    let queue = std::sync::Mutex::new(Queue {
        pending: vec![PathBuf::new()],
        active: 0,
        error: None,
    });
    let ready = std::sync::Condvar::new();
    let workers = thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 8));
    let read = |parent: &Path, found: &mut Vec<(PathBuf, bool)>, subdirectories: &mut Vec<_>| {
        let entries = match fs::read_dir(root.join(parent)) {
            Ok(entries) => entries,
            // A directory may disappear between enumeration and reading it.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("Could not read catalog directory"),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_name() == ".git" {
                continue;
            }
            let path = parent.join(entry.file_name());
            // Do not follow symlinks: cycles and paths outside the project stay leaves.
            let directory = match entry.file_type() {
                Ok(kind) => kind.is_dir(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if directory {
                subdirectories.push(path.clone());
            }
            found.push((path, directory));
        }
        anyhow::Ok(())
    };
    let results = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut found = Vec::new();
                    loop {
                        let parent = {
                            let mut queue = queue.lock().unwrap();
                            loop {
                                if queue.error.is_some() {
                                    return found;
                                }
                                if let Some(parent) = queue.pending.pop() {
                                    queue.active += 1;
                                    break parent;
                                }
                                if queue.active == 0 {
                                    return found;
                                }
                                queue = ready.wait(queue).unwrap();
                            }
                        };
                        let mut subdirectories = Vec::new();
                        let result = read(&parent, &mut found, &mut subdirectories);
                        let mut queue = queue.lock().unwrap();
                        queue.active -= 1;
                        queue.pending.append(&mut subdirectories);
                        if let Err(error) = result {
                            queue.error.get_or_insert(error);
                        }
                        ready.notify_all();
                    }
                })
            })
            .collect();
        handles.into_iter().map(joined).collect::<Vec<_>>()
    });
    if let Some(error) = queue.into_inner().unwrap().error {
        return Err(error);
    }
    Ok(results.into_iter().flatten().collect())
}

pub fn parse_status(raw: &[u8]) -> Result<Vec<ChangedFile>> {
    let mut records = raw.split(|b| *b == 0).filter(|record| !record.is_empty());
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            bail!("Invalid Git status record.");
        }
        let original_path = if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C')
        {
            Some(path_from_bytes(
                records
                    .next()
                    .context("Missing rename source in Git status")?,
            )?)
        } else {
            None
        };
        files.push(ChangedFile {
            path: path_from_bytes(&record[3..])?,
            directory: false,
            original_path,
            index: record[0] as char,
            worktree: record[1] as char,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Joins NUL-delimited `git ls-files` paths with the changed files: changed files keep
/// their status, every other path is clean. Staged deletions stay listed.
pub fn merge_catalog(files: &[ChangedFile], listed: &[u8]) -> Result<Vec<ChangedFile>> {
    let mut catalog: HashMap<PathBuf, ChangedFile> = files
        .iter()
        .map(|file| (file.path.clone(), file.clone()))
        .collect();
    for record in listed
        .split(|b| *b == 0)
        .filter(|record| !record.is_empty())
    {
        let path = path_from_bytes(record)?;
        catalog.entry(path.clone()).or_insert(ChangedFile {
            path,
            directory: false,
            original_path: None,
            index: ' ',
            worktree: ' ',
        });
    }
    let mut catalog: Vec<_> = catalog.into_values().collect();
    catalog.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(catalog)
}

pub fn parse_diff(raw: &str) -> Diff {
    let mut diff = Diff {
        raw: raw.to_owned(),
        ..Diff::default()
    };
    let (mut old, mut new) = (0, 0);
    let mut in_hunk = false;
    for text in raw.lines().take(LINE_LIMIT) {
        let kind = if text.starts_with("@@ ") {
            let mut parts = text.split_whitespace();
            parts.next();
            old = parts
                .next()
                .and_then(|p| p.trim_start_matches('-').split(',').next()?.parse().ok())
                .unwrap_or(0);
            new = parts
                .next()
                .and_then(|p| p.trim_start_matches('+').split(',').next()?.parse().ok())
                .unwrap_or(0);
            in_hunk = true;
            LineKind::Hunk
        } else if text.starts_with("diff --git ")
            || text.starts_with("--- ") && !in_hunk
            || text.starts_with("+++ ") && !in_hunk
        {
            LineKind::Header
        } else if text.starts_with('+') {
            LineKind::Added
        } else if text.starts_with('-') {
            LineKind::Removed
        } else if text.starts_with(' ') && in_hunk {
            LineKind::Context
        } else {
            LineKind::Header
        };
        let (old_number, new_number) = match kind {
            LineKind::Added => {
                diff.additions += 1;
                new += 1;
                (None, Some(new - 1))
            }
            LineKind::Removed => {
                diff.deletions += 1;
                old += 1;
                (Some(old - 1), None)
            }
            LineKind::Context => {
                old += 1;
                new += 1;
                (Some(old - 1), Some(new - 1))
            }
            _ => (None, None),
        };
        diff.lines.push(DiffLine {
            old: old_number,
            new: new_number,
            text: text.to_owned(),
            kind,
        });
    }
    if raw.lines().count() > LINE_LIMIT {
        diff.notice =
            Some("Preview limited to 20,000 lines. Copy diff includes the full patch.".into());
    }
    if raw.contains("Binary files ") {
        diff.notice = Some("Binary file changed · text preview unavailable.".into());
    }
    if raw.is_empty() {
        diff.notice = Some(
            "No text changes in this layer. The file may have a mode change or be a submodule."
                .into(),
        );
    }
    diff
}

pub(crate) fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(std::str::from_utf8(bytes)?))
    }
}

fn read_bounded(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut stream)
        .take(OUTPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    // Drain the pipe even after the limit, so the subprocess cannot block on stdout.
    std::io::copy(&mut stream, &mut std::io::sink())?;
    Ok(bytes)
}

pub(crate) fn run<I, S>(root: &Path, args: I, input: Option<&[u8]>, long: bool) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = git_command(root);
    command.args(args);
    execute(
        command,
        "Git",
        input,
        Duration::from_secs(if long { 120 } else { 15 }),
    )
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C");
    command
}

/// Bounded subprocess with a timeout; stdout and stderr drain concurrently.
pub(crate) fn execute(
    command: Command,
    name: &str,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Vec<u8>> {
    execute_with_no_match(command, name, input, timeout, false)
}

fn execute_with_no_match(
    mut command: Command,
    name: &str,
    input: Option<&[u8]>,
    timeout: Duration,
    allow_no_match: bool,
) -> Result<Vec<u8>> {
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!("Could not start {name}. Install {name} and make it available on PATH.")
        })?;
    let stdout = child.stdout.take().context("Missing stdout")?;
    let stderr = child.stderr.take().context("Missing stderr")?;
    let out_thread = thread::spawn(move || read_bounded(stdout));
    let err_thread = thread::spawn(move || read_bounded(stderr));
    if let Some(input) = input {
        child
            .stdin
            .take()
            .context("Missing stdin")?
            .write_all(input)?;
    }
    let status = child.wait_timeout(timeout)?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let out = out_thread
        .join()
        .map_err(|_| anyhow::anyhow!("{name} output reader failed"))??;
    let err = err_thread
        .join()
        .map_err(|_| anyhow::anyhow!("{name} diagnostic reader failed"))??;
    let status = status.with_context(|| format!("{name} operation timed out."))?;
    if !(status.success() || allow_no_match && status.code() == Some(1)) {
        let mut detail = String::from_utf8_lossy(&err).trim().to_owned();
        if detail.is_empty() {
            detail = String::from_utf8_lossy(&out).trim().to_owned();
        }
        if detail.is_empty() {
            detail = format!("{name} exited with {status}");
        }
        bail!("{detail}");
    }
    if out.len() > OUTPUT_LIMIT {
        bail!("{name} output exceeds 4 MiB. Inspect this change in your editor.");
    }
    Ok(out)
}
