//! Announce the bootstrap result as a herdr toast.
//!
//! herdr captures the plugin's stdout rather than printing it, so a run is
//! invisible unless someone goes digging with `herdr plugin log list`. A toast
//! is the only channel that reaches the user at the moment the worktree
//! appears, which is the moment they can still act on a half-bootstrapped one.

use serde::Deserialize;

use crate::Summary;
use crate::config::NotifyWhen;
use crate::herdr;

/// A notification, ready to hand to `herdr notification show`.
#[derive(Debug, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub body: String,
    /// One of herdr's sound names: `none`, `done`, `request`.
    pub sound: &'static str,
}

/// How the bootstrap ended.
pub enum Outcome<'a> {
    Succeeded(&'a Summary),
    Failed(&'a anyhow::Error),
}

/// Compose the toast for a finished run, or `None` to stay silent.
///
/// Kept free of I/O so the wording of every branch is testable without a herdr
/// server in the loop.
pub fn toast_for(when: NotifyWhen, branch: &str, outcome: Outcome<'_>) -> Option<Toast> {
    match (when, outcome) {
        (NotifyWhen::Never, _) => None,
        (NotifyWhen::Failure, Outcome::Succeeded(_)) => None,
        // Not worth interrupting anyone: this is every repo that has no config
        // at all, which would otherwise toast on every single worktree.
        (_, Outcome::Succeeded(summary)) if summary.is_empty() => None,
        (_, Outcome::Succeeded(summary)) => Some(Toast {
            title: format!("Worktree sync done · {branch}"),
            body: describe(summary),
            sound: "done",
        }),
        (_, Outcome::Failed(err)) => Some(Toast {
            title: format!("Worktree sync failed · {branch}"),
            // `{:#}` flattens anyhow's context chain onto one line, so the
            // toast carries the failing command and not just the outermost
            // "bootstrap failed".
            body: abbreviate(&format!("{err:#}"), TOAST_BODY_LINES),
            sound: "request",
        }),
    }
}

/// How many lines of a failure a toast gets. herdr's dialog truncates whatever
/// it cannot fit, and a truncation it performs is a truncation we don't control
/// — so the cut is made here, where the interesting lines can be kept.
/// [`crate::report`] has the untruncated version.
const TOAST_BODY_LINES: usize = 6;

/// Shorten a failure body to `max_lines`, keeping both ends.
///
/// Why not just the head or just the tail: the first line names the command
/// that failed and the last lines say why, and a package manager routinely puts
/// a dozen lines of progress between them.
fn abbreviate(body: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= max_lines {
        return body.to_string();
    }

    let mut kept = vec![lines[0], "…"];
    kept.extend_from_slice(&lines[lines.len() - (max_lines - 2)..]);
    kept.join("\n")
}

/// One line per phase that ran, in the order the phases execute.
fn describe(summary: &Summary) -> String {
    let mut lines = Vec::new();

    if summary.git_updated {
        lines.push("updated git".to_string());
    }
    match summary.copied {
        None => {}
        Some(0) => lines.push("copied no files".to_string()),
        Some(1) => lines.push("copied 1 file".to_string()),
        Some(n) => lines.push(format!("copied {n} files")),
    }
    match summary.linked {
        None => {}
        Some(0) => lines.push("linked no paths".to_string()),
        Some(1) => lines.push("linked 1 path".to_string()),
        Some(n) => lines.push(format!("linked {n} paths")),
    }
    match summary.installed.as_deref() {
        None => {}
        Some([]) => lines.push("no install rule matched".to_string()),
        Some(commands) => lines.extend(commands.iter().cloned()),
    }
    if summary.hooks_run > 0 {
        lines.push(format!("ran {} hooks", summary.hooks_run));
    }

    lines.join("\n")
}

/// Post the toast.
///
/// Never fails the bootstrap: herdr being unreachable, or refusing the request,
/// is worth a line in the log but not a worktree left unusable over a cosmetic
/// step that runs after all the real work is done.
pub fn show(toast: &Toast) {
    let output = herdr::command()
        .args(["notification", "show", &toast.title])
        .args(["--body", &toast.body])
        .args(["--sound", toast.sound])
        .output();

    match output {
        Err(err) => println!("[notify] could not run `herdr`: {err}"),
        Ok(output) if !output.status.success() => println!(
            "[notify] herdr rejected the toast: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Ok(output) => report_suppression(&output.stdout),
    }
}

#[derive(Deserialize)]
struct ShowResponse {
    result: ShowResult,
}

#[derive(Deserialize)]
struct ShowResult {
    shown: bool,
    #[serde(default)]
    reason: Option<String>,
}

/// herdr answers a suppressed toast with `shown: false` *and exit code 0*, so
/// without this the feature looks broken rather than switched off.
///
/// The reason is echoed verbatim because herdr owns that vocabulary and we
/// would only go stale guessing at it — `disabled` is the one we can act on
/// (toasts are off in herdr's default config, so it is the expected first
/// experience), while `busy` and friends are herdr deciding the user is
/// already looking at the thing.
fn report_suppression(stdout: &[u8]) {
    let Ok(response) = serde_json::from_slice::<ShowResponse>(stdout) else {
        return;
    };
    if response.result.shown {
        return;
    }

    let reason = response.result.reason.as_deref().unwrap_or("unknown");
    let hint = match reason {
        "disabled" => " — set `[ui.toast] delivery` in herdr's config.toml to see it",
        _ => "",
    };
    println!("[notify] herdr did not show the toast (reason: {reason}){hint}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> Summary {
        Summary {
            git_updated: true,
            copied: Some(3),
            linked: Some(2),
            installed: Some(vec!["pnpm install".to_string(), "uv sync".to_string()]),
            hooks_run: 2,
        }
    }

    fn body_of(when: NotifyWhen, summary: &Summary) -> String {
        toast_for(when, "wt/x", Outcome::Succeeded(summary))
            .expect("a non-empty summary should toast")
            .body
    }

    #[test]
    fn a_successful_run_lists_every_phase_in_execution_order() {
        let toast = toast_for(
            NotifyWhen::Always,
            "worktree/silver-stone",
            Outcome::Succeeded(&summary()),
        )
        .expect("a run that did work should toast");

        assert_eq!(toast.title, "Worktree sync done · worktree/silver-stone");
        assert_eq!(
            toast.body,
            "updated git\ncopied 3 files\nlinked 2 paths\npnpm install\nuv sync\nran 2 hooks"
        );
        assert_eq!(toast.sound, "done");
    }

    #[test]
    fn a_failure_carries_the_context_chain_and_the_attention_sound() {
        let err =
            anyhow::anyhow!("`pnpm install` exited with exit status: 1").context("install phase");
        let toast = toast_for(NotifyWhen::Always, "wt/x", Outcome::Failed(&err))
            .expect("failures always toast");

        assert_eq!(toast.title, "Worktree sync failed · wt/x");
        assert!(toast.body.contains("pnpm install"), "got: {}", toast.body);
        assert!(toast.body.contains("install phase"), "got: {}", toast.body);
        assert_eq!(toast.sound, "request");
    }

    /// The failure that prompted the capture: pnpm prints a wall of progress
    /// and puts the reason at the very bottom. Both ends have to survive, or
    /// the toast says either "something failed" or "…no such file" with no
    /// indication of what was running.
    #[test]
    fn a_long_failure_keeps_the_command_and_the_reason_around_the_cut() {
        let noise: Vec<String> = (1..=30).map(|n| format!("progress {n}")).collect();
        let err = anyhow::anyhow!(
            "`pnpm install` exited with exit status: 1\n{}\nERR_PNPM_ENOENT: no such file",
            noise.join("\n")
        );
        let body = toast_for(NotifyWhen::Always, "wt/x", Outcome::Failed(&err))
            .expect("failures always toast")
            .body;

        assert!(body.starts_with("`pnpm install` exited"), "got: {body}");
        assert!(
            body.ends_with("ERR_PNPM_ENOENT: no such file"),
            "got: {body}"
        );
        assert!(body.contains('…'), "the cut should be visible: {body}");
        assert_eq!(body.lines().count(), TOAST_BODY_LINES);
    }

    #[test]
    fn a_failure_short_enough_to_fit_is_left_alone() {
        let err = anyhow::anyhow!("`true` exited with exit status: 1\nboom");
        let body = toast_for(NotifyWhen::Always, "wt/x", Outcome::Failed(&err))
            .expect("failures always toast")
            .body;

        assert_eq!(body, "`true` exited with exit status: 1\nboom");
    }

    /// The two states worth telling apart: a phase that ran and found nothing
    /// is a likely misconfiguration, while a phase that never ran is not.
    #[test]
    fn a_phase_that_ran_and_found_nothing_still_reports() {
        let summary = Summary {
            copied: Some(0),
            installed: Some(vec![]),
            ..Summary::default()
        };
        assert_eq!(
            body_of(NotifyWhen::Always, &summary),
            "copied no files\nno install rule matched"
        );
    }

    #[test]
    fn a_run_with_no_phases_enabled_stays_silent() {
        let nothing = Summary::default();
        assert!(nothing.is_empty());
        assert!(toast_for(NotifyWhen::Always, "wt/x", Outcome::Succeeded(&nothing)).is_none());
    }

    #[test]
    fn failure_mode_keeps_quiet_about_success_but_not_about_errors() {
        let err = anyhow::anyhow!("boom");
        assert!(toast_for(NotifyWhen::Failure, "wt/x", Outcome::Succeeded(&summary())).is_none());
        assert!(toast_for(NotifyWhen::Failure, "wt/x", Outcome::Failed(&err)).is_some());
    }

    #[test]
    fn never_mode_is_silent_even_on_failure() {
        let err = anyhow::anyhow!("boom");
        assert!(toast_for(NotifyWhen::Never, "wt/x", Outcome::Succeeded(&summary())).is_none());
        assert!(toast_for(NotifyWhen::Never, "wt/x", Outcome::Failed(&err)).is_none());
    }

    #[test]
    fn one_copied_file_is_not_pluralised() {
        let summary = Summary {
            copied: Some(1),
            ..Summary::default()
        };
        assert_eq!(body_of(NotifyWhen::Always, &summary), "copied 1 file");
    }
}
