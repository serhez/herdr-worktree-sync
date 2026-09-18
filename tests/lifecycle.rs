//! The bootstrap lifecycle end to end: which phases run, in what order, and
//! what happens after the first failure.
//!
//! Hooks append a label to `log.txt` in their working directory, so the log is
//! a literal record of the order the phases executed in.

mod common;

use common::Dir;
use herdr_worktree_sync::{config, run};

/// A source repo carrying `config_toml`, plus an empty worktree to bootstrap.
fn setup(config_toml: &str) -> (Dir, Dir, config::Config) {
    let source = Dir::new();
    source.write(".worktree-sync.toml", config_toml);
    let worktree = Dir::new();
    let config =
        config::load(source.path(), &config::PluginConfig::default()).expect("config should load");
    (source, worktree, config)
}

/// The order the README promises: git update → pre → copy → install → post.
///
/// Copy has no hook of its own to log, so its position is pinned differently:
/// the install rule's marker is the very file copy delivers, so `install` can
/// only appear in the log if copy already ran.
#[test]
fn phases_run_in_the_documented_order() {
    let (source, worktree, config) = setup(
        r#"
        [git]
        update = true
        command = ["sh", "-c", "echo git >> log.txt"]

        [[hooks.pre]]
        command = ["sh", "-c", "echo pre >> log.txt"]

        [copy]
        enabled = true
        files = [".env"]

        [install]
        enabled = true

        [[install.rules]]
        marker = ".env"
        command = ["sh", "-c", "echo install >> log.txt"]

        [[hooks.post]]
        command = ["sh", "-c", "echo post >> log.txt"]
        "#,
    );
    source.write(".env", "SECRET");

    run(worktree.path(), Some(source.path()), &config).expect("bootstrap should succeed");

    assert_eq!(
        worktree.log_lines("log.txt"),
        ["git", "pre", "install", "post"]
    );
    assert_eq!(
        worktree.read(".env"),
        "SECRET",
        "copy must have run before install detected its marker"
    );
}

#[test]
fn symlinks_are_created_before_dependency_installation() {
    let (source, worktree, config) = setup(
        r#"
        [symlink]
        enabled = true
        files = ["shared/marker.txt"]

        [install]
        enabled = true

        [[install.rules]]
        marker = "shared/marker.txt"
        command = ["sh", "-c", "echo install >> log.txt"]
        "#,
    );
    source.write("shared/marker.txt", "shared");

    run(worktree.path(), Some(source.path()), &config).expect("bootstrap should succeed");

    assert_eq!(worktree.log_lines("log.txt"), ["install"]);
    assert_eq!(worktree.read("shared/marker.txt"), "shared");
}

#[test]
fn an_empty_config_runs_no_phases_at_all() {
    let (source, worktree, config) = setup("");

    run(worktree.path(), Some(source.path()), &config).expect("doing nothing should succeed");

    assert!(worktree.log_lines("log.txt").is_empty());
}

/// Fail-fast: a failing pre hook stops the bootstrap before anything is copied.
#[test]
fn a_failing_pre_hook_aborts_before_copy() {
    let (source, worktree, config) = setup(
        r#"
        [[hooks.pre]]
        command = ["sh", "-c", "exit 3"]

        [copy]
        enabled = true
        files = [".env"]

        [[hooks.post]]
        command = ["sh", "-c", "echo post >> log.txt"]
        "#,
    );
    source.write(".env", "SECRET");

    let err = run(worktree.path(), Some(source.path()), &config)
        .expect_err("a non-zero hook should abort the bootstrap");
    assert!(err.to_string().contains("exit"), "got: {err}");

    assert!(!worktree.exists(".env"), "copy must not have run");
    assert!(
        worktree.log_lines("log.txt").is_empty(),
        "post hooks must not have run"
    );
}

#[test]
fn a_failing_install_stops_the_post_hooks() {
    let (source, worktree, config) = setup(
        r#"
        [install]
        enabled = true

        [[install.rules]]
        marker = "marker.txt"
        command = ["sh", "-c", "exit 1"]

        [[hooks.post]]
        command = ["sh", "-c", "echo post >> log.txt"]
        "#,
    );
    worktree.write("marker.txt", "");

    run(worktree.path(), Some(source.path()), &config)
        .expect_err("a failing install should abort the bootstrap");

    assert!(
        worktree.log_lines("log.txt").is_empty(),
        "post hooks must not have run"
    );
}

