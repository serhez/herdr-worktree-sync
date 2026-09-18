# Contributing

Thanks for taking an interest. Issues and pull requests are both welcome.

## Getting set up

```sh
git clone git@github.com:serhez/herdr-worktree-sync.git
cd herdr-worktree-sync
cargo test
```

A stable Rust toolchain is all you need — no herdr installation required to
work on this. The lifecycle lives in the library, so `cargo test` drives it
directly without an event payload.

**Linux and macOS only.** The integration tests shell out to `sh`, and
`herdr-plugin.toml` declares those two platforms. Windows was dropped
deliberately rather than half-supported.

## The gate

CI runs exactly these three, on Linux and macOS. If they pass locally they pass
there:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
```

Note the `-D warnings`: CI sets `RUSTFLAGS: -D warnings` globally, so a warning
anywhere — including in `tests/` — fails the build.

## Where things live

| Path | What it holds |
| ---- | ------------- |
| `src/lib.rs` | `run()` — the phase order and fail-fast behaviour |
| `src/bootstrap.rs` | copy / install / hook execution, and `BUILTIN_RULES` |
| `src/config.rs` | the config schema and configured path loading |
| `src/event.rs` | `HERDR_PLUGIN_EVENT_JSON` types |
| `src/main.rs` | a thin wrapper: parse the event, call `run()` |

Keep `main.rs` thin. It is the one part the test suite cannot reach, so logic
that lands there is logic that stops being covered.

## Tests

Unit tests go inline, in `#[cfg(test)] mod tests` at the bottom of the module
they cover — that is what gives them access to the private pieces worth
pinning, like the glob matcher and the install-detection table. Integration
tests in `tests/` drive the public API.

**Don't mock git or the shell.** The copy tests create real repositories and the
lifecycle tests run real hooks, because delegating to `git ls-files` and exec'ing
commands *is* the behaviour under test. `tests/common/mod.rs` has the helpers;
note that `init_repo()` sets `core.excludesFile=/dev/null` so your personal
global gitignore can't change the result.

Name the test after what it asserts, in a sentence:
`a_failing_pre_hook_aborts_before_copy`, not `test_hooks_2`.

## Two things that bite

**Bump both manifests together.** The version lives in `Cargo.toml` *and*
`herdr-plugin.toml`, and neither tool reads the other's file. `tests/manifest.rs`
fails if they disagree — that test is the only thing standing between a
half-bumped version and a plugin that breaks the next time herdr fires an event.

**Adding an install rule** means three edits, not one:

1. `BUILTIN_RULES` in `src/bootstrap.rs` — **order matters**, the first matching
   marker wins, so a lockfile must come before the manifest it locks
   (`pnpm-lock.yaml` before `package.json`).
2. The detection table in `README.md`.
3. A case in the detection tests.

## Commit messages

Per `AGENTS.md`: the code says *how*, the tests say *what*, the commit log says
*why*, and code comments say *why not*. A commit message that restates the diff
is a wasted one — say what made the change necessary.

## Changing the config schema

`Config` uses `deny_unknown_fields`, so **adding a field is safe but removing or
renaming one is breaking**: an existing repo's committed config starts failing
to parse, and the bootstrap aborts. Pre-1.0 that is acceptable, but it belongs
in the changelog and in the PR description.

Any schema change also needs both examples updated —
`examples/worktree-sync.toml` and `examples/worktree-sync.yaml` are
parsed by the test suite and asserted to be equivalent, so they cannot drift.

Note it in `CHANGELOG.md` under **Unreleased** in the same PR, while you still
remember why.

## Releasing (maintainers)

Pushing a `v*` tag is the whole release. `.github/workflows/release.yml` refuses
to publish unless the tag, `Cargo.toml`, and `herdr-plugin.toml` all name the
same version and the changelog has a section for it — so the order below
matters.

1. **Rename the changelog section.** `## [Unreleased]` → `## [0.2.0] - 2026-09-15`,
   and add a fresh empty `## [Unreleased]` above it. This becomes the release
   body verbatim; the workflow fails if the section is empty.
2. **Bump the version in both manifests** — `Cargo.toml` and
   `herdr-plugin.toml`. `tests/manifest.rs` catches it if you do only one.
3. **Run `cargo build`** so `Cargo.lock` picks up the new version, and commit it.
   CI runs `--locked` and will fail on a stale lockfile.
4. Merge to `main`, then:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The workflow then re-runs the suite against the tagged commit on both platforms
and creates the GitHub release. A version with a `-` in it (`v0.2.0-rc.1`) is
published as a prerelease automatically.

No binaries are attached: herdr installs from source and runs the manifest's own
`cargo build --release`, so the release is a changelog entry and an installable
ref, nothing more.

**If the tag was wrong**, delete it (`git push --delete origin v0.2.0`), fix, and
re-tag — but only before the release is published. After that, ship a new
patch version instead.
