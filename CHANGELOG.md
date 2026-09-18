# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Because the config schema uses `deny_unknown_fields`, **removing or renaming a
config key is breaking** — an existing repo's committed `.worktree-sync.toml`
stops parsing and the worktree setup aborts. Such
changes are called out under **Changed** or **Removed**, never **Added**.

## [Unreleased]

### Added

- `[clone]` APFS copy-on-write operations for exact paths, directories, and
  glob patterns on macOS.
- Automatic relocation of absolute checkout paths in cloned Python virtual
  environments, detected by `pyvenv.cfg`.
- `[symlink]` file operations for exact paths, directories, and glob patterns.
- Glob and recursive-directory support for explicit `[copy].files` entries.
- A `sync` plugin action that reapplies copy, clone, and symlink operations to the
  focused linked worktree without rerunning hooks or dependency installation.
- User-level `repo_config_path` setting for replacing the default per-repo
  config path with a custom relative TOML or YAML path. The default is
  `.worktree-sync.toml`.
- Renamed the fork to Worktree Sync and its plugin ID to
  `serhez.herdr.worktree.sync`.

## [0.1.0] - 2026-09-16

### Added

- Copy phase: brings gitignored files (`.env` and friends) from the source repo
  into the new worktree. Discovery mode asks git itself which files are ignored,
  recursively, so nested `.gitignore`s and negations work and committed files
  like `.env.example` are never copied. Explicit mode (`files`) copies an exact
  list instead.
- Install phase: detects the package manager from a marker file and runs the
  matching install command — 37 built-in rules, plus per-repo `[[install.rules]]`
  that are checked first. `dirs` runs detection independently in each listed
  package of a monorepo.
- `[[hooks.pre]]` / `[[hooks.post]]`: arbitrary commands, exec'd directly (not
  through a shell), each optionally scoped to a subdirectory via `dir`.
- `[git]`: an update command (default `git fetch --all --prune`) run before
  everything else.
- Config in TOML or YAML, read from `.herdr/worktree-bootstrap.{toml,yaml,yml}`
  in the repo being bootstrapped. Unknown keys are an error rather than a silent
  default.
- Fail-fast: the first non-zero exit aborts the remaining phases.
- `[notify]`: a herdr toast when the bootstrap finishes, listing what each phase
  did, or the command that failed. Unlike every other section it defaults to on
  (`when = "always"`, also `"failure"` and `"never"`), because herdr captures the
  plugin's stdout rather than printing it — without a toast a run is invisible
  unless you go and read `herdr plugin log list`. Requires `[ui.toast] delivery`
  in herdr's own config; the plugin log says so when herdr suppresses a toast.
- `[failure]`: what becomes of a worktree whose bootstrap aborted.
  `action = "keep"` (default) leaves it alone; `action = "remove"` takes the
  worktree, its workspace and its pane back down, then deletes the branch with a
  non-forcing `git branch -d`. This is as close as a plugin can get to "don't
  create the worktree on failure" — herdr fires plugin events *after* the
  worktree already exists and does not treat the exit code as a veto. `remove`
  is opt-in because the removal is forced, and a bootstrap that runs for minutes
  is a window in which the pane was usable.
- A failure now writes a full report — branch, worktree path, and the whole
  error chain — to the plugin's state directory and opens it in a herdr pane
  running `less`, so the reason is scrollable and untruncated. The path is
  logged whether or not the pane opens.

### Changed

- A failing command's own output is now part of the error. Previously a failure
  reported only its exit status (`` `pnpm install` exited with exit status: 1 ``)
  and the real reason — `ERR_PNPM_ENOENT`, a missing binary, a lockfile mismatch
  — reached nothing but the plugin log. Commands are captured instead of
  inheriting stdio, and replayed verbatim so the log is unchanged; the last 40
  lines go into the error, and the toast keeps the first line and the last few
  of those.
- Replaced the unmaintained `serde_yaml` 0.9 with `serde_yaml_ng` 0.10. No
  behaviour change — the schema, the error messages, and unknown-key rejection
  are identical.

[Unreleased]: https://github.com/serhez/herdr-worktree-sync/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/piesuke/herdr-worktree-bootstrap/releases/tag/v0.1.0
