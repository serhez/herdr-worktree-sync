# Security Policy

## Supported versions

This plugin is pre-1.0. Only the latest release receives fixes; there are no
backports.

## The trust model, stated up front

Some things that look like vulnerabilities are the documented design, and
reporting them privately only delays the answer. Specifically:

**A repo's bootstrap config can run arbitrary commands.** Hooks and custom
install rules are command lines, executed when a worktree of that repo is
created. Anyone who can push to a repo — or who sends you a branch you check out
as a worktree — can therefore run code on your machine. This is the same trust
model as `.git/hooks`, `Makefile`, or a CI config, and it is the feature. Only
bootstrap repositories you trust.

**Copied files are secrets by construction.** The copy phase exists to move
gitignored files such as `.env` into a new worktree, so those secrets are
written to a second location on disk. Worktrees you delete still had them.

**Symlinked paths are shared state.** A write through a worktree symlink changes
the primary checkout's file or directory. Use copies for branch-specific or
untrusted state, and reserve symlinks for paths meant to be shared.

**Commands are not run through a shell**, so there is no shell-injection surface
from config values — but `["sh", "-c", ...]` is a documented escape hatch, and
anything inside it is your responsibility.

## Reporting a vulnerability

For anything outside the above — a path traversal that writes outside the
worktree, a config value that escapes the argv boundary, a secret leaked into
logs — please report it privately:

1. Open a [private security advisory](https://github.com/serhez/herdr-worktree-sync/security/advisories/new)
   on this repository. Do **not** open a public issue.
2. Include the config that triggers it, the repo layout, your OS, and what you
   expected instead.

Expect an initial response within 7 days. As a single-maintainer hobby project
there is no formal SLA beyond that; a fix ships as a patch release with the
advisory published alongside it.

## Scope

In scope: this plugin's own code — the copy, symlink, install, hook, and
config-loading paths.

Out of scope: [herdr](https://github.com/herdrdev/herdr) itself (report there),
and the package managers this plugin invokes on your behalf.
