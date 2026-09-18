//! User-level plugin settings and per-repo worktree sync configuration.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Config path relative to the repo/worktree root when the user has not
/// overridden it in the plugin's user-level config.
pub const DEFAULT_REPO_CONFIG_PATH: &str = ".worktree-sync.toml";

/// User-level settings read from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Path of the sync file inside every repository.
    #[serde(default = "default_repo_config_path")]
    pub repo_config_path: PathBuf,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            repo_config_path: default_repo_config_path(),
        }
    }
}

impl PluginConfig {
    /// Load user-level settings. A missing file uses the built-in repo config
    /// path.
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("config.toml");
        if !path.is_file() {
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading plugin config {}", path.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("parsing plugin config {}", path.display()))?;
        validate_repo_config_path(&config.repo_config_path)?;
        Ok(config)
    }

    /// Load settings from Herdr's runtime directory when available.
    pub fn load_from_env() -> Result<Self> {
        match std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
            Some(dir) => Self::load(Path::new(&dir)),
            None => Ok(Self::default()),
        }
    }
}

fn default_repo_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_REPO_CONFIG_PATH)
}

fn validate_repo_config_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        bail!(
            "repo_config_path must be a relative path inside the repository: {}",
            path.display()
        );
    }

    match path.extension().and_then(|extension| extension.to_str()) {
        Some("toml" | "yaml" | "yml") => Ok(()),
        _ => bail!("repo_config_path must end in .toml, .yaml, or .yml"),
    }
}

/// Every config struct denies unknown fields: a typo like `pattern` for
/// `patterns` would otherwise deserialize to the default and silently do the
/// wrong thing, which is the most expensive kind of config bug to debug.
#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Update git (e.g. fetch) before anything else.
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub copy: CopyConfig,
    #[serde(default)]
    pub clone: CloneConfig,
    #[serde(default)]
    pub symlink: SymlinkConfig,
    #[serde(default)]
    pub install: InstallConfig,
    /// Commands run before/after the file operations and install phases.
    #[serde(default)]
    pub hooks: Hooks,
    /// Whether to announce the result as a herdr toast.
    #[serde(default)]
    pub notify: NotifyConfig,
    /// What to do with the worktree when the bootstrap aborts.
    #[serde(default)]
    pub failure: FailureConfig,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FailureConfig {
    #[serde(default)]
    pub action: OnFailure,
}

/// What happens to a worktree whose bootstrap failed.
///
/// herdr fires this plugin *after* the worktree, its workspace and its pane
/// already exist, so "don't create it on failure" isn't on the menu — the only
/// choice is whether to undo what herdr just did.
#[derive(Deserialize, Default, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    /// Leave the half-bootstrapped worktree in place (default).
    #[default]
    Keep,
    /// Tear the worktree, its workspace and its pane back down.
    ///
    /// Opt-in, and deliberately not the default: a bootstrap can run for
    /// minutes, the agent pane is usable the whole time, and the removal is
    /// forced — so anything typed or written in that window goes with it.
    Remove,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotifyConfig {
    #[serde(default)]
    pub when: NotifyWhen,
}

