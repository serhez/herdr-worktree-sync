//! Bootstrap a freshly created git worktree: copy gitignored files, install
//! dependencies, and run per-repo hooks.
//!
//! The binary (`herdr-worktree-sync`) is a thin wrapper that parses the herdr
//! event payload and calls [`run`]. Everything that decides *what happens* lives
//! here so it can be exercised from tests without herdr in the loop.

pub mod action;
pub mod bootstrap;
pub mod config;
pub mod event;
pub mod herdr;
pub mod notify;
pub mod report;
pub mod rollback;

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::Config;

/// What a [`run`] actually did, for the end-of-run notification.
///
/// The file-operation and install fields are `Option` so that "the phase was
/// disabled" stays distinguishable from "the phase ran and found nothing".
/// Only the second is worth reporting, and it is the harder of the two to
/// diagnose from the outside — it looks identical to a phase that never ran.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub git_updated: bool,
    /// Files copied, when the copy phase ran.
    pub copied: Option<usize>,
    /// Files cloned, when the APFS clone phase ran.
    pub cloned: Option<usize>,
    /// Paths linked, when the symlink phase ran.
    pub linked: Option<usize>,
    /// The command run in each install directory, when the install phase ran.
    pub installed: Option<Vec<String>>,
    /// Pre and post hooks together.
    pub hooks_run: usize,
}

impl Summary {
    /// No phase was enabled, so there is nothing to report.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Run the whole bootstrap lifecycle against `worktree`.
///
/// Phases run in a fixed order — **git update → pre hooks → copy → clone →
/// symlink → install → post hooks** — and the first failure aborts the rest
/// (fail-fast), leaving the worktree partially bootstrapped rather than silently
/// continuing past a broken step. What becomes of that worktree afterwards is
/// the caller's decision, via [`config::OnFailure`].
///
/// `source` is the repo the worktree was derived from. It is only needed by the
/// copy, clone, and symlink phases; the other phases operate entirely inside
/// `worktree`.
pub fn run(worktree: &Path, source: Option<&Path>, config: &Config) -> Result<Summary> {
    let mut summary = Summary::default();

    // Phase 0: bring git up to date, before anything else runs.
    if config.git.update {
        bootstrap::git_update(worktree, config.git.command.as_deref())?;
        summary.git_updated = true;
    }

    summary.hooks_run += bootstrap::run_hooks(worktree, &config.hooks.pre)?;

    let file_summary = sync_files(worktree, source, config)?;
    summary.copied = file_summary.copied;
    summary.cloned = file_summary.cloned;
    summary.linked = file_summary.linked;

    if config.install.enabled {
        summary.installed = Some(bootstrap::install_deps(
            worktree,
            &config.install.rules,
            config.install.dirs.as_deref(),
        )?);
    }

    // Post hooks: run last, after file operations and install.
    summary.hooks_run += bootstrap::run_hooks(worktree, &config.hooks.post)?;

    Ok(summary)
}

/// Apply only the declared copy, clone, and symlink operations. This is shared
/// by the creation lifecycle and the manual `sync` plugin action.
pub fn sync_files(worktree: &Path, source: Option<&Path>, config: &Config) -> Result<Summary> {
    let mut summary = Summary::default();
    if config.copy.enabled || config.clone.enabled || config.symlink.enabled {
        let src = source.context("file operations are enabled but there is no source repo_root")?;
        if src == worktree {
            anyhow::bail!("file operations require a linked worktree, not the primary checkout");
        }

        if config.copy.enabled {
            summary.copied = Some(match config.copy.files.as_deref() {
                Some(files) => bootstrap::copy_files(src, worktree, files)?,
                None => bootstrap::copy_gitignored(src, worktree, config.copy.patterns.as_deref())?,
            });
        }
        if config.clone.enabled {
            summary.cloned = Some(bootstrap::clone_files(src, worktree, &config.clone.files)?);
        }
        if config.symlink.enabled {
            summary.linked = Some(bootstrap::symlink_files(
                src,
                worktree,
                &config.symlink.files,
            )?);
        }
    }
    Ok(summary)
}
