//! Working-tree branch operations. Git owns conflict checks; no force or autostash.
use crate::{
    git::{self, Repository},
    review::Branch,
};
use anyhow::{Context, Result, bail};

/// Switches a local branch, or creates/reuses a local tracking branch for a remote ref.
pub fn switch(repo: &Repository, branch: &Branch) -> Result<String> {
    let Some(remote) = &branch.remote else {
        let name = branch
            .reference
            .strip_prefix("refs/heads/")
            .context("Not a local branch")?;
        git::run(&repo.root, ["switch", "--no-guess", "--", name], None, true)?;
        return Ok(name.to_owned());
    };
    let prefix = format!("refs/remotes/{remote}/");
    let name = branch
        .reference
        .strip_prefix(&prefix)
        .context("Not a remote branch")?;
    // Use full refs so a same-named local branch or tag cannot redirect checkout.
    let local_ref = format!("refs/heads/{name}");
    let locals = git::run(
        &repo.root,
        [
            "for-each-ref",
            "--format=%(refname)%00%(upstream)",
            "refs/heads",
        ],
        None,
        false,
    )?;
    for line in String::from_utf8_lossy(&locals).lines() {
        let Some((reference, upstream)) = line.split_once('\0') else {
            continue;
        };
        if reference == local_ref {
            if upstream != branch.reference {
                bail!(
                    "Local branch '{name}' already exists and does not track '{}'. Choose the local branch or rename it first.",
                    branch.name
                );
            }
            git::run(&repo.root, ["switch", "--no-guess", "--", name], None, true)?;
            return Ok(name.to_owned());
        }
    }
    git::run(
        &repo.root,
        ["switch", "--track", "-c", name, "--", &branch.reference],
        None,
        true,
    )?;
    Ok(name.to_owned())
}

/// The checked-out local branch; detached HEAD cannot be pulled or published.
fn current(repo: &Repository) -> Result<String> {
    let raw = git::run(&repo.root, ["symbolic-ref", "--quiet", "HEAD"], None, false)
        .context("Choose a local branch first; HEAD is detached")?;
    let reference = String::from_utf8(raw)?;
    Ok(reference
        .trim_end()
        .strip_prefix("refs/heads/")
        .context("HEAD does not point to a local branch")?
        .to_owned())
}

pub fn remotes(repo: &Repository) -> Result<Vec<String>> {
    let raw = git::run(&repo.root, ["remote"], None, false)?;
    Ok(String::from_utf8(raw)?.lines().map(str::to_owned).collect())
}

/// Updates from the configured upstream, or discovers a unique same-named branch.
pub fn pull(repo: &Repository) -> Result<()> {
    let name = current(repo)?;
    // Read config rather than resolving @{upstream}: its remote ref may not exist
    // until the first fetch. Never replace an explicitly configured upstream.
    let configured = git::run(
        &repo.root,
        ["config", "--get", &format!("branch.{name}.merge")],
        None,
        false,
    );
    if configured.is_ok() {
        git::run(
            &repo.root,
            ["pull", "--ff-only", "--no-rebase", "--no-autostash"],
            None,
            true,
        )?;
        return Ok(());
    }
    crate::review::fetch(repo)?;
    let candidates: Vec<_> = crate::review::branches(repo)?
        .into_iter()
        .filter(|branch| {
            branch
                .remote
                .as_ref()
                .is_some_and(|remote| branch.reference == format!("refs/remotes/{remote}/{name}"))
        })
        .collect();
    if candidates.len() != 1 {
        bail!(
            "No upstream tracking branch for '{name}'. Use ‘Pull from…’ to choose a remote branch, or ‘Push current branch…’ to publish it."
        );
    }
    pull_from(repo, &candidates[0])
}

/// Pull an explicitly chosen remote branch and remember it after a successful update.
pub fn pull_from(repo: &Repository, branch: &Branch) -> Result<()> {
    let name = current(repo)?;
    let remote = branch
        .remote
        .as_ref()
        .context("Choose a remote branch to pull from")?;
    let source = branch
        .reference
        .strip_prefix(&format!("refs/remotes/{remote}/"))
        .context("Not a remote branch")?;
    let source_ref = format!("refs/heads/{source}");
    git::run(
        &repo.root,
        [
            "pull",
            "--ff-only",
            "--no-rebase",
            "--no-autostash",
            "--",
            remote,
            &source_ref,
        ],
        None,
        true,
    )?;
    git::run(
        &repo.root,
        [
            "branch",
            "--set-upstream-to",
            &branch.reference,
            "--",
            &name,
        ],
        None,
        false,
    )
    .context("Pull succeeded, but upstream tracking could not be saved")?;
    Ok(())
}