#[test]
fn hooks_run_in_the_order_they_are_listed() {
    let (source, worktree, config) = setup(
        r#"
        [[hooks.post]]
        command = ["sh", "-c", "echo first >> log.txt"]

        [[hooks.post]]
        command = ["sh", "-c", "echo second >> log.txt"]

        [[hooks.post]]
        command = ["sh", "-c", "echo third >> log.txt"]
        "#,
    );

    run(worktree.path(), Some(source.path()), &config).expect("bootstrap should succeed");

    assert_eq!(worktree.log_lines("log.txt"), ["first", "second", "third"]);
}

/// `dir` targets one package of a monorepo; the hook's cwd is that directory,
/// which is where its log lands.
#[test]
fn a_hook_dir_selects_the_working_directory() {
    let (source, worktree, config) = setup(
        r#"
        [[hooks.post]]
        command = ["sh", "-c", "echo web >> log.txt"]
        dir = "apps/web"
        "#,
    );
    worktree.write("apps/web/package.json", "{}");

    run(worktree.path(), Some(source.path()), &config).expect("bootstrap should succeed");

    assert_eq!(worktree.log_lines("apps/web/log.txt"), ["web"]);
    assert!(
        worktree.log_lines("log.txt").is_empty(),
        "the hook should not have run at the worktree root"
    );
}

/// A typo in `dir` is reported as such, instead of surfacing as a confusing
/// "failed to spawn sh".
#[test]
fn a_hook_dir_that_does_not_exist_is_reported_clearly() {
    let (source, worktree, config) = setup(
        r#"
        [[hooks.post]]
        command = ["sh", "-c", "true"]
        dir = "apps/nope"
        "#,
    );

    let err = run(worktree.path(), Some(source.path()), &config)
        .expect_err("a missing hook dir should abort");
    assert!(
        err.to_string().contains("working directory does not exist"),
        "got: {err}"
    );
}

#[test]
fn a_missing_install_dir_aborts_before_installing_anything() {
    let (source, worktree, config) = setup(
        r#"
        [install]
        enabled = true
        dirs = ["apps/web", "services/api"]

        [[install.rules]]
        marker = "package.json"
        command = ["sh", "-c", "echo installed >> ../../log.txt"]
        "#,
    );
    worktree.write("apps/web/package.json", "{}");

    let err = run(worktree.path(), Some(source.path()), &config)
        .expect_err("a missing install dir should abort");
    assert!(err.to_string().contains("services/api"), "got: {err}");

    assert!(
        worktree.log_lines("log.txt").is_empty(),
        "apps/web must not have been installed before the typo was caught"
    );
}

#[test]
fn install_runs_once_per_listed_directory() {
    let (source, worktree, config) = setup(
        r#"
        [install]
        enabled = true
        dirs = ["apps/web", "services/api"]

        [[install.rules]]
        marker = "package.json"
        command = ["sh", "-c", "echo node >> ../../log.txt"]

        [[install.rules]]
        marker = "go.mod"
        command = ["sh", "-c", "echo go >> ../../log.txt"]
        "#,
    );
    worktree.write("apps/web/package.json", "{}");
    worktree.write("services/api/go.mod", "module api");

    run(worktree.path(), Some(source.path()), &config).expect("bootstrap should succeed");

    assert_eq!(worktree.log_lines("log.txt"), ["node", "go"]);
}

/// File operations need the source repo and say so when Herdr omits it.
#[test]
fn copy_without_a_source_repo_is_an_error() {
    let (_source, worktree, config) = setup("[copy]\nenabled = true\n");

    let err = run(worktree.path(), None, &config).expect_err("copy needs a source repo");
    assert!(err.to_string().contains("source repo_root"), "got: {err}");
}

#[test]
fn symlink_without_a_source_repo_is_an_error() {
    let (_source, worktree, config) = setup("[symlink]\nenabled = true\nfiles = [\".cache\"]\n");

    let err = run(worktree.path(), None, &config).expect_err("symlink needs a source repo");
    assert!(err.to_string().contains("source repo_root"), "got: {err}");
}

/// Without copy enabled, a missing source is irrelevant.
#[test]
fn the_other_phases_do_not_need_a_source_repo() {
    let (_source, worktree, config) = setup(
        r#"
        [[hooks.post]]
        command = ["sh", "-c", "echo post >> log.txt"]
        "#,
    );

    run(worktree.path(), None, &config).expect("no source is fine when copy is off");

    assert_eq!(worktree.log_lines("log.txt"), ["post"]);
}
