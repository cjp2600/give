//! Branch review, following giv's `--review` algorithm: fetch all remotes, take
//! the GitLab merge request diff refs (or the merge-base with main), list
//! `base..head` and preview immutable blobs at `head`.
use crate::{
    git::{self, ChangedFile, Diff, LineKind, Repository, Snapshot},
    preview,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    ffi::OsString,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// giv's base candidates, in order.
const MAIN_CANDIDATES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// The name Git resolves: `feature` or `origin/feature`.
    pub name: String,
    /// Remote of a remote-tracking branch.
    pub remote: Option<String>,
    pub current: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitLab,
    GitHub,
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeRequest {
    pub iid: u64,
    pub web_url: String,
    pub base: String,
    pub start: String,
    pub head: String,
}

#[derive(Clone, Debug)]
pub struct Review {
    /// The branch as requested.
    pub branch: String,
    pub base: String,
    pub head: String,
    /// The main ref of the merge-base; empty when the merge request supplies refs.
    pub main: String,
    pub forge: Forge,
    pub merge_request: Option<MergeRequest>,
    /// Why forge integration is unavailable. Local branch review still works.
    pub integration_error: Option<String>,
    pub files: Vec<ChangedFile>,
}

/// Local branches first (the current one on top), then remote-tracking branches,
/// each by most recent commit.
pub fn branches(repo: &Repository) -> Result<Vec<Branch>> {
    let remotes = remotes(repo)?;
    let raw = git::run(
        &repo.root,
        [
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(HEAD)%00%(refname)%00%(refname:short)%00%(symref)",
            "refs/heads",
            "refs/remotes",
        ],
        None,
        false,
    )?;
    let mut branches: Vec<Branch> = String::from_utf8_lossy(&raw)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            let head = fields.next()?;
            let full = fields.next()?;
            let name = fields.next()?;
            // `origin/HEAD` points at another branch.
            if !fields.next().unwrap_or_default().is_empty() {
                return None;
            }
            let remote = match full.strip_prefix("refs/remotes/") {
                Some(rest) => Some(
                    remotes
                        .iter()
                        .filter(|remote| {
                            rest.strip_prefix(remote.as_str())
                                .is_some_and(|tail| tail.starts_with('/'))
                        })
                        .max_by_key(|remote| remote.len())?
                        .clone(),
                ),
                None => None,
            };
            Some(Branch {
                name: name.to_owned(),
                remote,
                current: head == "*",
            })
        })
        .collect();
    branches.sort_by_key(|branch| (!branch.current, branch.remote.is_some()));
    Ok(branches)
}

pub fn fetch(repo: &Repository) -> Result<()> {
    git::run(&repo.root, ["fetch", "--all", "--prune"], None, true)?;
    Ok(())
}

/// Inspects origin without a network request.
pub fn detect_forge(repo: &Repository) -> (Forge, String) {
    let Ok(raw) = git::run(&repo.root, ["remote", "get-url", "origin"], None, false) else {
        return (Forge::Unknown, String::new());
    };
    let remote = String::from_utf8_lossy(&raw).trim().to_owned();
    let host = remote.to_lowercase();
    let forge = if host.contains("gitlab") {
        Forge::GitLab
    } else if host.contains("github") {
        Forge::GitHub
    } else {
        Forge::Unknown
    };
    (forge, remote)
}

/// giv's `FindBranchBase`: the merge-base with the first existing main ref.
pub fn find_base(repo: &Repository, branch: &str) -> Result<(String, String)> {
    for candidate in MAIN_CANDIDATES {
        if !resolves(repo, candidate) {
            continue;
        }
        if let Ok(raw) = git::run(&repo.root, ["merge-base", candidate, branch], None, false) {
            let base = String::from_utf8_lossy(&raw).trim().to_owned();
            if !base.is_empty() {
                return Ok((base, candidate.to_owned()));
            }
        }
    }
    bail!("cannot find merge-base for branch {branch} against main/master")
}

