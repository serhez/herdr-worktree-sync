//! Worktree file operations, dependency installation, and hooks.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::{CommandConfig, InstallRule};

/// Default git update command when `[git] update = true` and no override given.
const DEFAULT_GIT_UPDATE: &[&str] = &["git", "fetch", "--all", "--prune"];

/// How many trailing lines of a failed command's output to quote in the error.
///
/// Sized for the pane and the report file, not the toast — the tail is where
/// package managers print the actual reason, and the two lines before it are
/// usually the path or the dependency it was working on. [`crate::notify`]
/// shortens this again for the toast.
const FAILURE_TAIL_LINES: usize = 40;

/// Default filename globs for recursive discovery when `[copy]` is enabled but
/// neither `files` nor `patterns` is set. Matches `.env` and `.env.<anything>`
/// (`.env.local`, `.env.production.local`, …) at any depth in the repo.
const DEFAULT_ENV_PATTERNS: &[&str] = &[".env", ".env.*"];

/// Built-in install rules, checked *after* any user-defined rules. The first
/// matching marker wins, so more specific lockfiles precede generic manifests.
/// A marker of the form `*.ext` matches any file with that extension in the
/// worktree root. Covers the ~top-20 languages by usage that have a package /
/// dependency manager.
const BUILTIN_RULES: &[(&str, &[&str])] = &[
    // JavaScript / TypeScript / Node
    ("bun.lockb", &["bun", "install"]),
    ("pnpm-lock.yaml", &["pnpm", "install", "--frozen-lockfile"]),
    ("yarn.lock", &["yarn", "install", "--frozen-lockfile"]),
    ("package-lock.json", &["npm", "ci"]),
    ("deno.lock", &["deno", "install"]),
    ("package.json", &["npm", "install"]),
    // Rust
    ("Cargo.toml", &["cargo", "fetch"]),
    // Go
    ("go.mod", &["go", "mod", "download"]),
    // Python
    ("uv.lock", &["uv", "sync"]),
    ("poetry.lock", &["poetry", "install"]),
    ("Pipfile.lock", &["pipenv", "install", "--dev"]),
    (
        "requirements.txt",
        &["pip", "install", "-r", "requirements.txt"],
    ),
    // Ruby
    ("Gemfile", &["bundle", "install"]),
    // PHP
    ("composer.json", &["composer", "install"]),
    // Java / Kotlin / Scala (JVM)
    ("pom.xml", &["mvn", "install", "-DskipTests"]),
    ("build.gradle.kts", &["gradle", "build", "-x", "test"]),
    ("build.gradle", &["gradle", "build", "-x", "test"]),
    ("build.sbt", &["sbt", "update"]),
    // C# / .NET
    ("*.sln", &["dotnet", "restore"]),
    ("*.csproj", &["dotnet", "restore"]),
    // C / C++
    ("vcpkg.json", &["vcpkg", "install"]),
    ("conanfile.txt", &["conan", "install", "."]),
    ("conanfile.py", &["conan", "install", "."]),
    // Swift / Objective-C
    ("Package.swift", &["swift", "package", "resolve"]),
    ("Podfile", &["pod", "install"]),
    // Dart / Flutter
    ("pubspec.yaml", &["dart", "pub", "get"]),
    // Elixir
    ("mix.exs", &["mix", "deps.get"]),
    // Erlang
    ("rebar.config", &["rebar3", "get-deps"]),
    // Haskell
    ("stack.yaml", &["stack", "build", "--only-dependencies"]),
    ("cabal.project", &["cabal", "build", "--only-dependencies"]),
    // R
    (
        "renv.lock",
        &["Rscript", "-e", "renv::restore(prompt = FALSE)"],
    ),
    // Perl
    ("cpanfile", &["cpanm", "--installdeps", "."]),
    // Clojure
    ("deps.edn", &["clojure", "-P"]),
    ("project.clj", &["lein", "deps"]),
    // Julia
    (
        "Project.toml",
        &["julia", "--project", "-e", "using Pkg; Pkg.instantiate()"],
    ),
    // Crystal
    ("shard.yml", &["shards", "install"]),
];

