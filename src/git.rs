use anyhow::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    fs,
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

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    pub kind: LineKind,
}

#[derive(Clone, Debug, Default)]
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
        Ok(Snapshot {
            branch,
            files: parse_status(&raw)?,
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
            text: text.replace('\t', "    "),
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

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
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

fn run<I, S>(root: &Path, args: I, input: Option<&[u8]>, commit: bool) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start Git. Install Git and make it available on PATH.")?;
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
    let status = child.wait_timeout(Duration::from_secs(if commit { 120 } else { 15 }))?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let out = out_thread
        .join()
        .map_err(|_| anyhow::anyhow!("Git output reader failed"))??;
    let err = err_thread
        .join()
        .map_err(|_| anyhow::anyhow!("Git diagnostic reader failed"))??;
    let status = status.context("Git operation timed out.")?;
    if !status.success() {
        bail!("{}", String::from_utf8_lossy(&err).trim());
    }
    if out.len() > OUTPUT_LIMIT {
        bail!("Git output exceeds 4 MiB. Inspect this change in your editor.");
    }
    Ok(out)
}
