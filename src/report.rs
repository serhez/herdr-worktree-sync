//! Put the full failure text somewhere it can actually be read.
//!
//! A toast is a few lines wide and herdr's own dialog truncates what it cannot
//! fit, so the line that says *why* a bootstrap failed is routinely the line
//! that gets cut. This module writes the whole thing to a file and asks herdr
//! to open a plugin pane on it — a real terminal, scrollable, as wide as the
//! window.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::herdr;

/// Entrypoint id of the `[[panes]]` block in `herdr-plugin.toml`. Public so
/// `tests/manifest.rs` can hold the manifest to it — herdr resolves the two by
/// string at runtime and nothing else would notice them drifting apart.
pub const PANE_ENTRYPOINT: &str = "failure";

/// Env var the pane entrypoint reads the report path from. The pane command is
/// fixed in the manifest, so this is the only channel for telling it which file
/// to show — and, like the entrypoint id, a name that matches only by
/// convention until `tests/manifest.rs` checks it.
pub const REPORT_PATH_VAR: &str = "WORKTREE_SYNC_REPORT";

/// Write the report and open a pane on it.
///
/// Never fails the bootstrap: the run has already failed, and a missing state
/// directory or a herdr that declines the pane should not replace the real
/// error with a worse one. The report path is logged either way, so the file
/// stays reachable even when the pane never appears.
pub fn show_failure(branch: &str, worktree: &Path, err: &anyhow::Error) {
    let body = compose(branch, worktree, err);
    match write_report(branch, &body) {
        Err(write_err) => println!("[report] could not write the failure report: {write_err:#}"),
        Ok(path) => {
            println!("[report] full failure report: {}", path.display());
            open_pane(&path);
        }
    }
}

/// The report file's contents: what failed, where, and the full error chain.
///
/// Kept free of I/O so the wording is testable without a herdr server or a
/// writable state directory in the loop.
fn compose(branch: &str, worktree: &Path, err: &anyhow::Error) -> String {
    let mut body = format!(
        "Worktree sync failed\n\nbranch:   {branch}\nworktree: {}\n\n",
        worktree.display()
    );
    // One cause per line rather than anyhow's `{:#}`, which joins them with
    // ": " — the innermost cause here is a command's captured output, and
    // folding its newlines into one line is exactly the truncation this file
    // exists to avoid.
    for cause in err.chain() {
        body.push_str(&cause.to_string());
        body.push('\n');
    }
    body
}

/// Where the plugin may keep files between runs. herdr hands this out per
/// plugin; the temp dir is the fallback for running the binary by hand.
fn state_dir() -> PathBuf {
    match std::env::var_os("HERDR_PLUGIN_STATE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::temp_dir(),
    }
}

/// Write `body` to the plugin's state directory, returning where it landed.
///
/// One file per branch, overwritten on each run: reports are only interesting
/// until the worktree is fixed or removed, and a timestamped name would grow a
/// directory nobody ever prunes.
fn write_report(branch: &str, body: &str) -> Result<PathBuf> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let path = dir.join(report_file_name(branch));
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// A file name for `branch`. Branch names contain `/` (and herdr's own default
/// is `worktree/<name>`), which would otherwise be read as a subdirectory that
/// does not exist.
fn report_file_name(branch: &str) -> String {
    let safe: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("failure-{safe}.log")
}

/// Remembers the pane id from the last failure, so the next one can close it.
/// See [`close_stale_pane`] for why that is necessary.
const PANE_ID_FILE: &str = "failure-pane-id";

/// herdr's answer to `plugin pane open`, trimmed to the id we need back.
#[derive(serde::Deserialize)]
struct OpenResponse {
    result: OpenResult,
}

#[derive(serde::Deserialize)]
struct OpenResult {
    plugin_pane: PluginPane,
}

#[derive(serde::Deserialize)]
struct PluginPane {
    pane: PaneInfo,
}

#[derive(serde::Deserialize)]
struct PaneInfo {
    pane_id: String,
}