/// Update git before the rest of the bootstrap (defaults to `git fetch`).
pub fn git_update(worktree: &Path, command: Option<&[String]>) -> Result<()> {
    match command {
        Some(argv) => run_command(worktree, argv),
        None => {
            let owned: Vec<String> = DEFAULT_GIT_UPDATE.iter().map(|s| s.to_string()).collect();
            run_command(worktree, &owned)
        }
    }
}

/// Copy an explicit list of relative paths from the source repo into the
/// worktree. Missing source files are skipped, not errors. Returns how many
/// files were actually copied, which is what the end-of-run toast reports.
pub fn copy_files(source: &Path, worktree: &Path, files: &[String]) -> Result<usize> {
    let mut copied = 0usize;
    for (src, relative) in expand_file_entries(source, files)? {
        copied += copy_into_worktree(&src, worktree, &relative)?;
        println!("[copy] {}", relative.display());
    }
    Ok(copied)
}

/// Create copy-on-write clones of relative files, directories, or glob matches.
/// Existing destinations are replaced. The operation is deliberately strict:
/// unsupported platforms or filesystems return an error rather than silently
/// falling back to a byte-for-byte copy.
pub fn clone_files(source: &Path, worktree: &Path, files: &[String]) -> Result<usize> {
    let entries = expand_file_entries(source, files)?;
    ensure_clone_platform_supported()?;

    let mut cloned = 0usize;
    for (src, relative) in entries {
        prepare_destination_parent(worktree, &relative)?;
        cloned += clone_entry(&src, &worktree.join(&relative))?;
        println!("[clone] {}", relative.display());
    }
    Ok(cloned)
}

#[cfg(target_os = "macos")]
fn ensure_clone_platform_supported() -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn ensure_clone_platform_supported() -> Result<()> {
    bail!("APFS file cloning is only supported on macOS")
}

/// Recursively reproduce a filesystem entry, cloning each regular file rather
/// than asking `clonefile(2)` to clone a directory hierarchy. Apple explicitly
/// discourages using that API recursively on a directory.
#[cfg(target_os = "macos")]
fn clone_entry(src: &Path, dst: &Path) -> Result<usize> {
    let metadata = src
        .symlink_metadata()
        .with_context(|| format!("reading metadata for {}", src.display()))?;

    if metadata.file_type().is_symlink() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        remove_destination(dst)?;
        let target =
            fs::read_link(src).with_context(|| format!("reading symlink {}", src.display()))?;
        std::os::unix::fs::symlink(&target, dst)
            .with_context(|| format!("cloning symlink {}", src.display()))?;
        return Ok(1);
    }

    if metadata.is_dir() {
        if let Ok(destination_metadata) = dst.symlink_metadata()
            && !destination_metadata.is_dir()
        {
            remove_destination(dst)?;
        }
        fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
        let mut cloned = 0;
        for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
            let entry = entry?;
            cloned += clone_entry(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(cloned);
    }

    if metadata.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        remove_destination(dst)?;
        clone_regular_file(src, dst)?;
        return Ok(1);
    }

    println!("[clone] skip (unsupported file type): {}", src.display());
    Ok(0)
}

#[cfg(not(target_os = "macos"))]
fn clone_entry(_src: &Path, _dst: &Path) -> Result<usize> {
    unreachable!("platform support is checked before cloning entries")
}

