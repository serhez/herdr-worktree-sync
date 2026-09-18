//! Declarative symlinks from the primary checkout into a linked worktree.

mod common;

use common::Dir;
use herdr_worktree_sync::bootstrap;

#[test]
fn symlink_entries_accept_files_directories_and_globs() {
    let source = Dir::new();
    source.write(".env.local", "ENV");
    source.write("cache/a/data", "A");
    source.write("cache/b/data", "B");
    let worktree = Dir::new();

    let linked = bootstrap::symlink_files(
        source.path(),
        worktree.path(),
        &[".env.local".to_string(), "cache/*".to_string()],
    )
    .expect("symlink phase should run");

    assert_eq!(linked, 3);
    assert_eq!(worktree.read(".env.local"), "ENV");
    assert_eq!(worktree.read("cache/a/data"), "A");
    assert_eq!(worktree.read("cache/b/data"), "B");
    assert!(worktree.path().join(".env.local").is_symlink());
    assert!(worktree.path().join("cache/a").is_symlink());
    assert!(worktree.path().join("cache/b").is_symlink());
}

#[test]
fn symlinks_use_relative_targets() {
    let source = Dir::new();
    source.write("shared/cache", "CACHE");
    let worktree = Dir::new();

    bootstrap::symlink_files(
        source.path(),
        worktree.path(),
        &["shared/cache".to_string()],
    )
    .expect("symlink phase should run");

    let target = std::fs::read_link(worktree.path().join("shared/cache"))
        .expect("destination should be a symlink");
    assert!(
        target.is_relative(),
        "target should be relative: {target:?}"
    );
}

#[test]
fn reapplying_symlinks_replaces_an_existing_destination() {
    let source = Dir::new();
    source.write("shared/cache", "SOURCE");
    let worktree = Dir::new();
    worktree.write("shared/cache", "STALE");

    bootstrap::symlink_files(
        source.path(),
        worktree.path(),
        &["shared/cache".to_string()],
    )
    .expect("symlink phase should replace the destination");

    assert!(worktree.path().join("shared/cache").is_symlink());
    assert_eq!(worktree.read("shared/cache"), "SOURCE");
}

#[test]
fn missing_symlink_matches_are_skipped() {
    let source = Dir::new();
    let worktree = Dir::new();

    let linked =
        bootstrap::symlink_files(source.path(), worktree.path(), &["missing/*".to_string()])
            .expect("a missing match should not abort bootstrap");

    assert_eq!(linked, 0);
}

#[test]
fn symlink_entries_cannot_escape_the_source_repo() {
    let source = Dir::new();
    let worktree = Dir::new();

    let err = bootstrap::symlink_files(source.path(), worktree.path(), &["../secret".to_string()])
        .expect_err("parent traversal should be rejected");

    assert!(err.to_string().contains("relative"), "got: {err}");
}

#[test]
fn overlapping_glob_matches_link_only_the_outer_directory() {
    let source = Dir::new();
    source.write("cache/nested/data", "DATA");
    let worktree = Dir::new();

    let linked = bootstrap::symlink_files(
        source.path(),
        worktree.path(),
        &["cache".to_string(), "cache/**".to_string()],
    )
    .expect("overlapping matches should be collapsed safely");

    assert_eq!(linked, 1);
    assert!(worktree.path().join("cache").is_symlink());
    assert_eq!(source.read("cache/nested/data"), "DATA");
}

#[test]
fn linking_a_nested_path_never_follows_a_destination_parent_symlink() {
    let source = Dir::new();
    source.write("config/cache/new", "NEW");
    let outside = Dir::new();
    outside.write("new", "OUTSIDE");
    let worktree = Dir::new();
    worktree.write("config/placeholder", "");
    std::os::unix::fs::symlink(outside.path(), worktree.path().join("config/cache"))
        .expect("creating destination parent symlink");

    bootstrap::symlink_files(
        source.path(),
        worktree.path(),
        &["config/cache/new".to_string()],
    )
    .expect("linking should replace the unsafe parent symlink");

    assert!(!worktree.path().join("config/cache").is_symlink());
    assert!(worktree.path().join("config/cache/new").is_symlink());
    assert_eq!(worktree.read("config/cache/new"), "NEW");
    assert_eq!(outside.read("new"), "OUTSIDE");
}

#[test]
fn symlink_entries_cannot_replace_git_worktree_metadata() {
    let source = Dir::new();
    source.write(".git/config", "source metadata");
    let worktree = Dir::new();
    worktree.write(".git", "gitdir: elsewhere");

    let err = bootstrap::symlink_files(source.path(), worktree.path(), &[".git".to_string()])
        .expect_err("repository metadata must never be linked");

    assert!(err.to_string().contains("Git metadata"), "got: {err}");
    assert_eq!(worktree.read(".git"), "gitdir: elsewhere");
}
