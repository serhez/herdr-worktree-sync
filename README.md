<div align="center">

# Worktree Sync for Herdr

**Copy, clone, or symlink local files, install dependencies, and run hooks when a worktree is born.**

[![CI](https://github.com/serhez/herdr-worktree-sync/actions/workflows/ci.yml/badge.svg)](https://github.com/serhez/herdr-worktree-sync/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![herdr 0.7.1+](https://img.shields.io/badge/herdr-0.7.1%2B-2b7489)
![platforms: linux | macos](https://img.shields.io/badge/platforms-linux%20%7C%20macos-lightgrey)
![rust edition 2024](https://img.shields.io/badge/rust-edition%202024-dea584)

</div>

A [Herdr](https://github.com/herdrdev/herdr) plugin that prepares freshly
created git worktrees: it copies gitignored files (like `.env`), makes APFS
copy-on-write clones, symlinks shared files or directories, installs
dependencies, and runs your own pre/post commands automatically on
`worktree.created`. Its `sync` action can re-apply all file operations later.

This project is a fork of
[piesuke/herdr-worktree-bootstrap](https://github.com/piesuke/herdr-worktree-bootstrap).
It retains the original project's MIT license and copyright notice.

**Each repository configures its own worktree setup** via a committed
`.worktree-sync.toml`, so different projects can copy different files, install
with different tools, and run different hooks.

```toml
# .worktree-sync.toml — commit this to any repo you want synchronized
[copy]
enabled = true          # bring .env and friends into the new worktree

[install]
enabled = true          # detect the package manager, install

[[hooks.post]]
command = ["direnv", "allow"]
```

## Features

- **Copies what a fresh checkout is missing** — env files and other gitignored
  paths, found recursively via git itself, so nested `.gitignore`s and negations
  just work. Monorepo-safe.
- **Copies, clones, or links explicit paths** — exact paths, directories, and
  glob patterns can use the ownership semantics each path needs.
- **Uses APFS copy-on-write clones** — independent files without an immediate
  second full copy of their data; changed blocks diverge later.
- **Re-applies file operations** — the `sync` plugin action repairs copied,
  cloned, and linked paths without rerunning setup commands.
- **Detects the package manager** — 37 built-in markers, from `bun.lockb` to
  `shard.yml`, with per-repo rules that override them.
- **Runs your own commands** — `pre`/`post` hooks, each optionally scoped to one
  package of a monorepo.
- **Configured per repository** — TOML or YAML, committed alongside the code, so
  every clone and every teammate gets the same setup.
- **Loud about typos** — unknown config keys are an error, not a silent default.
- **Fail-fast** — the first non-zero exit aborts the run, instead of leaving you
  to discover it later.
- **Tells you what it did** — a herdr toast when the worktree is ready, or when
  the setup aborted and why, with the failing command's own output quoted.
- **Shows you the whole error** — a failure also opens a scrollable pane with
  the full report, because a toast is too small for a package manager's answer.
- **Optionally cleans up after itself** — `[failure] action = "remove"` takes a
  worktree whose setup failed back out again.

## Table of contents

- [How it works](#how-it-works)
- [Installing the plugin](#installing-the-plugin)
- [Using it](#using-it)
- [Configuration reference](#configuration-reference)
- [Project layout](#project-layout)
- [Development](#development)
- [Contributing](#contributing)
- [Security note](#security-note)
- [License](#license)

Changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## How it works

```
Herdr (worktree.created)
  └─ ./target/release/herdr-worktree-sync
       │  reads HERDR_PLUGIN_EVENT_JSON (worktree path, branch, source repo_root)
       │  loads the configured per-repo TOML or YAML file
       │
       ├─ git update     bring git up to date (e.g. fetch) — optional
       ├─ pre hooks      commands run before file operations/install
       ├─ copy           <repo>/<path>  ->  <worktree>/<path>
       ├─ clone          <repo>/<path>  =>  <worktree>/<path> (APFS CoW)
       ├─ symlink        <repo>/<path>  ->  <worktree>/<path>
       ├─ install        detect package manager from lockfiles, install
       ├─ post hooks     commands run after file operations + install
       ├─ notify         herdr toast: what ran, or what failed
       └─ on failure     full report in a pane, and optionally a rollback
```

Lifecycle order: **git update → pre → copy → clone → symlink → install → post**.
Any non-zero exit aborts the whole setup (fail-fast). If a repo has no config file, the
plugin does nothing. Whatever the outcome, the run ends with a toast — see
[`[notify]`](#notify--announce-the-result) for the herdr setting it needs — and
a failure additionally opens the
[failure pane](#failure--what-happens-to-a-worktree-that-failed).

### Config format: TOML or YAML

The config can be written as TOML or YAML with the same schema. By default, the
plugin checks one path:

```
.worktree-sync.toml
```

The reference below shows TOML. See
[`examples/worktree-sync.yaml`](examples/worktree-sync.yaml) for the
identical config in YAML.

**Prefer TOML**, including in non-Rust repos. The schemas are identical, but
several fields in this one take values starting with `*` (`patterns`, and
`*.ext` install markers), and in YAML a leading `*` is alias syntax — an
unquoted `marker: *.csproj` is a parse error. TOML also reports unknown-key
errors with the offending line. To use YAML, configure its path explicitly.

**Unknown keys are an error.** A typo like `pattern` instead of `patterns`
aborts the setup with a message naming the offending key and the valid ones,
rather than silently falling back to the default — a config that looks right but
does nothing is the most expensive failure mode here.

### Custom repo config path

The default can be replaced with one repository-relative path in the plugin's
user config:

```toml
# $(herdr plugin config-dir serhez.herdr.worktree.sync)/config.toml
repo_config_path = "config/worktree.yaml"
```

The path must stay inside the repository and end in `.toml`, `.yaml`, or
`.yml`. Only the configured path is checked.

## Installing the plugin

Requires **herdr 0.7.1+** (`min_herdr_version` in the manifest) and a **Rust
1.85+** toolchain — herdr builds the binary from source on your machine, with
the `[[build]]` steps in the manifest. Linux and macOS only.

### Install from GitHub

```sh
herdr plugin install serhez/herdr-worktree-sync
```

### Or link a local checkout (for development)

```sh
git clone git@github.com:serhez/herdr-worktree-sync.git
herdr plugin link ./herdr-worktree-sync        # add --disabled to link without enabling
```

Either way herdr registers the plugin under the id
**`serhez.herdr.worktree.sync`** and records it in
`~/.config/herdr/plugins.json`.

### Build the binary

The manifest declares the build steps (`cargo fetch`, then
`cargo build --release`), which produce `./target/release/herdr-worktree-sync`
— the binary `[[events]]` invokes. If that file doesn't exist after installing,
run the build yourself from the plugin root:

```sh
cargo build --release
```

### Verify

```sh
herdr plugin list
```

The entry should show `enabled: true`. Editing `herdr-plugin.toml` afterwards
does **not** require re-linking — herdr re-reads the manifest from
`manifest_path` on its own.

## Using it

Once installed, the plugin is entirely passive: it runs whenever herdr creates a
worktree, for every repo. What it *does* is decided per repository.

### 1. Add a config to a repo

Nothing happens until a repo has one. Commit `.worktree-sync.toml` to each repo
you want synchronized:

```toml
[copy]
enabled = true
files = [".env", ".env.local"]

[install]
enabled = true

[[hooks.post]]
command = ["direnv", "allow"]
```

See [`examples/worktree-sync.toml`](examples/worktree-sync.toml) for the full schema and
the [configuration reference](#configuration-reference) below for each section.

### 2. Create a worktree

Create a worktree of that repo in herdr as usual. The plugin fires on
`worktree.created` and runs the phases in order.

The full bootstrap runs only when a worktree is created. File operations can be
re-applied later to the focused linked worktree without rerunning hooks or
dependency installation:

```sh
herdr plugin action invoke serhez.herdr.worktree.sync.sync
```

### 3. Read the logs

The plugin's stdout is captured by herdr, not printed to your terminal. To see
what a run did:

```sh
herdr plugin log list --plugin serhez.herdr.worktree.sync --limit 5
```

Each entry carries the `exit_code`, `status`, and full `stdout`/`stderr`. Useful
things you'll see there:

| Output | Meaning |
| ------ | ------- |
| `[worktree-sync] no config (...) in <repo>, nothing to do` | The configured path is absent — step 1 was skipped |
| `[copy] no gitignored files matched [...]` | Discovery ran but found nothing — check the files are actually gitignored |
| `[install] no matching install rule in <dir>, skipping` | No known marker file in that directory |
| `unknown field ...` | A typo in the config; the bootstrap aborted |

### Managing the plugin

```sh
herdr plugin disable serhez.herdr.worktree.sync   # stop it firing, keep it registered
herdr plugin enable  serhez.herdr.worktree.sync
herdr plugin unlink  serhez.herdr.worktree.sync   # remove a linked local checkout
herdr plugin uninstall serhez.herdr.worktree.sync # remove an installed copy
```

## Configuration reference

By default, the config lives at `.worktree-sync.toml` in each repository.
`repo_config_path` can replace it with a custom TOML or YAML path.

### `[git]` — update git first

Runs before everything else. Handy so a new worktree starts from the latest
remote state.

```toml
[git]
update = true
# command = ["git", "fetch", "--all", "--prune"]   # default; override as needed
```

Set `update = true` to run the update; `command` overrides the default
`git fetch --all --prune` (e.g. `["git", "pull", "--ff-only"]`).

### `[copy]` — copy files into the worktree

Brings gitignored files (env files, local secrets) that a fresh checkout won't
have into the new worktree. There are two modes.

**Discovery mode (default).** Omit `files`, and the plugin recursively finds
**gitignored** files in the source repo whose name matches `patterns` and copies
each to the same relative path in the worktree:

```toml
[copy]
enabled = true
# patterns = [".env", ".env.*"]   # omit for these env defaults; `*` = any chars
```

This is the right model for monorepos, where env files live in subdirectories
(`apps/web/.env`, `packages/db/.env.local`). Two properties make it safe:

- **Recursive** — files at any depth are found, not just the repo root.
- **Only gitignored files** — "is this gitignored?" is answered by git itself
  (`git ls-files`), so nested `.gitignore`s, negations, and globs all work.
  Committed files like `.env.example` are **never** copied: they aren't
  gitignored, and the fresh checkout already has them. Wholly-ignored
  directories (e.g. `node_modules/`) are skipped, not walked into.

**Explicit mode.** Set `files` to relative paths or glob patterns. Files are
copied, directories are copied recursively, and missing matches are skipped;
discovery is then disabled:

```toml
[copy]
enabled = true
files = [".env", "config/*.local.toml", "fixtures"]
```

Existing destinations are replaced. When a directory is copied, unrelated
files already present in its destination are retained while matching entries
are updated. Source symlinks are preserved rather than followed. Repository
root and `.git` matches are rejected to protect worktree metadata.

### `[clone]` — make APFS copy-on-write clones

Creates independent files at the same paths while initially sharing their data
blocks with the primary checkout. Writes to either side use new blocks and do
not change the other file:

```toml
[clone]
enabled = true
files = ["models/*.bin", "fixtures", ".venv"]
```

Entries use the same exact paths, directories, and glob patterns as explicit
copy mode. Directories are walked and each regular file is cloned separately;
source symlinks are preserved. Existing destinations are replaced, while
unrelated files in destination directories are retained.

This phase is available only on macOS and both paths must be on volumes that
support `clonefile(2)`, normally APFS. It deliberately fails on Linux,
non-clone-capable filesystems, or cross-volume operations instead of silently
falling back to a full copy. Repository root, `.git`, absolute paths, and parent
traversal are rejected.

### `[symlink]` — share files and directories with the primary checkout

Creates relative symlinks at the same paths in the linked worktree. Entries
accept the same exact relative paths and glob patterns as explicit copy mode:

```toml
[symlink]
enabled = true
files = [".pnpm-store", ".next/cache", "config/*.local"]
```

Files reached through these links are shared: changing one from any worktree
changes the primary checkout's copy. Existing destinations are replaced when
the phase runs, which lets the manual `sync` action repair stale links. Parent
traversal and absolute paths are rejected.
Repository root and `.git` matches are rejected as well.

### `[install]` — install dependencies

Detects the package manager from lockfiles/manifests present in the worktree
and runs the matching install command. The first matching marker wins; a
`*.ext` marker matches any file with that extension.

```toml
[install]
enabled = true
```

**Detection is not recursive.** By default only the worktree root is examined,
and exactly one install command runs. For a monorepo, list the packages to
install in `dirs` — detection then runs independently in each, so several
install commands can run:

```toml
[install]
enabled = true
dirs = ["apps/web", "services/api", "ml"]   # relative to the worktree root
```

A JS monorepo with a root `package.json` and lockfile needs no `dirs`: the root
install already handles workspaces. `dirs` is for the polyglot case — a node app
next to a go service next to a python package — which would otherwise install
nothing at all.

Paths are not globbed (`apps/*` won't expand), and a listed directory that
doesn't exist aborts the bootstrap before anything runs, rather than being
skipped — a typo there means the config is wrong, and installing nothing
silently is exactly what this option exists to prevent.

Built-in detection covers the major languages (checked in this order):

| Language      | Marker file           | Command                                 |
| ------------- | --------------------- | --------------------------------------- |
| JS/TS         | `bun.lockb`           | `bun install`                           |
| JS/TS         | `pnpm-lock.yaml`      | `pnpm install --frozen-lockfile`        |
| JS/TS         | `yarn.lock`           | `yarn install --frozen-lockfile`        |
| JS/TS         | `package-lock.json`   | `npm ci`                                |
| Deno          | `deno.lock`           | `deno install`                          |
| JS/TS         | `package.json`        | `npm install`                           |
| Rust          | `Cargo.toml`          | `cargo fetch`                           |
| Go            | `go.mod`              | `go mod download`                       |
| Python        | `uv.lock`             | `uv sync`                               |
| Python        | `poetry.lock`         | `poetry install`                        |
| Python        | `Pipfile.lock`        | `pipenv install --dev`                  |
| Python        | `requirements.txt`    | `pip install -r requirements.txt`       |
| Ruby          | `Gemfile`             | `bundle install`                        |
| PHP           | `composer.json`       | `composer install`                      |
| Java          | `pom.xml`             | `mvn install -DskipTests`               |
| Kotlin/Gradle | `build.gradle.kts`    | `gradle build -x test`                  |
| Java/Gradle   | `build.gradle`        | `gradle build -x test`                  |
| Scala         | `build.sbt`           | `sbt update`                            |
| C#/.NET       | `*.sln`               | `dotnet restore`                        |
| C#/.NET       | `*.csproj`            | `dotnet restore`                        |
| C/C++         | `vcpkg.json`          | `vcpkg install`                         |
| C/C++         | `conanfile.txt`       | `conan install .`                       |
| C/C++         | `conanfile.py`        | `conan install .`                       |
| Swift         | `Package.swift`       | `swift package resolve`                 |
| Obj-C/Swift   | `Podfile`             | `pod install`                           |
| Dart/Flutter  | `pubspec.yaml`        | `dart pub get`                          |
| Elixir        | `mix.exs`             | `mix deps.get`                          |
| Erlang        | `rebar.config`        | `rebar3 get-deps`                       |
| Haskell       | `stack.yaml`          | `stack build --only-dependencies`       |
| Haskell       | `cabal.project`       | `cabal build --only-dependencies`       |
| R             | `renv.lock`           | `Rscript -e 'renv::restore(...)'`       |
| Perl          | `cpanfile`            | `cpanm --installdeps .`                 |
| Clojure       | `deps.edn`            | `clojure -P`                            |
| Clojure       | `project.clj`         | `lein deps`                             |
| Julia         | `Project.toml`        | `julia --project -e 'Pkg.instantiate()'`|
| Crystal       | `shard.yml`           | `shards install`                        |

**Custom rules** are checked *before* the built-ins, so they can add a language
or override one:

```toml
[[install.rules]]
marker = "flake.nix"
command = ["nix", "develop", "--command", "true"]
```

### `[[hooks.pre]]` / `[[hooks.post]]` — arbitrary commands

Commands run inside the new worktree, in order. `pre` runs before file operations/install;
`post` runs after. Add `dir` to run a hook in one package of a monorepo instead
of at the root:

```toml
[[hooks.pre]]
command = ["mise", "install"]

[[hooks.post]]
command = ["direnv", "allow"]

[[hooks.post]]
command = ["npm", "run", "codegen"]
dir = "apps/web"                      # relative to the worktree root
```

A `dir` that doesn't exist aborts with an explicit error, rather than surfacing
as a confusing "failed to spawn" for the program.

> **Not run through a shell.** Each command is exec'd directly, so `&&`, pipes,
> `$VARS`, redirects, and globs do **not** work. Wrap them yourself:
>
> ```toml
> [[hooks.post]]
> command = ["sh", "-c", "npm run codegen && npm run build"]
> ```
>
> The program is resolved via `PATH`; if it isn't found the bootstrap aborts.

### `[notify]` — announce the result

herdr captures this plugin's stdout instead of printing it, so without a
notification a run is invisible unless you go and read
[the logs](#3-read-the-logs). When the bootstrap finishes, the plugin posts a
herdr toast summarising it:

```
Worktree sync done · worktree/green-harbor-ad23
  updated git
  copied 3 files
  pnpm install --frozen-lockfile
  uv sync
```

```
Worktree sync failed · worktree/green-harbor-ad23
  `pnpm install --frozen-lockfile` exited with exit status: 1
  …
  ERR_PNPM_ENOENT  Failed to create bin at …/node_modules/.bin/next:
  ENOENT: no such file or directory
```

A failure quotes what the command printed, not just its exit status — the exit
status is the one thing you already knew. Only the ends survive the toast (the
command at the top, the reason at the bottom); the whole thing is in the
[failure pane](#failure--what-happens-to-a-worktree-that-failed).

This is the **only section that is on by default**, because a half-bootstrapped
worktree you were never told about is the failure this plugin is supposed to
prevent. Turn it down with:

```toml
[notify]
# when = "always"    # default: after every run in which a phase was enabled
# when = "failure"   # only when the bootstrap aborts
# when = "never"     # stay silent
```

A run where no phase was enabled (a repo with no config) never toasts, whatever
`when` says.

> **herdr must be set to deliver toasts.** They are off in herdr's own default
> config, so until you set `[ui.toast] delivery` in *herdr's* `config.toml`
> nothing appears and `herdr notification show` quietly answers
> `"shown": false` at exit code 0:
>
> ```toml
> # ~/.config/herdr/config.toml
> [ui.toast]
> delivery = "herdr"     # in-app toast; or "terminal" / "system" for desktop
> ```
>
> herdr can still decline a toast after that — `[ui.toast]` is *background*
> notification delivery, so it suppresses popups for whatever you are already
> looking at. Either way the plugin log names the reason it got back, rather
> than leaving you to wonder whether the notification was ever sent:
>
> ```
> [notify] herdr did not show the toast (reason: disabled) — set `[ui.toast] delivery` in herdr's config.toml to see it
> [notify] herdr did not show the toast (reason: busy)
> ```

### `[failure]` — what happens to a worktree that failed

Every failure writes a **full report** — the branch, the worktree path, and the
complete error chain including the failing command's output — and opens it in a
herdr pane:

```
Worktree sync failed

branch:   worktree/green-harbor-ad23
worktree: /Users/you/.herdr/worktrees/app/worktree-green-harbor-ad23

`pnpm install --frozen-lockfile` exited with exit status: 1
Progress: resolved 576, reused 574, downloaded 0, added 0
ERR_PNPM_ENOENT  Failed to create bin at …/node_modules/.bin/next: ENOENT: no such file or directory
```

The pane is a real terminal running `less`, so it scrolls and nothing is cut —
press `q` to close it. The file stays behind either way, and its path is in
[the logs](#3-read-the-logs):

```
[report] full failure report: ~/.local/state/herdr/plugins/serhez.herdr.worktree.sync/failure-worktree-green-harbor-ad23.log
```

The worktree itself is **kept** by default:

```toml
[failure]
# action = "keep"     # default: leave the half-bootstrapped worktree alone
# action = "remove"   # take the worktree, its workspace and its pane back down
```

`remove` is the closest this plugin can get to "don't create the worktree if
the bootstrap fails". herdr fires plugin events *after* the worktree, workspace
and pane already exist, and the event's exit code is not a veto — so the only
option is to undo them, which is what `remove` does (it also deletes the branch,
via a non-forcing `git branch -d` that refuses if you committed something).

> **`remove` is destructive, which is why it is not the default.** A bootstrap
> can run for minutes on a large repo, and the agent pane is usable the entire
> time. The removal is forced — it has to be, or the half-written `node_modules`
> that caused the failure would block it — so anything you typed or wrote in
> that window goes with it.

## Project layout

```
├── Cargo.toml
├── herdr-plugin.toml        # plugin manifest (generic, no per-repo settings)
├── examples/
│   ├── worktree-sync.toml   # sample config for consumers to copy
│   └── worktree-sync.yaml   # the same config in YAML
├── src/
│   ├── action.rs            # manual file synchronization
│   ├── main.rs              # binary: parse the event, then hand off to run()
│   ├── lib.rs               # run(): the bootstrap lifecycle
│   ├── event.rs             # HERDR_PLUGIN_EVENT_JSON types
│   ├── config.rs            # plugin settings and per-repo config loading
│   ├── bootstrap.rs         # copy / clone / symlink / install / hook execution
│   ├── herdr.rs             # which herdr binary the plugin calls back into
│   ├── notify.rs            # the end-of-run herdr toast
│   ├── report.rs            # the full failure report and the pane showing it
│   └── rollback.rs          # `[failure] action = "remove"`
└── tests/
    ├── common/mod.rs        # temp dirs and throwaway git repos
    ├── config_load.rs       # which config file wins; the examples still parse
    ├── copy.rs              # discovery and explicit copying
    ├── clone.rs             # APFS copy-on-write cloning
    ├── symlink.rs           # declarative shared paths
    ├── sync.rs              # manual action context and behavior
    ├── lifecycle.rs         # phase order and fail-fast
    └── manifest.rs          # Cargo.toml and herdr-plugin.toml agree
```

The two manifests are read by different tools — `Cargo.toml` by cargo,
`herdr-plugin.toml` by herdr — and neither reads the other, so their overlap
(the version, the binary's name and path) is hand-maintained and drifts
silently. `tests/manifest.rs` is the handshake: bump the version in one file
only, or rename the binary, and CI says so instead of the plugin failing the
next time herdr fires the event.

The crate is a library plus a thin binary. Everything that decides *what
happens* lives in the library, so the lifecycle can be tested without herdr in
the loop; `main.rs` selects the creation event or manual action, loads the
repo's config, and calls the corresponding library entrypoint.

## Development

```sh
cargo test           # unit + integration
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Unit tests live beside the code they cover, in `#[cfg(test)] mod tests` at the
bottom of each module — that's what gives them access to the private pieces
worth pinning, like the glob matcher and the install-detection table.
Integration tests in `tests/` drive the public API instead, and they don't mock:
the copy tests create real git repositories and the lifecycle tests run real
shell hooks, because delegating to `git ls-files` and exec'ing commands *is* the
behaviour under test.

**Linux and macOS only.** The integration tests shell out to `sh`, and the
manifest declares those two platforms. The `[clone]` phase itself is macOS-only.

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and the test suite
on Linux and macOS, so the three commands above are the whole gate.

Two things worth knowing before you open a PR:

- **Bump both manifests together.** The version lives in `Cargo.toml` *and*
  `herdr-plugin.toml`; `tests/manifest.rs` fails if they disagree.
- **Adding an install rule?** Add it to `BUILTIN_RULES` in `src/bootstrap.rs`,
  to the table above, and to the detection tests — order matters, because the
  first matching marker wins.

## Security note

Because hooks and install commands come from the repo's bootstrap config,
**anyone who can push to a repo can run arbitrary commands** when a worktree of
it is created — the same trust model as
`.git/hooks` or CI config. Only enable automatic bootstrap for repositories you
trust.

See [SECURITY.md](SECURITY.md) for the full trust model and how to report a
vulnerability.

## License

[MIT](LICENSE) © piesuke. Fork maintained by serhez.