#[cfg(target_os = "macos")]
fn clone_regular_file(src: &Path, dst: &Path) -> Result<()> {
    use std::ffi::{CString, c_char};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn clonefile(src: *const c_char, dst: *const c_char, flags: u32) -> i32;
    }

    let src_c = CString::new(src.as_os_str().as_bytes())
        .with_context(|| format!("source path contains a NUL byte: {}", src.display()))?;
    let dst_c = CString::new(dst.as_os_str().as_bytes())
        .with_context(|| format!("destination path contains a NUL byte: {}", dst.display()))?;

    // SAFETY: both pointers come from live `CString`s, are NUL-terminated, and
    // remain valid for the duration of the call. The destination was removed
    // immediately above because `clonefile` requires it not to exist.
    if unsafe { clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) } == -1 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "APFS-cloning {} to {} (both paths must be on a clone-capable volume)",
                src.display(),
                dst.display()
            )
        });
    }
    Ok(())
}

/// Symlink relative paths or glob matches from the source repo into a
/// worktree. Existing destinations are replaced so the operation can repair a
/// stale or deleted link when manually re-applied.
pub fn symlink_files(source: &Path, worktree: &Path, files: &[String]) -> Result<usize> {
    let entries = expand_file_entries(source, files)?;
    for (src, relative) in &entries {
        prepare_destination_parent(worktree, relative)?;
        let destination = worktree.join(relative);
        remove_destination(&destination)?;

        let parent = destination
            .parent()
            .context("symlink destination has no parent directory")?;
        let target = pathdiff::diff_paths(src, parent)
            .context("could not make symlink target relative to its destination")?;
        std::os::unix::fs::symlink(&target, &destination).with_context(|| {
            format!(
                "symlinking {} -> {}",
                destination.display(),
                target.display()
            )
        })?;
        println!("[symlink] {}", relative.display());
    }
    Ok(entries.len())
}

/// Resolve exact relative paths and glob patterns without allowing a pattern
/// to reach outside the source repository. If a directory and one of its
/// descendants both match, keep only the directory operation; processing both
/// would duplicate a copy and could traverse a symlink created moments ago.
fn expand_file_entries(source: &Path, patterns: &[String]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut matches = BTreeMap::new();
    for pattern in patterns {
        validate_relative_pattern(pattern)?;
        let full_pattern = source.join(pattern);
        let full_pattern = full_pattern
            .to_str()
            .with_context(|| format!("file pattern is not valid UTF-8: {pattern}"))?;
        let mut matched = false;
        for entry in
            glob::glob(full_pattern).with_context(|| format!("invalid file pattern `{pattern}`"))?
        {
            let path = entry.with_context(|| format!("expanding file pattern `{pattern}`"))?;
            let relative = path
                .strip_prefix(source)
                .with_context(|| format!("matched path escaped source repo: {}", path.display()))?
                .to_path_buf();
            validate_file_operation_match(&relative)?;
            matches.insert(relative, path);
            matched = true;
        }
        if !matched {
            println!("[files] skip (no matches): {pattern}");
        }
    }

    let mut candidates: Vec<_> = matches.into_iter().collect();
    candidates.sort_by(|(left, _), (right, _)| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut directory_roots = Vec::new();
    let mut entries = Vec::new();
    for (relative, path) in candidates {
        if directory_roots
            .iter()
            .any(|root: &PathBuf| relative.starts_with(root))
        {
            continue;
        }
        if path
            .symlink_metadata()
            .with_context(|| format!("reading metadata for {}", path.display()))?
            .is_dir()
        {
            directory_roots.push(relative.clone());
        }
        entries.push((path, relative));
    }
    Ok(entries)
}

fn validate_relative_pattern(pattern: &str) -> Result<()> {
    let path = Path::new(pattern);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        bail!("file pattern must be relative to the source repo: `{pattern}`");
    }
    Ok(())
}

fn validate_file_operation_match(relative: &Path) -> Result<()> {
    let first = relative.components().next();
    if first.is_none() || first == Some(Component::Normal(".git".as_ref())) {
        bail!(
            "file operations cannot replace Git metadata: {}",
            relative.display()
        );
    }
    Ok(())
}

