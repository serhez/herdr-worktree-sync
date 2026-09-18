//! Deserialization of the `HERDR_PLUGIN_EVENT_JSON` payload.
//!
//! This is the one input the plugin does not control, and a payload that fails
//! to parse aborts the whole bootstrap before any phase runs — with the error
//! going to herdr's plugin log rather than to a terminal, so nobody notices
//! until they wonder why the worktree is empty. The payloads below are shaped
//! after herdr's own `EventEnvelope`, so a field that herdr marks optional is
//! pinned here as optional too.

use herdr_worktree_sync::event::Event;

/// A `worktree.created` payload. `branch` and `workspace.worktree` are passed
/// as raw JSON fragments so a test can omit either key entirely — which is what
/// herdr does for its `skip_serializing_if = "Option::is_none"` fields, rather
/// than emitting `null`.
fn payload(branch: &str, source_worktree: &str) -> String {
    format!(
        r#"{{
          "event": "worktree.created",
          "data": {{
            "type": "worktree.created",
            "workspace": {{
              "workspace_id": "ws-1", "number": 1, "label": "feature",
              "focused": true, "pane_count": 1, "tab_count": 1,
              "active_tab_id": "tab-1", "agent_status": "idle"
              {source_worktree}
            }},
            "worktree": {{
              "path": "/tmp/wt", {branch}
              "is_bare": false, "is_detached": false, "is_prunable": false,
              "is_linked_worktree": true, "label": "feature"
            }}
          }}
        }}"#
    )
}

const SOURCE: &str = r#", "worktree": {
    "repo_key": "k", "repo_name": "repo", "repo_root": "/src/repo",
    "checkout_path": "/tmp/wt", "is_linked_worktree": true
}"#;

#[test]
fn a_worktree_on_a_branch_reports_that_branch() {
    let event: Event = serde_json::from_str(&payload(r#""branch": "feature/x","#, SOURCE))
        .expect("a branch payload should parse");

    assert_eq!(event.data.worktree.path, "/tmp/wt");
    assert_eq!(event.data.worktree.branch.as_deref(), Some("feature/x"));
}

#[test]
fn a_detached_worktree_omits_the_branch_key_entirely() {
    // herdr skips the key rather than sending null, so a required `branch`
    // field fails with "missing field `branch`" and kills the whole run.
    let event: Event = serde_json::from_str(&payload("", SOURCE))
        .expect("a detached payload has no branch key and must still parse");

    assert_eq!(event.data.worktree.branch, None);
    assert_eq!(event.data.worktree.path, "/tmp/wt");
}

#[test]
fn the_source_repo_root_is_read_off_the_workspace_not_the_worktree() {
    let event: Event = serde_json::from_str(&payload(r#""branch": "main","#, SOURCE))
        .expect("payload should parse");

    let source = event.data.workspace.worktree.expect("a source worktree");
    assert_eq!(source.repo_root, "/src/repo");
}

#[test]
fn a_workspace_with_no_source_worktree_still_parses() {
    let event: Event = serde_json::from_str(&payload(r#""branch": "main","#, ""))
        .expect("a workspace without a source worktree must still parse");

    assert!(event.data.workspace.worktree.is_none());
}

#[test]
fn fields_the_plugin_does_not_know_about_are_ignored() {
    // herdr adds fields to these structs between releases, and the plugin
    // declares a minimum herdr version, not a maximum. Rejecting unknown keys
    // here would break the plugin on every herdr upgrade.
    let event: Event = serde_json::from_str(&payload(
        r#""branch": "main", "some_future_herdr_field": {"nested": [1, 2]},"#,
        SOURCE,
    ))
    .expect("unknown fields must not fail the parse");

    assert_eq!(event.data.worktree.branch.as_deref(), Some("main"));
}
