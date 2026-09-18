//! The copy phase against real git repositories.
//!
//! Discovery delegates "is this gitignored?" to `git ls-files`, so these tests
//! drive actual repositories rather than mocking git — the delegation *is* the
//! behaviour worth testing.

mod common;

use common::{Dir, init_repo};
use herdr_worktree_sync::bootstrap;

/// A source repo shaped like a small monorepo: committed files in each package
/// (so git doesn't collapse the package as wholly-ignored) plus gitignored env
/// files at several depths.
fn monorepo() -> Dir {
    let repo = Dir::new();
    repo.write(".gitignore", ".env\n.env.*\n!.env.example\nnode_modules/\n");
    repo.write(".env.example", "EXAMPLE");
    repo.write("apps/web/package.json", "{}");
    repo.write("packages/db/schema.sql", "-- schema");
    init_repo(&repo);

    // Written after the commit so they stay untracked-and-ignored.
    repo.write(".env", "ROOT");
    repo.write("apps/web/.env.local", "WEB");
    repo.write("packages/db/.env", "DB");
    repo.write("node_modules/.env", "VENDORED");
    repo
}

#[test]
fn discovery_copies_gitignored_env_files_at_any_depth() {
    let source = monorepo();
    let worktree = Dir::new();

    bootstrap::copy_gitignored(source.path(), worktree.path(), None).expect("discovery should run");

    assert_eq!(worktree.read(".env"), "ROOT");
    assert_eq!(worktree.read("apps/web/.env.local"), "WEB");
    assert_eq!(worktree.read("packages/db/.env"), "DB");
}

/// `.env.example` is committed, so a fresh checkout already has it and copying
/// it would overwrite the worktree's own copy.
#[test]
fn discovery_never_copies_committed_files() {
    let source = monorepo();
    let worktree = Dir::new();

    bootstrap::copy_gitignored(source.path(), worktree.path(), None).expect("discovery should run");

    assert!(
        !worktree.exists(".env.example"),
        ".env.example is committed and must not be copied"
    );
}

/// A wholly-ignored directory is collapsed by `--directory` and skipped rather
/// than walked, so a 100k-file `node_modules` costs nothing.
#[test]
fn discovery_does_not_descend_into_wholly_ignored_directories() {
    let source = monorepo();
    let worktree = Dir::new();

    bootstrap::copy_gitignored(source.path(), worktree.path(), None).expect("discovery should run");

    assert!(
        !worktree.exists("node_modules/.env"),
        "node_modules/ should be skipped, not walked into"
    );
}

#[test]
fn discovery_honours_custom_patterns() {
    let source = Dir::new();
    source.write(".gitignore", ".env\n*.secret\n");
    source.write("README.md", "# repo");
    init_repo(&source);
    source.write(".env", "ENV");
    source.write("keys.secret", "SHH");

    let worktree = Dir::new();
    let patterns = vec!["*.secret".to_string()];
    bootstrap::copy_gitignored(source.path(), worktree.path(), Some(&patterns))
        .expect("discovery should run");

    assert_eq!(worktree.read("keys.secret"), "SHH");
    assert!(
        !worktree.exists(".env"),
        ".env does not match the custom patterns"
    );
}

/// Nested `.gitignore` files and negations are git's job, not ours. Here only
/// the service's own `.gitignore` marks `.env` as ignored, so the identically
/// named file at the root must be left alone.
#[test]
fn discovery_honours_nested_gitignore_files() {
    let source = Dir::new();
    source.write(".gitignore", "node_modules/\n");
    source.write("svc/.gitignore", ".env\n");
    source.write("svc/main.go", "package main");
    init_repo(&source);
    source.write(".env", "ROOT-NOT-IGNORED");
    source.write("svc/.env", "SVC");

    let worktree = Dir::new();
    bootstrap::copy_gitignored(source.path(), worktree.path(), None).expect("discovery should run");

    assert_eq!(worktree.read("svc/.env"), "SVC");
    assert!(
        !worktree.exists(".env"),
        "the root .env isn't gitignored, so it isn't discovered"
    );
}

#[test]
fn discovery_is_quiet_when_nothing_matches() {
    let source = Dir::new();
    source.write(".gitignore", "node_modules/\n");
    source.write("README.md", "# repo");
    init_repo(&source);

    let worktree = Dir::new();
    bootstrap::copy_gitignored(source.path(), worktree.path(), None)
        .expect("finding nothing is not an error");
}

/// A directory that isn't a git repo at all: `git ls-files` fails, and the
/// bootstrap logs and continues rather than aborting.
#[test]
fn discovery_survives_a_source_that_is_not_a_repo() {
    let source = Dir::new();
    source.write(".env", "ENV");

    let worktree = Dir::new();
    bootstrap::copy_gitignored(source.path(), worktree.path(), None)
        .expect("a failed `git ls-files` should not abort the bootstrap");
    assert!(!worktree.exists(".env"));
}

#[test]
fn explicit_files_are_copied_with_their_parent_directories() {
    let source = Dir::new();
    source.write(".env", "ROOT");
    source.write("apps/web/.env.local", "WEB");

    let worktree = Dir::new();
    let files = [".env", "apps/web/.env.local"].map(String::from);
    bootstrap::copy_files(source.path(), worktree.path(), &files).expect("copy should run");

    assert_eq!(worktree.read(".env"), "ROOT");
    assert_eq!(worktree.read("apps/web/.env.local"), "WEB");
}