/// Recursively discover **gitignored** files in the source repo whose basename
/// matches one of `patterns` (default: env files) and copy each to the same
/// relative path in the worktree. This is the right model for env files in a
/// monorepo: they live in subdirectories and are gitignored, so a fresh
/// checkout won't have them — while committed files like `.env.example` are
/// left alone because they aren't gitignored.
///
/// "Which files are gitignored" is answered by git itself (`git ls-files`), so
/// nested `.gitignore` files, negations, and globs are all honored correctly.
pub fn copy_gitignored(
    source: &Path,
    worktree: &Path,
    patterns: Option<&[String]>,
) -> Result<usize> {
    let default: Vec<String>;
    let patterns: &[String] = match patterns {
        Some(p) => p,
        None => {
            default = DEFAULT_ENV_PATTERNS.iter().map(|s| s.to_string()).collect();
            &default
        }
    };

    // `--others --ignored --exclude-standard` lists working-tree files that git
    // ignores; `--directory` collapses wholly-ignored dirs (e.g. node_modules/)
    // to a single entry so we don't walk into them. `-z` is NUL-delimited to
    // survive odd filenames.
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--full-name",
        ])
        .output()
        .context("running `git ls-files` to discover gitignored files")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!(
            "[copy] skip discovery: `git ls-files` failed ({})",
            stderr.trim()
        );
        return Ok(0);
    }

    let mut copied = 0usize;
    for entry in output.stdout.split(|b| *b == 0) {
        if entry.is_empty() {
            continue;
        }
        let rel = String::from_utf8_lossy(entry);
        // A trailing slash means a collapsed ignored directory — skip it.
        if rel.ends_with('/') {
            continue;
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        if !patterns.iter().any(|pat| glob_match(pat, name)) {
            continue;
        }

        let src = source.join(&*rel);
        copy_into_worktree(&src, worktree, Path::new(&*rel))?;
        println!("[copy] {rel}");
        copied += 1;
    }

    if copied == 0 {
        println!("[copy] no gitignored files matched {patterns:?}");
    }
    Ok(copied)
}

/// Copy `src` to `worktree/rel`, creating parent directories as needed.
fn copy_into_worktree(src: &Path, worktree: &Path, rel: &Path) -> Result<usize> {
    prepare_destination_parent(worktree, rel)?;
    let dst = worktree.join(rel);
    copy_entry(src, &dst)
}

/// Ensure every destination parent is a real directory inside the worktree.
/// A stale link from an earlier configuration must be removed before a nested
/// operation, or filesystem calls would follow it outside the worktree.
fn prepare_destination_parent(worktree: &Path, relative: &Path) -> Result<()> {
    let mut current = worktree.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => current.push(name),
            _ => bail!(
                "destination path must be relative to the worktree: {}",
                relative.display()
            ),
        }

        match current.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                remove_destination(&current)?;
                fs::create_dir(&current)
                    .with_context(|| format!("creating {}", current.display()))?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .with_context(|| format!("creating {}", current.display()))?;
            }
            Err(err) => {
                return Err(err).with_context(|| format!("inspecting {}", current.display()));
            }
        }
    }
    Ok(())
}

/// Recursively copy one filesystem entry. Existing directories are merged;
/// conflicting files, directories, or links are replaced without following
/// destination symlinks.
fn copy_entry(src: &Path, dst: &Path) -> Result<usize> {
    let metadata = src
        .symlink_metadata()
        .with_context(|| format!("reading metadata for {}", src.display()))?;

    if metadata.file_type().is_symlink() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        remove_destination(dst)?;
        let target =
            fs::read_link(src).with_context(|| format!("reading symlink {}", src.display()))?;
        std::os::unix::fs::symlink(&target, dst)
            .with_context(|| format!("copying symlink {} -> {}", src.display(), dst.display()))?;
        return Ok(1);
    }

    if metadata.is_dir() {
        if let Ok(destination_metadata) = dst.symlink_metadata()
            && !destination_metadata.is_dir()
        {
            remove_destination(dst)?;
        }
        fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
        let mut copied = 0;
        for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
            let entry = entry?;
            copied += copy_entry(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(copied);
    }

    if metadata.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        remove_destination(dst)?;
        fs::copy(src, dst)
            .with_context(|| format!("copying {} -> {}", src.display(), dst.display()))?;
        return Ok(1);
    }

    println!("[copy] skip (unsupported file type): {}", src.display());
    Ok(0)
}