/// Publish only the current branch to the selected remote, with the same name.
pub fn push(repo: &Repository, remote: &str) -> Result<String> {
    let name = current(repo)?;
    if !remotes(repo)?.iter().any(|configured| configured == remote) {
        bail!("Remote '{remote}' is not configured");
    }
    let reference = format!("refs/heads/{name}:refs/heads/{name}");
    git::run(
        &repo.root,
        [
            "push",
            "--no-force",
            "--set-upstream",
            "--",
            remote,
            &reference,
        ],
        None,
        true,
    )?;
    Ok(format!(
        "Pushed {name} → {remote}/{name} · upstream tracking configured"
    ))
}

#[derive(Debug)]
pub enum CommitPushOutcome {
    Pushed(String),
    PushFailed(String),
}

/// Commit the existing index, then publish the current branch. Resolve the
/// destination first so a missing/ambiguous remote does not create a commit.
/// A push failure is a completed local commit, not a failed commit to retry.
pub fn commit_and_push(repo: &Repository, message: &str) -> Result<CommitPushOutcome> {
    let name = current(repo)?;
    let remotes = remotes(repo)?;
    let configured = [
        format!("branch.{name}.pushRemote"),
        "remote.pushDefault".to_owned(),
        format!("branch.{name}.remote"),
    ]
    .into_iter()
    .find_map(|key| {
        git::run(&repo.root, ["config", "--get", &key], None, false)
            .ok()
            .map(|raw| String::from_utf8_lossy(&raw).trim().to_owned())
    });
    let remote = match configured {
        Some(remote) if remotes.contains(&remote) => remote,
        Some(remote) => {
            bail!("Push remote '{remote}' is not configured. Choose a remote in the branch menu.")
        }
        None if remotes.len() == 1 => remotes[0].clone(),
        None if remotes.iter().any(|remote| remote == "origin") => "origin".to_owned(),
        None => {
            bail!("No default push remote. Choose a remote with ‘Push…’ in the branch menu first.")
        }
    };
    repo.commit(message)?;
    Ok(match push(repo, &remote) {
        Ok(message) => CommitPushOutcome::Pushed(message),
        Err(error) => CommitPushOutcome::PushFailed(format!("{error:#}")),
    })
}

/// Merge a selected full branch ref into the checked-out branch.
pub fn merge(repo: &Repository, branch: &Branch) -> Result<String> {
    let target = current(repo)?;
    if branch.reference == format!("refs/heads/{target}") {
        bail!("Choose another branch to merge into '{target}'");
    }
    if !repo.snapshot()?.files.is_empty() {
        bail!("Commit or stash your changes before merging into '{target}'");
    }
    if git::run(
        &repo.root,
        ["rev-parse", "--verify", "--quiet", "MERGE_HEAD"],
        None,
        false,
    )
    .is_ok()
    {
        bail!("A merge is already in progress. Finish it or use ‘Abort merge’ first.");
    }
    let result = git::run(
        &repo.root,
        [
            "merge",
            "--no-edit",
            "--no-autostash",
            "--ff",
            "--no-squash",
            "--commit",
            "--",
            &branch.reference,
        ],
        None,
        true,
    );
    if let Err(error) = result {
        let raw = git::run(
            &repo.root,
            ["diff", "--name-only", "--diff-filter=U", "-z"],
            None,
            false,
        )?;
        let conflicts: Vec<_> = raw
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect();
        if !conflicts.is_empty() {
            bail!(
                "Merge conflict: {} → {target}\n{}\nResolve the files, stage them and commit, or use ‘Abort merge’.",
                branch.name,
                conflicts.join("\n")
            );
        }
        return Err(error);
    }
    Ok(format!("Merged {} → {target}", branch.name))
}

pub fn abort_merge(repo: &Repository) -> Result<()> {
    git::run(&repo.root, ["merge", "--abort"], None, true)?;
    Ok(())
}