/// Explicit mode is a wish list, not a manifest: a repo that legitimately has
/// no `.env.local` shouldn't fail every worktree it creates.
#[test]
fn explicit_files_skip_what_the_source_does_not_have() {
    let source = Dir::new();
    source.write(".env", "ROOT");

    let worktree = Dir::new();
    let files = [".env", ".env.local"].map(String::from);
    bootstrap::copy_files(source.path(), worktree.path(), &files)
        .expect("a missing source file is skipped, not an error");

    assert_eq!(worktree.read(".env"), "ROOT");
    assert!(!worktree.exists(".env.local"));
}

/// Explicit mode bypasses discovery entirely, so it can carry a committed file
/// that discovery would deliberately leave behind.
#[test]
fn explicit_files_are_not_filtered_by_gitignore() {
    let source = monorepo();
    let worktree = Dir::new();

    let files = [".env.example".to_string()];
    bootstrap::copy_files(source.path(), worktree.path(), &files).expect("copy should run");

    assert_eq!(worktree.read(".env.example"), "EXAMPLE");
}

#[test]
fn explicit_entries_accept_globs() {
    let source = Dir::new();
    source.write("config/local.one.toml", "ONE");
    source.write("config/local.two.toml", "TWO");
    source.write("config/public.toml", "PUBLIC");
    let worktree = Dir::new();

    bootstrap::copy_files(
        source.path(),
        worktree.path(),
        &["config/local.*.toml".to_string()],
    )
    .expect("glob copy should run");

    assert_eq!(worktree.read("config/local.one.toml"), "ONE");
    assert_eq!(worktree.read("config/local.two.toml"), "TWO");
    assert!(!worktree.exists("config/public.toml"));
}

#[test]
fn explicit_entries_copy_directories_recursively() {
    let source = Dir::new();
    source.write("config/local/app.toml", "APP");
    source.write("config/local/nested/db.toml", "DB");
    let worktree = Dir::new();

    bootstrap::copy_files(
        source.path(),
        worktree.path(),
        &["config/local".to_string()],
    )
    .expect("directory copy should run");

    assert_eq!(worktree.read("config/local/app.toml"), "APP");
    assert_eq!(worktree.read("config/local/nested/db.toml"), "DB");
}

#[test]
fn explicit_entries_cannot_escape_the_source_repo() {
    let source = Dir::new();
    let worktree = Dir::new();

    let err = bootstrap::copy_files(source.path(), worktree.path(), &["../secret".to_string()])
        .expect_err("parent traversal should be rejected");

    assert!(err.to_string().contains("relative"), "got: {err}");
}

#[test]
fn copying_a_directory_never_follows_a_destination_symlink() {
    let source = Dir::new();
    source.write("cache/new", "NEW");
    let outside = Dir::new();
    outside.write("keep", "KEEP");
    let worktree = Dir::new();
    std::os::unix::fs::symlink(outside.path(), worktree.path().join("cache"))
        .expect("creating destination symlink");

    bootstrap::copy_files(source.path(), worktree.path(), &["cache".to_string()])
        .expect("directory copy should replace the destination symlink");

    assert!(!worktree.path().join("cache").is_symlink());
    assert_eq!(worktree.read("cache/new"), "NEW");
    assert_eq!(outside.read("keep"), "KEEP");
    assert!(
        !outside.exists("new"),
        "copy must not escape through the link"
    );
}

#[test]
fn directory_copy_preserves_source_symlinks() {
    let source = Dir::new();
    source.write("shared/data", "DATA");
    std::os::unix::fs::symlink("data", source.path().join("shared/link"))
        .expect("creating source symlink");
    let worktree = Dir::new();

    bootstrap::copy_files(source.path(), worktree.path(), &["shared".to_string()])
        .expect("directory copy should run");

    assert!(worktree.path().join("shared/link").is_symlink());
    assert_eq!(worktree.read("shared/link"), "DATA");
}

#[test]
fn copying_a_nested_file_never_follows_a_destination_parent_symlink() {
    let source = Dir::new();
    source.write("config/cache/new", "NEW");
    let outside = Dir::new();
    outside.write("new", "OUTSIDE");
    let worktree = Dir::new();
    worktree.write("config/placeholder", "");
    std::os::unix::fs::symlink(outside.path(), worktree.path().join("config/cache"))
        .expect("creating destination parent symlink");

    bootstrap::copy_files(
        source.path(),
        worktree.path(),
        &["config/cache/new".to_string()],
    )
    .expect("copy should replace the unsafe parent symlink");

    assert!(!worktree.path().join("config/cache").is_symlink());
    assert_eq!(worktree.read("config/cache/new"), "NEW");
    assert_eq!(outside.read("new"), "OUTSIDE");
}

#[test]
fn copy_entries_cannot_replace_git_worktree_metadata() {
    let source = Dir::new();
    source.write(".git/config", "source metadata");
    let worktree = Dir::new();
    worktree.write(".git", "gitdir: elsewhere");

    for pattern in [".", ".git", ".git/*"] {
        let err = bootstrap::copy_files(source.path(), worktree.path(), &[pattern.to_string()])
            .expect_err("repository metadata must never be copied");
        assert!(err.to_string().contains("Git metadata"), "got: {err}");
    }

    assert_eq!(worktree.read(".git"), "gitdir: elsewhere");
}