fn remove_destination(path: &Path) -> Result<()> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("inspecting {}", path.display()));
        }
    };
    if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// Glob match against a filename. Supports `*` (any sequence, including empty);
/// no `?` or character classes. Used to match discovered basenames.
fn glob_match(pattern: &str, name: &str) -> bool {
    fn helper(pat: &[u8], name: &[u8]) -> bool {
        match pat.split_first() {
            None => name.is_empty(),
            Some((b'*', rest)) => (0..=name.len()).any(|i| helper(rest, &name[i..])),
            Some((c, rest)) => name.first() == Some(c) && helper(rest, &name[1..]),
        }
    }
    helper(pattern.as_bytes(), name.as_bytes())
}

/// Install dependencies inside the worktree.
///
/// Detection runs independently in each of `dirs` (relative to the worktree
/// root, defaulting to the root itself), so a polyglot monorepo can install a
/// node app and a go service in one bootstrap. Within a directory the first
/// matching marker wins and exactly one command runs there.
///
/// A listed directory that doesn't exist aborts: `dirs` describes committed
/// repo structure, so a missing one means the config is wrong, and silently
/// installing nothing is the failure mode this option exists to fix.
///
/// Returns the command run in each directory, in order. An empty vec means
/// every directory was checked and none matched a rule — a distinction the
/// end-of-run toast makes, because "installed nothing" is usually a surprise.
pub fn install_deps(
    worktree: &Path,
    rules: &[InstallRule],
    dirs: Option<&[String]>,
) -> Result<Vec<String>> {
    let default: Vec<String>;
    let dirs: &[String] = match dirs {
        Some(dirs) => dirs,
        None => {
            default = vec![".".to_string()];
            &default
        }
    };

    // Validate every directory before installing anything: a typo in the last
    // entry should not leave the earlier packages half-installed.
    for dir in dirs {
        if !worktree.join(dir).is_dir() {
            bail!("install dir `{dir}` does not exist in the worktree");
        }
    }

    let mut installed = Vec::new();
    for dir in dirs {
        if let Some(argv) = install_in_dir(&worktree.join(dir), dir, rules)? {
            installed.push(argv.join(" "));
        }
    }
    Ok(installed)
}

/// Decide which install command applies in `base`, without running anything.
/// User-defined `rules` are checked first (so they can add a language or
/// override a built-in), then [`BUILTIN_RULES`]; the first matching marker
/// wins. `None` means nothing matched.
///
/// Detection is split from execution so the whole table — and the precedence
/// between its entries — can be tested without spawning a process.
fn detect_install(base: &Path, rules: &[InstallRule]) -> Option<Vec<String>> {
    for rule in rules {
        if marker_matches(base, &rule.marker) {
            return Some(rule.command.clone());
        }
    }
    BUILTIN_RULES
        .iter()
        .find(|(marker, _)| marker_matches(base, marker))
        .map(|(_, argv)| argv.iter().map(|s| s.to_string()).collect())
}

/// Detect and install in a single directory, returning the command it ran.
fn install_in_dir(base: &Path, label: &str, rules: &[InstallRule]) -> Result<Option<Vec<String>>> {
    match detect_install(base, rules) {
        Some(argv) => {
            run_command(base, &argv)?;
            Ok(Some(argv))
        }
        None => {
            println!("[install] no matching install rule in {label}, skipping");
            Ok(None)
        }
    }
}

/// Does a marker match in the worktree root? A `*.ext` marker matches any file
/// with that extension; anything else is an exact filename.
fn marker_matches(worktree: &Path, marker: &str) -> bool {
    if let Some(ext) = marker.strip_prefix("*.") {
        let suffix = format!(".{ext}");
        std::fs::read_dir(worktree).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().ends_with(&suffix))
        })
    } else {
        worktree.join(marker).exists()
    }
}

