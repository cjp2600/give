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

/// Updates the checked-out branch from its upstream without creating a merge commit.
pub fn pull(repo: &Repository) -> Result<()> {
    git::run(
        &repo.root,
        ["pull", "--ff-only", "--no-rebase", "--no-autostash"],
        None,
        true,
    )?;
    Ok(())
}