/// When to post a herdr toast summarising the run.
///
/// Unlike every other section this defaults to on, because the cost of being
/// wrong is asymmetric: herdr captures the plugin's stdout instead of printing
/// it, so a run nobody is told about is a run nobody can see. It cannot
/// surprise anyone either — herdr suppresses toasts entirely unless the user
/// has already set `[ui.toast] delivery` in their own config.
#[derive(Deserialize, Default, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum NotifyWhen {
    /// Stay silent whatever happens.
    Never,
    /// Only when the bootstrap aborts.
    Failure,
    /// After every run in which at least one phase was enabled.
    #[default]
    Always,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitConfig {
    /// Bring git up to date before the pre hooks run.
    #[serde(default)]
    pub update: bool,
    /// Override the update command. Defaults to `git fetch --all --prune`.
    #[serde(default)]
    pub command: Option<Vec<String>>,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// Run first, before file operations and install.
    #[serde(default)]
    pub pre: Vec<CommandConfig>,
    /// Run last, after file operations and install.
    #[serde(default)]
    pub post: Vec<CommandConfig>,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CopyConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Relative paths or glob patterns to copy from the source repo. When set,
    /// gitignored discovery is disabled; directories are copied recursively
    /// and missing matches are skipped. Omit to use discovery instead.
    #[serde(default)]
    pub files: Option<Vec<String>>,
    /// Filename globs used by recursive discovery (only when `files` is
    /// omitted). Discovery walks the source repo for **gitignored** files whose
    /// basename matches one of these globs and copies each to the same relative
    /// path in the worktree. Committed files (e.g. `.env.example`) are never
    /// copied because they aren't gitignored. Omit for the env-file defaults
    /// (`.env`, `.env.*`). `*` matches any sequence of characters.
    #[serde(default)]
    pub patterns: Option<Vec<String>>,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CloneConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Relative paths or glob patterns to clone from the primary checkout.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SymlinkConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Relative paths or glob patterns to link from the primary checkout.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Directories to install in, relative to the worktree root. Detection runs
    /// independently in each one, so a polyglot monorepo can install several
    /// packages. Omit for `["."]` — the worktree root only. A listed directory
    /// that doesn't exist is an error, not a skip.
    #[serde(default)]
    pub dirs: Option<Vec<String>>,
    /// Custom detection rules, checked *before* the built-ins.
    /// Add any language here without touching Rust.
    #[serde(default)]
    pub rules: Vec<InstallRule>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstallRule {
    /// File whose presence in the worktree triggers this rule (e.g. "go.mod").
    pub marker: String,
    /// Command to run when the marker is found (e.g. ["go", "mod", "download"]).
    pub command: Vec<String>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    /// argv, e.g. ["cargo", "build"]. First element is the program.
    pub command: Vec<String>,
    /// Working directory, relative to the worktree root. Omit to run at the
    /// root. Lets a hook target one package of a monorepo.
    #[serde(default)]
    pub dir: Option<String>,
}

/// Load the repo's configured sync file. A missing file is not an error —
/// it just means "do nothing". TOML and YAML use the same schema.
pub fn load(repo: &Path, plugin_config: &PluginConfig) -> Result<Config> {
    let path = repo.join(&plugin_config.repo_config_path);
    if !path.is_file() {
        println!(
            "[worktree-sync] no config ({}) in {}, nothing to do",
            plugin_config.repo_config_path.display(),
            repo.display()
        );
        return Ok(Config::default());
    }

    println!("[worktree-sync] config:   {}", path.display());
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

    let config: Config = match path.extension().and_then(|ext| ext.to_str()) {
        Some("yaml") | Some("yml") => {
            serde_yaml_ng::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
        }
        _ => toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?,
    };
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml_config(text: &str) -> Config {
        toml::from_str(text).expect("config should parse")
    }

    #[test]
    fn an_empty_config_is_all_defaults() {
        let config = toml_config("");
        assert_eq!(config, Config::default());
        // Every phase is opt-in: an empty config must do nothing at all.
        assert!(!config.git.update);
        assert!(!config.copy.enabled);
        assert!(!config.clone.enabled);
        assert!(!config.symlink.enabled);
        assert!(!config.install.enabled);
        assert!(config.hooks.pre.is_empty());
        assert!(config.hooks.post.is_empty());
    }

    /// Discovery mode is signalled by `files` being absent, not empty — an
    /// empty list means "copy nothing", which must stay distinguishable.
    #[test]
    fn copy_distinguishes_an_absent_files_list_from_an_empty_one() {
        assert_eq!(toml_config("[copy]\nenabled = true").copy.files, None);
        assert_eq!(
            toml_config("[copy]\nenabled = true\nfiles = []").copy.files,
            Some(vec![])
        );
    }

    /// Every *phase* is opt-in, but notification is opt-out: see [`NotifyWhen`]
    /// for why. Pinned here because it is the one place the config breaks its
    /// own "an empty config does nothing" rule.
    #[test]
    fn notification_is_the_one_section_that_defaults_to_on() {
        assert_eq!(toml_config("").notify.when, NotifyWhen::Always);
        assert_eq!(
            toml_config("[notify]\nwhen = \"failure\"").notify.when,
            NotifyWhen::Failure
        );
        assert_eq!(
            toml_config("[notify]\nwhen = \"never\"").notify.when,
            NotifyWhen::Never
        );
    }

    #[test]
    fn an_unknown_notify_mode_is_rejected() {
        let err = toml::from_str::<Config>("[notify]\nwhen = \"sometimes\"")
            .expect_err("only the three documented modes are valid");
        let msg = err.to_string();
        for mode in ["never", "failure", "always"] {
            assert!(msg.contains(mode), "error should list `{mode}`, got: {msg}");
        }
    }

    /// Removal is destructive and irreversible, so the one thing this test
    /// pins is that it can only happen because somebody asked for it.
    #[test]
    fn a_failed_bootstrap_keeps_its_worktree_unless_told_otherwise() {
        assert_eq!(toml_config("").failure.action, OnFailure::Keep);
        assert_eq!(
            toml_config("[failure]\naction = \"remove\"").failure.action,
            OnFailure::Remove
        );
    }

    #[test]
    fn an_unknown_failure_action_is_rejected() {
        let err = toml::from_str::<Config>("[failure]\naction = \"rollback\"")
            .expect_err("only the two documented actions are valid");
        let msg = err.to_string();
        for action in ["keep", "remove"] {
            assert!(
                msg.contains(action),
                "error should list `{action}`, got: {msg}"
            );
        }
    }

    #[test]
    fn parses_a_full_config() {
        let config = toml_config(
            r#"
            [git]
            update = true
            command = ["git", "pull", "--ff-only"]

            [copy]
            enabled = true
            files = [".env", "apps/web/.env.local"]

            [symlink]
            enabled = true
            files = [".pnpm-store", ".next/cache"]

            [clone]
            enabled = true
            files = ["models/*.bin"]

            [install]
            enabled = true
            dirs = ["apps/web", "services/api"]

            [[install.rules]]
            marker = "flake.nix"
            command = ["nix", "develop"]

            [[hooks.pre]]
            command = ["mise", "install"]

            [[hooks.post]]
            command = ["npm", "run", "codegen"]
            dir = "apps/web"
            "#,
        );

        assert!(config.git.update);
        assert_eq!(
            config.git.command.as_deref(),
            Some(["git", "pull", "--ff-only"].map(String::from).as_slice())
        );
        assert_eq!(
            config.copy.files.as_deref(),
            Some([".env", "apps/web/.env.local"].map(String::from).as_slice())
        );
        assert_eq!(
            config.symlink.files,
            [".pnpm-store", ".next/cache"].map(String::from)
        );
        assert_eq!(config.clone.files, ["models/*.bin"].map(String::from));
        assert_eq!(config.install.rules.len(), 1);
        assert_eq!(config.install.rules[0].marker, "flake.nix");
        assert_eq!(config.hooks.pre.len(), 1);
        assert_eq!(config.hooks.pre[0].dir, None);
        assert_eq!(config.hooks.post[0].dir.as_deref(), Some("apps/web"));
    }

    /// `deny_unknown_fields` exists so a typo fails loudly instead of
    /// deserializing to the default and silently doing nothing.
    #[test]
    fn a_typo_names_both_the_bad_key_and_the_valid_ones() {
        let err = toml::from_str::<Config>("[copy]\nenabled = true\npattern = [\".env\"]")
            .expect_err("a misspelled key should be rejected");
        let msg = err.to_string();
        assert!(msg.contains("unknown field"), "got: {msg}");
        assert!(msg.contains("pattern"), "got: {msg}");
        assert!(msg.contains("patterns"), "got: {msg}");
    }

    #[test]
    fn unknown_keys_are_rejected_at_every_level() {
        for text in [
            "bogus = true",
            "[bogus]\nenabled = true",
            "[git]\nbogus = true",
            "[install]\nbogus = true",
            "[[install.rules]]\nmarker = \"a\"\ncommand = []\nbogus = true",
            "[[hooks.pre]]\ncommand = []\nbogus = true",
            "[failure]\nbogus = true",
        ] {
            assert!(
                toml::from_str::<Config>(text).is_err(),
                "should have been rejected: {text}"
            );
        }
    }

    #[test]
    fn required_keys_are_enforced_in_rules_and_hooks() {
        // `marker` and `command` have no default — omitting either is an error.
        assert!(toml::from_str::<Config>("[[install.rules]]\nmarker = \"a\"").is_err());
        assert!(toml::from_str::<Config>("[[install.rules]]\ncommand = []").is_err());
        assert!(toml::from_str::<Config>("[[hooks.pre]]\ndir = \"apps/web\"").is_err());
    }

    /// The README promises TOML and YAML are the same schema. They deserialize
    /// into the same struct, so the only way they can diverge is a parser quirk
    /// — pin the equivalence here, and against the shipped examples in
    /// `tests/config_load.rs`.
    #[test]
    fn yaml_and_toml_produce_the_same_config() {
        let from_toml = toml_config(
            r#"
            [copy]
            enabled = true
            patterns = [".env", ".env.*"]

            [[hooks.post]]
            command = ["direnv", "allow"]
            "#,
        );
        let from_yaml: Config = serde_yaml_ng::from_str(
            r#"
copy:
  enabled: true
  patterns: [".env", ".env.*"]
hooks:
  post:
    - command: ["direnv", "allow"]
"#,
        )
        .expect("yaml should parse");

        assert_eq!(from_toml, from_yaml);
    }

    #[test]
    fn yaml_also_rejects_unknown_keys() {
        assert!(serde_yaml_ng::from_str::<Config>("copy:\n  pattern: [\".env\"]\n").is_err());
    }
}