/// `git diff --name-status base head`, mapped as giv does: renames read as
/// modifications of their destination.
pub fn changed_files(repo: &Repository, base: &str, head: &str) -> Result<Vec<ChangedFile>> {
    let raw = git::run(
        &repo.root,
        ["diff", "--name-status", "-z", "--no-ext-diff", base, head],
        None,
        false,
    )?;
    let mut fields = raw.split(|b| *b == 0).filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        if matches!(status.first(), Some(b'R' | b'C')) {
            fields.next().context("Missing rename source in Git diff")?;
        }
        let path = fields.next().context("Missing path in Git diff")?;
        files.push(ChangedFile {
            path: git::path_from_bytes(path)?,
            original_path: None,
            index: match status.first() {
                Some(b'A') => 'A',
                Some(b'D') => 'D',
                _ => 'M',
            },
            worktree: ' ',
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

impl Review {
    /// Everything after the fetch: forge context, refs and the changed files.
    pub fn load(repo: &Repository, branch: &str) -> Result<Self> {
        let branch = branch.trim();
        if branch.is_empty() {
            bail!("Enter a branch name.");
        }
        let (local, source) = source_branch(repo, branch)?;
        let (forge, remote) = detect_forge(repo);
        let (merge_request, integration_error) = match forge {
            Forge::GitLab => match load_merge_request(repo, &source) {
                Ok(request) => (Some(request), None),
                Err(error) => (None, Some(format!("{error:#}"))),
            },
            Forge::GitHub => (
                None,
                Some("GitHub review integration is not supported yet".into()),
            ),
            Forge::Unknown if remote.is_empty() => {
                (None, Some("origin remote is not configured".into()))
            }
            Forge::Unknown => (None, Some(format!("unsupported origin remote: {remote}"))),
        };
        let (base, head, main) = match &merge_request {
            Some(request) if !request.base.is_empty() && !request.head.is_empty() => {
                (request.base.clone(), request.head.clone(), String::new())
            }
            _ => {
                let (base, main) = find_base(repo, &local)?;
                (base, local, main)
            }
        };
        let files = changed_files(repo, &base, &head).context("diff files")?;
        Ok(Self {
            branch: branch.to_owned(),
            base,
            head,
            main,
            forge,
            merge_request,
            integration_error,
            files,
        })
    }

    /// The status line giv shows for the forge integration.
    pub fn status(&self) -> String {
        if let Some(request) = &self.merge_request {
            return format!("MR !{} · {}", request.iid, request.web_url);
        }
        let error = self.integration_error.clone().unwrap_or_default();
        match self.forge {
            Forge::GitLab if error.contains("glab is not installed") => {
                "Install glab for full GitLab review".into()
            }
            Forge::GitLab => format!("GitLab review unavailable: {error}"),
            Forge::GitHub => "GitHub detected · review integration is not supported yet".into(),
            Forge::Unknown => error,
        }
    }

    /// What the files are compared against, for the title bar.
    pub fn compared_to(&self) -> String {
        let short = |sha: &str| sha.chars().take(8).collect::<String>();
        match &self.merge_request {
            Some(request) if self.main.is_empty() => {
                format!("MR !{} · {}", request.iid, short(&self.base))
            }
            _ => format!("merge-base {} · {}", self.main, short(&self.base)),
        }
    }

    /// Review files never change on disk; their version is the compared refs.
    pub fn snapshot(&self) -> Snapshot {
        let versions = self
            .files
            .iter()
            .map(|file| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                self.base.hash(&mut hash);
                self.head.hash(&mut hash);
                file.index.hash(&mut hash);
                (file.path.clone(), hash.finish())
            })
            .collect::<HashMap<_, _>>();
        Snapshot {
            branch: self.branch.clone(),
            files: self.files.clone(),
            versions,
        }
    }

    /// giv's `BuildReviewPreview`: both modes from one diff and one blob read.
    pub fn preview_pair(&self, repo: &Repository, file: &ChangedFile) -> Result<(Diff, Diff)> {
        let notice = |text: &str| {
            let diff = Diff {
                notice: Some(text.into()),
                ..Diff::default()
            };
            Ok((diff.clone(), diff))
        };
        if file.index == 'D' {
            return notice("File deleted in this branch.");
        }
        let binary_path = !preview::known_text(&file.path) && preview::binary_path(&file.path);
        if file.index == 'A' {
            if binary_path {
                return notice("Binary file — content not shown.");
            }
            let body = match blob(repo, &self.head, &file.path) {
                Ok(body) => body,
                Err(error) => return notice(&format!("read error: {error:#}")),
            };
            if !preview::known_text(&file.path) && preview::binary_content(&body) {
                return notice("Binary file — content not shown.");
            }
            // New files are entirely additions, numbered as GitLab positions them.
            let lines: Vec<_> = preview::full_context(&body)
                .into_iter()
                .map(|mut line| {
                    line.kind = LineKind::Added;
                    line.old = None;
                    line
                })
                .collect();
            let diff = Diff {
                additions: lines.len(),
                notice: Some(if lines.len() >= preview::MAX_LINES {
                    "New file — preview limited to 6,000 lines.".into()
                } else {
                    "New file — showing full content".into()
                }),
                lines,
                ..Diff::default()
            };
            return Ok((diff.clone(), diff));
        }

        let mut args: Vec<OsString> =
            ["diff", "-w", "--no-ext-diff", "--no-textconv", "--no-color"]
                .into_iter()
                .map(Into::into)
                .collect();
        if preview::known_text(&file.path) {
            args.push("--text".into());
        }
        args.extend([
            self.base.as_str().into(),
            self.head.as_str().into(),
            "--".into(),
            file.path.as_os_str().into(),
        ]);
        let raw = git::run(&repo.root, args, None, false).context("git diff")?;
        let diff = git::parse_diff(&String::from_utf8_lossy(&raw));
        let body = blob(repo, &self.head, &file.path);
        if diff.raw.trim().is_empty() {
            let Ok(body) = body else {
                return notice("No diff available.");
            };
            if !preview::known_text(&file.path) && preview::binary_content(&body) {
                return notice("Binary file — content not shown.");
            }
            let result = Diff {
                lines: preview::full_context(&body),
                notice: Some("No changes detected.".into()),
                ..Diff::default()
            };
            return Ok((result.clone(), result));
        }
        if binary_path || (!preview::known_text(&file.path) && preview::binary_notice(&diff.raw)) {
            return notice("Binary files differ — content not shown.");
        }
        let build = |show_deletions| {
            let mut result = diff.clone();
            match &body {
                Err(error) => {
                    result.lines = preview::fallback(&diff, show_deletions);
                    result.notice = Some(format!("read file: {error:#}"));
                }
                Ok(body) => {
                    if !preview::known_text(&file.path) && preview::binary_content(body) {
                        result.lines.clear();
                        result.notice = Some("Binary files differ — content not shown.".into());
                        return result;
                    }
                    result.lines = preview::overlay(body, &diff.raw, show_deletions)
                        .unwrap_or_else(|| preview::fallback(&diff, show_deletions));
                    result.notice = (result.lines.len() >= preview::MAX_LINES)
                        .then(|| "Preview limited to 6,000 lines.".into());
                }
            }
            result
        };
        Ok((build(false), build(true)))
    }
}

fn remotes(repo: &Repository) -> Result<Vec<String>> {
    let raw = git::run(&repo.root, ["remote"], None, false)?;
    Ok(String::from_utf8_lossy(&raw)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn resolves(repo: &Repository, name: &str) -> bool {
    git::run(
        &repo.root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
        None,
        false,
    )
    .is_ok()
}

/// The ref to compare locally and the source branch name on the forge.
/// `origin/feature` reviews that ref for the `feature` merge request; a name
/// that exists only remotely falls back to `origin/<name>`.
fn source_branch(repo: &Repository, branch: &str) -> Result<(String, String)> {
    if git::run(
        &repo.root,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
        None,
        false,
    )
    .is_ok()
    {
        return Ok((branch.to_owned(), branch.to_owned()));
    }
    for remote in remotes(repo)? {
        if let Some(source) = branch
            .strip_prefix(remote.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
            && resolves(repo, branch)
        {
            return Ok((branch.to_owned(), source.to_owned()));
        }
    }
    if !resolves(repo, branch) && resolves(repo, &format!("origin/{branch}")) {
        return Ok((format!("origin/{branch}"), branch.to_owned()));
    }
    Ok((branch.to_owned(), branch.to_owned()))
}

fn blob(repo: &Repository, rev: &str, path: &Path) -> Result<Vec<u8>> {
    let mut spec = OsString::from(format!("{rev}:"));
    spec.push(path.as_os_str());
    git::run(
        &repo.root,
        [OsString::from("cat-file"), "blob".into(), spec],
        None,
        false,
    )
}

/// Finds the open merge request for `source` and its latest collected diff
/// version, which is authoritative over the MR's possibly stale `diff_refs`.
fn load_merge_request(repo: &Repository, source: &str) -> Result<MergeRequest> {
    let query = format!(
        "projects/:fullpath/merge_requests?state=opened&source_branch={}",
        query_escape(source)
    );
    let requests = glab(repo, &["api", &query])?;
    let iid = requests
        .as_array()
        .context("decode merge requests")?
        .first()
        .with_context(|| format!("no open merge request for branch {source:?}"))?
        .get("iid")
        .and_then(serde_json::Value::as_u64)
        .context("decode merge requests")?;
    let path = format!("projects/:fullpath/merge_requests/{iid}");
    let request = glab(repo, &["api", &path])?;
    let text = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let refs = request.get("diff_refs").cloned().unwrap_or_default();
    let mut result = MergeRequest {
        iid,
        web_url: text(&request, "web_url"),
        base: text(&refs, "base_sha"),
        start: text(&refs, "start_sha"),
        head: text(&refs, "head_sha"),
    };
    let versions = glab(repo, &["api", &format!("{path}/versions")])?;
    let versions = versions
        .as_array()
        .context("decode merge request diff versions")?;
    if let Some(version) = versions.iter().find(|version| {
        text(version, "state") == "collected"
            && ["base_commit_sha", "start_commit_sha", "head_commit_sha"]
                .iter()
                .all(|key| !text(version, key).is_empty())
    }) {
        result.base = text(version, "base_commit_sha");
        result.start = text(version, "start_commit_sha");
        result.head = text(version, "head_commit_sha");
    }
    Ok(result)
}

fn glab(repo: &Repository, args: &[&str]) -> Result<serde_json::Value> {
    let program = find_program("glab").context("glab is not installed")?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&repo.root)
        .env("GLAB_NO_PROMPT", "1");
    let raw = git::execute(command, "glab", None, Duration::from_secs(60))
        .map_err(|error| anyhow::anyhow!("glab: {error}"))?;
    serde_json::from_slice(&raw).context("glab returned invalid JSON")
}

/// Apps started from the Dock do not inherit the shell PATH, so Homebrew
/// locations are checked too.
fn find_program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn query_escape(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