/// Ask herdr to open the plugin's failure pane on `path`.
fn open_pane(path: &Path) {
    let Some(plugin_id) = std::env::var_os("HERDR_PLUGIN_ID") else {
        println!("[report] HERDR_PLUGIN_ID is not set, cannot open the failure pane");
        return;
    };

    let dir = state_dir();
    close_stale_pane(&dir);

    let output = herdr::command()
        .args(["plugin", "pane", "open", "--plugin"])
        .arg(&plugin_id)
        .args(["--entrypoint", PANE_ENTRYPOINT])
        // Overlay, and focused: the pane is the reason the user was just
        // interrupted, so burying it behind the agent would defeat the point.
        .args(["--placement", "overlay"])
        .arg("--env")
        .arg(format!("{REPORT_PATH_VAR}={}", path.display()))
        .output();

    match output {
        Err(err) => println!("[report] could not run `herdr`: {err}"),
        Ok(output) if !output.status.success() => println!(
            "[report] herdr declined to open the failure pane: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Ok(output) => remember_pane(&dir, &output.stdout),
    }
}

/// Close the pane the *previous* failure opened, if it is still up.
///
/// Why not just open another one: herdr hands back the existing pane for a
/// given entrypoint rather than opening a second, and a reused pane keeps the
/// `less` process — and therefore the report path — it was started with. Two
/// failures in a row would leave the user staring at the first one's error
/// while being told it describes the second, which is worse than no pane.
///
/// Every step here is best-effort: the id can name a pane the user already
/// closed, and that is the common case rather than an anomaly.
fn close_stale_pane(dir: &Path) {
    let marker = dir.join(PANE_ID_FILE);
    let Ok(pane_id) = std::fs::read_to_string(&marker) else {
        return;
    };
    let _ = std::fs::remove_file(&marker);

    let _ = herdr::command()
        .args(["plugin", "pane", "close", pane_id.trim()])
        .output();
}

/// Record the pane herdr just opened, for the next failure to clean up.
fn remember_pane(dir: &Path, stdout: &[u8]) {
    let Ok(response) = serde_json::from_slice::<OpenResponse>(stdout) else {
        println!("[report] could not read the pane id out of herdr's reply");
        return;
    };
    let path = dir.join(PANE_ID_FILE);
    if let Err(err) = std::fs::write(&path, &response.result.plugin_pane.pane.pane_id) {
        println!(
            "[report] could not record the pane id in {}: {err}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> anyhow::Error {
        anyhow::anyhow!("`pnpm install` exited with exit status: 1\nERR_PNPM_ENOENT\nreflink")
            .context("install phase")
    }

    /// The report exists to carry the multi-line command output that a toast
    /// cannot: every line of it has to survive, on its own line.
    #[test]
    fn the_report_keeps_the_command_output_line_by_line() {
        let body = compose("worktree/x", Path::new("/tmp/wt"), &failure());

        assert!(body.contains("\nbranch:   worktree/x\n"), "got: {body}");
        assert!(body.contains("\nworktree: /tmp/wt\n"), "got: {body}");
        assert!(body.contains("\ninstall phase\n"), "got: {body}");
        assert!(body.contains("\nERR_PNPM_ENOENT\nreflink\n"), "got: {body}");
    }

    /// A real `herdr plugin pane open` reply. The pane id is buried three
    /// levels down, and reading it back is the only thing that lets the *next*
    /// failure close this pane instead of inheriting its stale report.
    #[test]
    fn the_pane_id_is_read_out_of_herdrs_reply() {
        let reply = br#"{"id":"cli:plugin","result":{"plugin_pane":{
            "entrypoint":"failure",
            "pane":{"agent_status":"unknown","cwd":"/repo","focused":true,
                    "label":"Worktree sync failure","pane_id":"w3T:pG",
                    "terminal_id":"term_65b9","workspace_id":"w3T"},
            "plugin_id":"serhez.herdr.worktree.sync"},
            "type":"plugin_pane_opened"}}"#;

        let response: OpenResponse =
            serde_json::from_slice(reply).expect("herdr's reply should parse");
        assert_eq!(response.result.plugin_pane.pane.pane_id, "w3T:pG");
    }

    #[test]
    fn a_slashed_branch_name_does_not_become_a_subdirectory() {
        assert_eq!(
            report_file_name("worktree/silver-stone"),
            "failure-worktree-silver-stone.log"
        );
        assert!(!report_file_name("feature/a/b").contains('/'));
    }
}
