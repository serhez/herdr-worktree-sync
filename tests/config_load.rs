//! `config::load` — which path is loaded, what a missing file means, and whether the
//! shipped examples still parse.

mod common;

use common::Dir;
use herdr_worktree_sync::config::{self, Config, PluginConfig};

/// The shipped examples, compiled in so a broken sample fails CI rather than a
/// user's first run.
const EXAMPLE_TOML: &str = include_str!("../examples/worktree-sync.toml");
const EXAMPLE_YAML: &str = include_str!("../examples/worktree-sync.yaml");

#[test]
fn a_repo_without_a_config_does_nothing() {
    let repo = Dir::new();
    let config = config::load(repo.path(), &PluginConfig::default())
        .expect("a missing config is not an error");
    assert_eq!(config, Config::default());
}

#[test]
fn the_default_repo_config_path_is_worktree_sync_toml() {
    let repo = Dir::new();
    repo.write(".worktree-sync.toml", "[copy]\nenabled = true\n");

    let config = config::load(repo.path(), &PluginConfig::default()).expect("config should load");
    assert!(config.copy.enabled);
}

#[test]
fn legacy_bootstrap_paths_are_not_loaded_implicitly() {
    let repo = Dir::new();
    repo.write(".herdr/worktree-bootstrap.toml", "[copy]\nenabled = true\n");

    let config = config::load(repo.path(), &PluginConfig::default()).expect("config should load");
    assert_eq!(config, Config::default());
}

/// The README's central claim about the two formats: same schema, so the two
/// shipped examples must deserialize to exactly the same config.
#[test]
fn the_toml_and_yaml_examples_are_equivalent() {
    let from_toml = Dir::new();
    from_toml.write(".worktree-sync.toml", EXAMPLE_TOML);

    let from_yaml = Dir::new();
    let plugin_dir = Dir::new();
    plugin_dir.write(
        "config.toml",
        "repo_config_path = \".worktree-sync.yaml\"\n",
    );
    from_yaml.write(".worktree-sync.yaml", EXAMPLE_YAML);
    let yaml_plugin_config =
        PluginConfig::load(plugin_dir.path()).expect("plugin config should load");

    let toml_config = config::load(from_toml.path(), &PluginConfig::default())
        .expect("examples/*.toml should parse");
    let yaml_config =
        config::load(from_yaml.path(), &yaml_plugin_config).expect("examples/*.yaml should parse");

    assert_eq!(toml_config, yaml_config);
}

#[test]
fn a_broken_config_reports_the_file_it_came_from() {
    let repo = Dir::new();
    repo.write(".worktree-sync.toml", "[copy\nenabled = true\n");

    let err = config::load(repo.path(), &PluginConfig::default())
        .expect_err("malformed TOML should abort");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("worktree-sync.toml"),
        "error should name the config file, got: {chain}"
    );
}

#[test]
fn a_typo_in_a_repos_config_aborts_the_load() {
    let repo = Dir::new();
    repo.write(
        ".worktree-sync.toml",
        "[copy]\nenabled = true\npattern = [\".env\"]\n",
    );

    let err = config::load(repo.path(), &PluginConfig::default())
        .expect_err("an unknown key should abort");
    let chain = format!("{err:#}");
    assert!(chain.contains("pattern"), "got: {chain}");
}

#[test]
fn plugin_config_can_select_a_different_repo_config_path() {
    let repo = Dir::new();
    repo.write(".worktree-sync.toml", "[copy]\nenabled = false\n");
    repo.write("config/worktree.yaml", "copy:\n  enabled: true\n");
    let plugin_dir = Dir::new();
    plugin_dir.write(
        "config.toml",
        "repo_config_path = \"config/worktree.yaml\"\n",
    );

    let plugin_config = PluginConfig::load(plugin_dir.path()).expect("plugin config should load");
    let config = config::load(repo.path(), &plugin_config).expect("repo config should load");

    assert!(config.copy.enabled);
}

#[test]
fn a_custom_repo_config_path_must_stay_inside_the_repo() {
    for path in [
        "/tmp/bootstrap.toml",
        "../bootstrap.toml",
        "a/../../bootstrap.toml",
    ] {
        let plugin_dir = Dir::new();
        plugin_dir.write("config.toml", &format!("repo_config_path = {path:?}\n"));

        let err = PluginConfig::load(plugin_dir.path())
            .expect_err("an escaping config path should be rejected");
        assert!(err.to_string().contains("repo_config_path"), "got: {err}");
    }
}

#[test]
fn a_custom_repo_config_path_uses_its_extension_to_choose_the_parser() {
    let repo = Dir::new();
    repo.write("config/bootstrap.yaml", "copy:\n  enabled: true\n");
    let plugin_dir = Dir::new();
    plugin_dir.write(
        "config.toml",
        "repo_config_path = \"config/bootstrap.yaml\"\n",
    );

    let plugin_config = PluginConfig::load(plugin_dir.path()).expect("plugin config should load");
    let config = config::load(repo.path(), &plugin_config).expect("YAML config should load");

    assert!(config.copy.enabled);
}

#[test]
fn clone_paths_are_part_of_the_repo_config_schema() {
    let repo = Dir::new();
    repo.write(
        ".worktree-sync.toml",
        "[clone]\nenabled = true\nfiles = [\"models/*.bin\", \"cache\"]\n",
    );

    let config = config::load(repo.path(), &PluginConfig::default()).expect("config should load");

    assert!(config.clone.enabled);
    assert_eq!(config.clone.files, ["models/*.bin", "cache"]);
}
