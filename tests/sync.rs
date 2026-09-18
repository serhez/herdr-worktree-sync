//! Manual file synchronization uses Herdr's action context to identify the
//! primary checkout and the focused linked worktree.

mod common;

use common::Dir;
use herdr_worktree_sync::{action, config::PluginConfig};

#[test]
fn sync_reapplies_copy_and_symlink_operations_to_the_focused_worktree() {
    let source = Dir::new();
    source.write(
        ".worktree-sync.toml",
        r#"
        [copy]
        enabled = true
        files = [".env"]

        [symlink]
        enabled = true
        files = ["cache"]
        "#,
    );
    source.write(".env", "ENV");
    source.write("cache/data", "CACHE");
    let worktree = Dir::new();
    let context = serde_json::json!({
        "worktree": {
            "repo_root": source.path(),
            "checkout_path": worktree.path(),
            "is_linked_worktree": true
        }
    });

    let summary =
        action::sync(&context.to_string(), &PluginConfig::default()).expect("sync should succeed");

    assert_eq!(summary.copied, Some(1));
    assert_eq!(summary.linked, Some(1));
    assert_eq!(worktree.read(".env"), "ENV");
    assert_eq!(worktree.read("cache/data"), "CACHE");
    assert!(worktree.path().join("cache").is_symlink());
}

#[cfg(target_os = "macos")]
#[test]
fn sync_reapplies_clone_operations_to_the_focused_worktree() {
    let source = Dir::new();
    source.write(
        ".worktree-sync.toml",
        r#"
        [clone]
        enabled = true
        files = ["models/weights.bin"]
        "#,
    );
    source.write("models/weights.bin", "WEIGHTS");
    let worktree = Dir::new();
    let context = serde_json::json!({
        "worktree": {
            "repo_root": source.path(),
            "checkout_path": worktree.path(),
            "is_linked_worktree": true
        }
    });

    let summary =
        action::sync(&context.to_string(), &PluginConfig::default()).expect("sync should succeed");

    assert_eq!(summary.cloned, Some(1));
    assert_eq!(worktree.read("models/weights.bin"), "WEIGHTS");
}

#[test]
fn sync_refuses_to_apply_file_operations_to_the_primary_checkout() {
    let source = Dir::new();
    let context = serde_json::json!({
        "worktree": {
            "repo_root": source.path(),
            "checkout_path": source.path(),
            "is_linked_worktree": false
        }
    });

    let err = action::sync(&context.to_string(), &PluginConfig::default())
        .expect_err("syncing the primary checkout could overwrite its source files");

    assert!(err.to_string().contains("linked worktree"), "got: {err}");
}