/// Run a list of hook commands in order. Any non-zero exit aborts. A hook's
/// `dir` selects a subdirectory of the worktree to run in (default: the root).
/// Returns how many ran, which on the happy path is all of them.
pub fn run_hooks(worktree: &Path, hooks: &[CommandConfig]) -> Result<usize> {
    for hook in hooks {
        let cwd = match &hook.dir {
            Some(dir) => worktree.join(dir),
            None => worktree.to_path_buf(),
        };
        run_command(&cwd, &hook.command)?;
    }
    Ok(hooks.len())
}

/// Run one command in `cwd`. Non-zero exit aborts, and the error carries the
/// tail of what the command printed.
///
/// Crate-internal: callers outside the library drive the phases through
/// [`crate::run`], not individual commands.
pub(crate) fn run_command(cwd: &Path, argv: &[String]) -> Result<()> {
    let Some((program, rest)) = argv.split_first() else {
        bail!("empty command in config");
    };
    // Checked up front: a bad `dir` would otherwise surface as a spawn failure
    // indistinguishable from "the program isn't installed".
    if !cwd.is_dir() {
        bail!(
            "working directory does not exist: {} (running `{}`)",
            cwd.display(),
            argv.join(" ")
        );
    }
    println!("[run] {}", argv.join(" "));

    // Captured instead of inherited so the failure message can quote it. An
    // exit status alone ("`pnpm install` exited with exit status: 1") is the
    // one thing the user already knows; the reason lives in the output.
    //
    // Why not stream it: the output is replayed verbatim below, so nothing is
    // lost from the plugin log — only the live interleaving, which nobody is
    // watching because herdr redirects this process's stdout to a log file.
    let output = Command::new(program)
        .args(rest)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("spawning `{program}`"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    eprint!("{stderr}");

    if !output.status.success() {
        let command = argv.join(" ");
        match failure_tail(&stdout, &stderr) {
            Some(tail) => bail!("`{command}` exited with {}\n{tail}", output.status),
            None => bail!(
                "`{command}` exited with {} without printing anything",
                output.status
            ),
        }
    }
    Ok(())
}

/// The tail of a failed command's output, for the error message.
///
/// Why not stderr only: pnpm — the failure that prompted this — prints
/// `ERR_PNPM_ENOENT` and its explanation on *stdout*, so a quiet stderr must
/// fall through to stdout rather than report the command as silent.
fn failure_tail(stdout: &str, stderr: &str) -> Option<String> {
    [stderr, stdout].into_iter().find_map(last_lines)
}

/// The last [`FAILURE_TAIL_LINES`] non-blank lines of `text`, or `None` if it
/// has none. Blank lines are dropped so a trailing newline run cannot push the
/// real message out of the window.
fn last_lines(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let start = lines.len().saturating_sub(FAILURE_TAIL_LINES);
    Some(lines[start..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A directory containing each of `files` as an empty file.
    fn dir_with(files: &[&str]) -> TempDir {
        let dir = tempfile::tempdir().expect("creating tempdir");
        for name in files {
            std::fs::write(dir.path().join(name), "").expect("writing marker");
        }
        dir
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn rule(marker: &str, command: &[&str]) -> InstallRule {
        InstallRule {
            marker: marker.to_string(),
            command: argv(command),
        }
    }

    #[test]
    fn glob_matches_literal_names() {
        assert!(glob_match(".env", ".env"));
        assert!(!glob_match(".env", ".env.local"));
        assert!(!glob_match(".env", "env"));
        assert!(!glob_match(".env", ""));
    }

    #[test]
    fn glob_star_matches_any_run_including_empty() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", ".env.production.local"));
        assert!(glob_match("*.csproj", "App.csproj"));
        assert!(glob_match("a*c", "ac"));
        assert!(glob_match("a*c", "abbbc"));
        assert!(!glob_match("a*c", "ab"));
    }

    /// The reason [`DEFAULT_ENV_PATTERNS`] needs *both* `.env` and `.env.*`:
    /// the star can match empty, but the literal dot before it cannot, so
    /// `.env.*` alone would silently miss a plain `.env`.
    #[test]
    fn dotted_glob_does_not_match_the_bare_name() {
        assert!(glob_match(".env.*", ".env.local"));
        assert!(glob_match(".env.*", ".env."));
        assert!(!glob_match(".env.*", ".env"));

        let defaults: Vec<String> = DEFAULT_ENV_PATTERNS.iter().map(|s| s.to_string()).collect();
        for name in [".env", ".env.local", ".env.production.local"] {
            assert!(
                defaults.iter().any(|pat| glob_match(pat, name)),
                "{name} should match the default env patterns"
            );
        }
        assert!(
            !defaults.iter().any(|pat| glob_match(pat, "envrc")),
            "unrelated names should not match the defaults"
        );
    }

    #[test]
    fn marker_matches_exact_filenames() {
        let dir = dir_with(&["go.mod"]);
        assert!(marker_matches(dir.path(), "go.mod"));
        assert!(!marker_matches(dir.path(), "Cargo.toml"));
    }

    #[test]
    fn marker_matches_extension_wildcards() {
        let dir = dir_with(&["App.csproj"]);
        assert!(marker_matches(dir.path(), "*.csproj"));
        assert!(!marker_matches(dir.path(), "*.sln"));
    }

    #[test]
    fn marker_does_not_match_in_a_missing_directory() {
        let dir = tempfile::tempdir().expect("creating tempdir");
        let missing = dir.path().join("nope");
        assert!(!marker_matches(&missing, "go.mod"));
        assert!(!marker_matches(&missing, "*.csproj"));
    }

    #[test]
    fn detects_nothing_in_an_empty_directory() {
        let dir = dir_with(&[]);
        assert_eq!(detect_install(dir.path(), &[]), None);
    }

    /// Lockfiles precede the generic manifest in [`BUILTIN_RULES`], so a repo
    /// with both gets the reproducible install rather than a loose one.
    #[test]
    fn lockfiles_win_over_the_generic_manifest() {
        for (files, expected) in [
            (vec!["package.json"], argv(&["npm", "install"])),
            (
                vec!["package-lock.json", "package.json"],
                argv(&["npm", "ci"]),
            ),
            (vec!["bun.lockb", "package.json"], argv(&["bun", "install"])),
        ] {
            let dir = dir_with(&files);
            assert_eq!(
                detect_install(dir.path(), &[]),
                Some(expected),
                "for {files:?}"
            );
        }
    }

    /// Within the JS family the table order is the tiebreak, top to bottom.
    #[test]
    fn earlier_builtin_rules_win_ties() {
        let dir = dir_with(&["pnpm-lock.yaml", "yarn.lock", "package-lock.json"]);
        assert_eq!(
            detect_install(dir.path(), &[]),
            Some(argv(&["pnpm", "install", "--frozen-lockfile"]))
        );
    }

    #[test]
    fn custom_rules_override_builtins() {
        let dir = dir_with(&["package.json"]);
        let rules = vec![rule("package.json", &["npm", "install", "--offline"])];
        assert_eq!(
            detect_install(dir.path(), &rules),
            Some(argv(&["npm", "install", "--offline"]))
        );
    }

    #[test]
    fn custom_rules_can_add_an_unknown_language() {
        let dir = dir_with(&["flake.nix"]);
        let rules = vec![rule("flake.nix", &["nix", "develop", "--command", "true"])];
        assert_eq!(
            detect_install(dir.path(), &rules),
            Some(argv(&["nix", "develop", "--command", "true"]))
        );
        // Without the rule the same directory matches nothing.
        assert_eq!(detect_install(dir.path(), &[]), None);
    }

    #[test]
    fn custom_rules_are_checked_in_order() {
        let dir = dir_with(&["go.mod", "Cargo.toml"]);
        let rules = vec![
            rule("Cargo.toml", &["cargo", "fetch", "--offline"]),
            rule("go.mod", &["go", "mod", "download"]),
        ];
        assert_eq!(
            detect_install(dir.path(), &rules),
            Some(argv(&["cargo", "fetch", "--offline"]))
        );
    }

    #[test]
    fn install_dirs_are_validated_before_anything_runs() {
        let worktree = dir_with(&[]);
        std::fs::create_dir(worktree.path().join("apps")).expect("creating apps/");

        let dirs = vec!["apps".to_string(), "services".to_string()];
        let err = install_deps(worktree.path(), &[], Some(&dirs))
            .expect_err("a missing install dir should abort");
        assert!(
            err.to_string().contains("services"),
            "error should name the missing dir, got: {err}"
        );
    }

    #[test]
    fn empty_commands_are_rejected() {
        let dir = dir_with(&[]);
        let err = run_command(dir.path(), &[]).expect_err("an empty argv should be an error");
        assert!(err.to_string().contains("empty command"), "got: {err}");
    }

    /// The whole point of capturing: without it the error is just an exit
    /// status, and the user has to go digging through herdr's plugin log for
    /// the line that says what actually broke.
    #[test]
    fn a_failure_quotes_what_the_command_printed_to_stderr() {
        let dir = dir_with(&[]);
        let err = run_command(dir.path(), &argv(&["sh", "-c", "echo boom >&2; exit 3"]))
            .expect_err("a non-zero exit should abort");

        let msg = err.to_string();
        assert!(msg.contains("boom"), "got: {msg}");
        assert!(msg.contains("exit status: 3"), "got: {msg}");
    }

    /// pnpm prints `ERR_PNPM_ENOENT` on stdout with nothing on stderr, so a
    /// stderr-only tail would report the real-world failure as silent.
    #[test]
    fn a_failure_falls_back_to_stdout_when_stderr_is_empty() {
        let dir = dir_with(&[]);
        let err = run_command(
            dir.path(),
            &argv(&["sh", "-c", "echo ERR_PNPM_ENOENT; exit 1"]),
        )
        .expect_err("a non-zero exit should abort");

        assert!(err.to_string().contains("ERR_PNPM_ENOENT"), "got: {err}");
    }

    #[test]
    fn a_failure_with_no_output_at_all_says_so() {
        let dir = dir_with(&[]);
        let err = run_command(dir.path(), &argv(&["sh", "-c", "exit 1"]))
            .expect_err("a non-zero exit should abort");

        assert!(
            err.to_string().contains("without printing anything"),
            "got: {err}"
        );
    }

    /// A build that logs thousands of lines must not paste all of them into a
    /// notification; the tail is the part that carries the reason.
    #[test]
    fn a_long_failure_is_cut_down_to_its_last_lines() {
        let noisy = format!("seq 1 {}", FAILURE_TAIL_LINES * 3);
        let dir = dir_with(&[]);
        let err = run_command(
            dir.path(),
            &argv(&["sh", "-c", &format!("{noisy} >&2; exit 1")]),
        )
        .expect_err("a non-zero exit should abort");

        let msg = err.to_string();
        let last = (FAILURE_TAIL_LINES * 3).to_string();
        assert!(msg.contains(&last), "the last line should survive: {msg}");
        assert!(
            !msg.contains("\n1\n"),
            "the first line should be cut: {msg}"
        );
        // One line for the command itself, the rest for the output window.
        assert_eq!(msg.lines().count(), FAILURE_TAIL_LINES + 1);
    }

    #[test]
    fn running_in_a_missing_directory_is_an_error() {
        let dir = tempfile::tempdir().expect("creating tempdir");
        let missing = dir.path().join("nope");
        let err = run_command(&missing, &argv(&["true"]))
            .expect_err("a missing cwd should abort before spawning");
        assert!(
            err.to_string().contains("working directory does not exist"),
            "got: {err}"
        );
    }
}
