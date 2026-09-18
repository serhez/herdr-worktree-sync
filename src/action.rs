//! Manual plugin actions.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::{self, PluginConfig};
use crate::{Summary, sync_files};

#[derive(Deserialize)]
struct InvocationContext {
    worktree: Option<InvocationWorktree>,
}

#[derive(Deserialize)]
struct InvocationWorktree {
    repo_root: String,
    checkout_path: String,
    is_linked_worktree: bool,
}

/// Re-apply file operations to the linked worktree in Herdr's invocation
/// context. Setup commands and hooks deliberately do not run during a sync.
pub fn sync(context_json: &str, plugin_config: &PluginConfig) -> Result<Summary> {
    let context: InvocationContext =
        serde_json::from_str(context_json).context("failed to parse HERDR_PLUGIN_CONTEXT_JSON")?;
    let worktree = context
        .worktree
        .context("the focused workspace is not associated with a git worktree")?;
    if !worktree.is_linked_worktree || worktree.repo_root == worktree.checkout_path {
        bail!("file sync requires a linked worktree, not the primary checkout");
    }

    let source = Path::new(&worktree.repo_root);
    let destination = Path::new(&worktree.checkout_path);
    let repo_config =
        config::load(source, plugin_config).context("failed to load worktree sync config")?;
    sync_files(destination, Some(source), &repo_config)
}
