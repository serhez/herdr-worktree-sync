//! Entry point: parse the herdr event payload, locate the repo's config, and
//! hand off to [`herdr_worktree_sync::run`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use herdr_worktree_sync::config::OnFailure;
use herdr_worktree_sync::notify::{self, Outcome};
use herdr_worktree_sync::{config, event::Event, report, rollback, run};

fn main() -> Result<()> {
    if std::env::var("HERDR_PLUGIN_ACTION_ID").as_deref() == Ok("sync") {
        return sync_files_action();
    }
    created_event()
}

fn sync_files_action() -> Result<()> {
    let context = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .context("HERDR_PLUGIN_CONTEXT_JSON is not set")?;
    let plugin_config = config::PluginConfig::load_from_env()?;
    let summary = herdr_worktree_sync::action::sync(&context, &plugin_config)?;
    if let Some(toast) = notify::toast_for(
        config::NotifyWhen::Always,
        "file sync",
        Outcome::Succeeded(&summary),
    ) {
        notify::show(&toast);
    }
    println!("[sync] done");
    Ok(())
}

fn created_event() -> Result<()> {
    let event_json =
        std::env::var("HERDR_PLUGIN_EVENT_JSON").context("HERDR_PLUGIN_EVENT_JSON is not set")?;
    let event: Event =
        serde_json::from_str(&event_json).context("failed to parse HERDR_PLUGIN_EVENT_JSON")?;

    let worktree = Path::new(&event.data.worktree.path);
    println!("[worktree-sync] worktree: {}", worktree.display());
    // A detached worktree carries no branch at all. Print a placeholder rather
    // than dropping the line: these logs are the only window into a run, and a
    // missing line reads as "the plugin never got that far".
    let branch = event
        .data
        .worktree
        .branch
        .as_deref()
        .unwrap_or("(detached)");
    println!("[worktree-sync] branch:   {branch}");

    let source = event
        .data
        .workspace
        .worktree
        .as_ref()
        .map(|w| PathBuf::from(&w.repo_root));
    if let Some(src) = &source {
        println!("[worktree-sync] source:   {}", src.display());
    }

    // Config is owned by the repo: read the configured path from the source
    // repo (falls back to the new worktree, which has the same committed copy).
    let config_dir = source.as_deref().unwrap_or(worktree);
    let plugin_config = config::PluginConfig::load_from_env()?;
    let config =
        config::load(config_dir, &plugin_config).context("failed to load worktree sync config")?;

    // The toast is posted for both outcomes before the error propagates, so a
    // failed bootstrap is the one the user hears about rather than the one
    // that vanishes into the captured log.
    let outcome = run(worktree, source.as_deref(), &config);
    let toast = match &outcome {
        Ok(summary) => notify::toast_for(config.notify.when, branch, Outcome::Succeeded(summary)),
        Err(err) => notify::toast_for(config.notify.when, branch, Outcome::Failed(err)),
    };
    if let Some(toast) = toast {
        notify::show(&toast);
    }

    if let Err(err) = &outcome {
        // Report before rollback, in that order: removing the workspace takes
        // this pane's siblings with it, and a pane opened afterwards would have
        // nothing to attach to. The report file outlives both either way.
        report::show_failure(branch, worktree, err);

        if config.failure.action == OnFailure::Remove {
            rollback::remove_worktree(
                event.data.workspace.workspace_id.as_deref(),
                event.data.worktree.branch.as_deref(),
                source.as_deref(),
            );
        }
    }
    outcome?;

    println!("[worktree-sync] done");
    Ok(())
}
