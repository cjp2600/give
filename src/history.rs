//! Read-only history of one reachable branch tip, with immutable commit details.
use crate::git::{self, Repository};
use anyhow::{Context, Result, ensure};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: String,
    pub subject: String,
    pub references: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub status: char,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub commits: Vec<Commit>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct Details {
    pub message: String,
    pub files: Vec<File>,
}

/// `limit` grows as older pages are requested. An unborn HEAD has no commits.
pub fn load(repo: &Repository, revision: &str, limit: usize) -> Result<Page> {
    let resolved = git::run(
        &repo.root,
        [
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
        None,
        false,
    );
    let tip = match resolved {
        Ok(bytes) => String::from_utf8(bytes)?.trim().to_owned(),
        Err(error) => {
            // Distinguish an unborn branch from an invalid ref or broken repository.
            if revision == "HEAD"
                && let Ok(reference) =
                    git::run(&repo.root, ["symbolic-ref", "--quiet", "HEAD"], None, false)
            {
                let reference = String::from_utf8(reference)?.trim().to_owned();
                let refs = git::run(
                    &repo.root,
                    ["for-each-ref", "--format=%(refname)", &reference],
                    None,
                    false,
                )?;
                if refs.is_empty() {
                    return Ok(Page::default());
                }
            }
            return Err(error);
        }
    };
    let count = format!("--max-count={}", limit.saturating_add(1));
    let raw = git::run(
        &repo.root,
        [
            "log",
            "--topo-order",
            "--no-color",
            "--decorate=short",
            "-z",
            &count,
            "--format=%H%x00%P%x00%aN%x00%aE%x00%ai%x00%s%x00%D",
            &tip,
            "--",
        ],
        None,
        false,
    )?;
    let fields: Vec<_> = raw.split(|byte| *byte == 0).collect();
    ensure!(
        fields.last() == Some(&&b""[..]) && (fields.len() - 1).is_multiple_of(7),
        "Invalid Git log output"
    );
    let mut commits = Vec::new();
    for fields in fields[..fields.len() - 1].chunks_exact(7) {
        let text = |i: usize| String::from_utf8_lossy(fields[i]).into_owned();
        commits.push(Commit {
            id: text(0),
            parents: text(1).split_whitespace().map(str::to_owned).collect(),
            author: text(2),
            email: text(3),
            date: text(4),
            subject: text(5),
            references: text(6),
        });
    }
    let has_more = commits.len() > limit;
    commits.truncate(limit);
    Ok(Page { commits, has_more })
}

/// Merge files are compared to the first parent, including conflict resolutions.
pub fn details(repo: &Repository, commit: &Commit) -> Result<Details> {
    ensure!(
        commit.id.len() >= 40 && commit.id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid commit ID"
    );
    let message = git::run(
        &repo.root,
        ["show", "--no-patch", "--format=%B", &commit.id, "--"],
        None,
        false,
    )?;
    let mut args = vec![
        "diff-tree",
        "--no-commit-id",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status",
        "-z",
        "-r",
        "--find-renames",
        "--root",
    ];
    if let Some(parent) = commit.parents.first() {
        args.push(parent);
    }
    args.extend([&commit.id, "--"]);
    let raw = git::run(&repo.root, args, None, false)?;
    let mut fields = raw.split(|b| *b == 0).filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let status = status[0] as char;
        let first = fields.next().context("Missing commit file path")?;
        let (original_path, path) = if matches!(status, 'R' | 'C') {
            (
                Some(git::path_from_bytes(first)?),
                fields.next().context("Missing rename destination")?,
            )
        } else {
            (None, first)
        };
        files.push(File {
            path: git::path_from_bytes(path)?,
            original_path,
            status,
        });
    }
    Ok(Details {
        message: String::from_utf8_lossy(&message).trim_end().to_owned(),
        files,
    })
}
