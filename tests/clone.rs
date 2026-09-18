//! Copy-on-write clones from the primary checkout into a linked worktree.

mod common;

use common::Dir;
use herdr_worktree_sync::bootstrap;

#[cfg(target_os = "macos")]
#[test]
fn clone_entries_accept_files_directories_and_globs() {
    let source = Dir::new();
    source.write("data.bin", "DATA");
    source.write("cache/a/item.bin", "A");
    source.write("cache/b/item.bin", "B");
    let worktree = Dir::new();

    let cloned = bootstrap::clone_files(
        source.path(),
        worktree.path(),
        &["data.bin".to_string(), "cache/*".to_string()],
    )
    .expect("clone phase should run on APFS");

    assert_eq!(cloned, 3);
    assert_eq!(worktree.read("data.bin"), "DATA");
    assert_eq!(worktree.read("cache/a/item.bin"), "A");
    assert_eq!(worktree.read("cache/b/item.bin"), "B");

    worktree.write("data.bin", "CHANGED");
    assert_eq!(
        source.read("data.bin"),
        "DATA",
        "clones must diverge on write"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn reapplying_clones_replaces_existing_destinations() {
    let source = Dir::new();
    source.write("models/weights.bin", "SOURCE");
    let worktree = Dir::new();
    worktree.write("models/weights.bin", "STALE");

    bootstrap::clone_files(
        source.path(),
        worktree.path(),
        &["models/weights.bin".to_string()],
    )
    .expect("clone phase should replace the destination");

    assert_eq!(worktree.read("models/weights.bin"), "SOURCE");
}

#[cfg(target_os = "macos")]
#[test]
fn cloned_python_venvs_are_relocated_to_the_worktree() {
    use std::os::unix::fs::PermissionsExt;

    let source = Dir::new();
    let worktree = Dir::new();
    source.write(".venv/pyvenv.cfg", "home = /opt/homebrew/bin\n");
    let source_root = source.path().canonicalize().expect("canonicalizing source");
    let worktree_root = worktree
        .path()
        .canonicalize()
        .expect("canonicalizing worktree");
    let tool = source.write(
        ".venv/bin/tool",
        &format!("#!{}/.venv/bin/python3\n", source_root.display()),
    );
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))
        .expect("setting executable mode");

    bootstrap::clone_files(source.path(), worktree.path(), &[".venv".to_string()])
        .expect("venv clone should run on APFS");

    let cloned_tool = worktree.path().join(".venv/bin/tool");
    assert_eq!(
        std::fs::read_to_string(&cloned_tool).unwrap(),
        format!("#!{}/.venv/bin/python3\n", worktree_root.display())
    );
    assert_eq!(
        std::fs::metadata(cloned_tool).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[cfg(target_os = "macos")]
#[test]
fn cloning_a_nested_file_never_follows_a_destination_parent_symlink() {
    let source = Dir::new();
    source.write("models/cache/item.bin", "SOURCE");
    let outside = Dir::new();
    outside.write("item.bin", "OUTSIDE");
    let worktree = Dir::new();
    worktree.write("models/placeholder", "");
    std::os::unix::fs::symlink(outside.path(), worktree.path().join("models/cache"))
        .expect("creating destination parent symlink");

    bootstrap::clone_files(
        source.path(),
        worktree.path(),
        &["models/cache/item.bin".to_string()],
    )
    .expect("cloning should replace the unsafe parent symlink");

    assert!(!worktree.path().join("models/cache").is_symlink());
    assert_eq!(worktree.read("models/cache/item.bin"), "SOURCE");
    assert_eq!(outside.read("item.bin"), "OUTSIDE");
}

#[test]
fn clone_entries_cannot_escape_the_source_repo_or_replace_git_metadata() {
    let source = Dir::new();
    source.write(".git/config", "source metadata");
    let worktree = Dir::new();
    worktree.write(".git", "gitdir: elsewhere");

    for pattern in ["../secret", ".git"] {
        let err = bootstrap::clone_files(source.path(), worktree.path(), &[pattern.to_string()])
            .expect_err("unsafe clone paths must be rejected before platform checks");
        assert!(
            err.to_string().contains("relative") || err.to_string().contains("Git metadata"),
            "got: {err}"
        );
    }

    assert_eq!(worktree.read(".git"), "gitdir: elsewhere");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn clone_phase_fails_clearly_on_unsupported_platforms() {
    let source = Dir::new();
    source.write("data.bin", "DATA");
    let worktree = Dir::new();

    let err = bootstrap::clone_files(source.path(), worktree.path(), &["data.bin".to_string()])
        .expect_err("APFS clones are macOS-only");

    assert!(err.to_string().contains("macOS"), "got: {err}");
    assert!(!worktree.exists("data.bin"));
}
