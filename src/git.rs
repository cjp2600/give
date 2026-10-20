use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
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
    pub original_path: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
}

impl ChangedFile {
    pub fn staged(&self) -> bool {
        self.index != ' ' && self.index != '?'
    }

    pub fn unstaged(&self) -> bool {
        self.worktree != ' '
    }

    pub fn untracked(&self) -> bool {
        self.index == '?'
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
    /// Every non-ignored project file, sorted by path, with changed files as in
    /// `files` and clean ones as `'  '`. Empty unless requested.
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

    /// With `catalog`, also lists every tracked and untracked, non-ignored file.
    pub fn snapshot_with(&self, catalog: bool) -> Result<Snapshot> {
        let raw = run(
            &self.root,
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            None,
            false,
        )?;
        let branch = match run(
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
        };
        let files = parse_status(&raw)?;
        let catalog = if catalog {
            let listed = run(
                &self.root,
                [
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                ],
                None,
                false,
            )?;
            merge_catalog(&files, &listed)?
        } else {
            Vec::new()
        };
        let head =
            run(&self.root, ["rev-parse", "--verify", "HEAD"], None, false).unwrap_or_default();
        // A porcelain status can stay " M" while the file changes repeatedly.
        // Per-file versions invalidate only that file, also when HEAD changes.
        let versions = files
            .iter()
            .chain(catalog.iter().filter(|file| file.clean()))
            .map(|file| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                head.hash(&mut hash);
                file.index.hash(&mut hash);
                file.worktree.hash(&mut hash);
                if let Ok(metadata) = fs::metadata(self.root.join(&file.path)) {
                    metadata.len().hash(&mut hash);
                    metadata.modified().ok().hash(&mut hash);
                }
                (file.path.clone(), hash.finish())
            })
            .collect();
        Ok(Snapshot {
            branch,
            files,
            catalog,
            versions,
        })
    }

    pub fn diff(&self, file: &ChangedFile, layer: Layer) -> Result<Diff> {
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
        let new_file = file.untracked() || file.index == 'A';
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
                            if file.untracked() {
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
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C");
    execute(
        command,
        "Git",
        input,
        Duration::from_secs(if long { 120 } else { 15 }),
    )
}

/// Bounded subprocess with a timeout; stdout and stderr drain concurrently.
pub(crate) fn execute(
    mut command: Command,
    name: &str,
    input: Option<&[u8]>,
    timeout: Duration,
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
    if !status.success() {
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
