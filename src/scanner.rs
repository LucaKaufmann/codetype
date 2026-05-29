//! Enumerates source files in a Git-managed repository.
//!
//! Shells out to `git ls-files` to determine which files belong to the
//! codebase, then filters by the active Extractor's claimed extensions and
//! a line-count cap. The only module in CodeType that runs external
//! processes — that containment is deliberate: relying on `git ls-files`
//! keeps untracked, ignored, and generated files out of the exercise pool.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};

/// Default line-count cap. Files exceeding this are dropped before the
/// Extractor ever sees them — they're almost certainly generated or
/// pathological.
const DEFAULT_MAX_LINES: usize = 2000;

pub struct RepoScanner {
    root: PathBuf,
    extensions: Vec<&'static str>,
    max_lines: usize,
}

impl RepoScanner {
    pub fn new(root: impl Into<PathBuf>, extensions: Vec<&'static str>) -> Self {
        Self {
            root: root.into(),
            extensions,
            max_lines: DEFAULT_MAX_LINES,
        }
    }

    /// Builder-style override of the line cap. Useful for tests, and for any
    /// future "scan everything regardless of size" mode.
    pub fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = max_lines;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Enumerate candidate source files. Returns absolute paths, all of
    /// which exist, all of which match an Extractor extension, and none of
    /// which exceed [`Self::max_lines`].
    pub fn scan(&self) -> anyhow::Result<Vec<PathBuf>> {
        let tracked = self.list_tracked_files()?;
        let kept = tracked
            .into_iter()
            .filter(|p| self.extension_matches(p))
            .filter(|p| under_line_limit(p, self.max_lines))
            .collect();
        Ok(kept)
    }

    fn list_tracked_files(&self) -> anyhow::Result<Vec<PathBuf>> {
        list_tracked_files(&self.root)
    }

    fn extension_matches(&self, path: &Path) -> bool {
        // `Path::extension` returns the bit after the last `.`, without the
        // dot, as an `&OsStr`. We then try to coerce to `&str`; if the path
        // is somehow not valid UTF-8 (extremely rare for source files), we
        // skip it.
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        self.extensions.contains(&ext)
    }
}

/// List every file tracked by Git under `root`, as absolute paths. Used both
/// by [`RepoScanner`] (which then filters by extension) and by language
/// auto-detection (which needs the full list to count files per language).
pub fn list_tracked_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("ls-files")
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to invoke `git` in {}", root.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "{} is not a Git repository (or `git ls-files` failed): {}",
            root.display(),
            stderr.trim()
        ));
    }

    // `git ls-files` outputs paths relative to the repo root, one per line.
    // We join with `root` so downstream consumers (extractors, renderers) get
    // absolute paths.
    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| root.join(line))
        .collect();
    Ok(paths)
}

/// Cheap predicate: read the file as bytes, count `'\n'`, compare to cap.
/// Returns `false` on any IO error — a file we cannot read is a file we
/// cannot drill, so it deserves to be dropped silently.
fn under_line_limit(path: &Path, max_lines: usize) -> bool {
    match count_lines(path) {
        Ok(n) => n <= max_lines,
        Err(_) => false,
    }
}

fn count_lines(path: &Path) -> std::io::Result<usize> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    let mut count = bytes.iter().filter(|&&b| b == b'\n').count();
    // A trailing-newline-free file still has one line of content past the
    // last `'\n'`.
    if !bytes.ends_with(b"\n") {
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    // -- Test fixtures -------------------------------------------------------
    //
    // Each test sets up a real ephemeral git repository on disk. We use
    // `tempfile::TempDir` so the directory is automatically removed when the
    // test ends (even on panic). Running real `git` commands is slow compared
    // to pure-logic tests, but it's the only honest way to test a module
    // whose contract is "this is what `git ls-files` returns."

    /// Create a tempdir, `git init` it, write the given files, and `git add`
    /// each one so they show up in `git ls-files`. Returns the TempDir which
    /// the caller must keep alive for the test's duration.
    fn make_repo(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().expect("create tempdir");
        run_git(dir.path(), &["init", "--quiet"]);
        for (rel_path, content) in files {
            let full = dir.path().join(rel_path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("create parent dirs");
            }
            fs::write(&full, content).expect("write file");
            run_git(dir.path(), &["add", rel_path]);
        }
        dir
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("invoke git");
        assert!(
            status.success(),
            "git {:?} failed in {}",
            args,
            dir.display()
        );
    }

    /// Collapse absolute paths back to repo-relative for cleaner assertions.
    fn relative_to(root: &Path, paths: &[PathBuf]) -> Vec<String> {
        let mut rel: Vec<String> = paths
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
            .collect();
        rel.sort();
        rel
    }

    // -- Tests ---------------------------------------------------------------

    #[test]
    fn non_git_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let scanner = RepoScanner::new(dir.path(), vec!["swift"]);
        let err = scanner.scan().unwrap_err();
        // Don't assert on the exact stderr — `git`'s wording varies across
        // versions. Just confirm we hit the "not a Git repository" branch.
        let msg = format!("{}", err);
        assert!(
            msg.contains("not a Git repository") || msg.contains("ls-files"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn empty_git_repo_returns_empty() {
        let repo = make_repo(&[]);
        let scanner = RepoScanner::new(repo.path(), vec!["swift"]);
        assert!(scanner.scan().unwrap().is_empty());
    }

    #[test]
    fn finds_swift_files_only() {
        let repo = make_repo(&[
            ("a.swift", "func a() {}"),
            ("b.swift", "func b() {}"),
            ("c.py", "def c(): pass"),
            ("README.md", "# Project"),
        ]);
        let scanner = RepoScanner::new(repo.path(), vec!["swift"]);
        let found = scanner.scan().unwrap();
        assert_eq!(relative_to(repo.path(), &found), vec!["a.swift", "b.swift"]);
    }

    #[test]
    fn untracked_files_are_not_returned() {
        let repo = make_repo(&[("tracked.swift", "func t() {}")]);
        // Write an extra file but do NOT `git add` it.
        fs::write(repo.path().join("untracked.swift"), "func u() {}").unwrap();

        let scanner = RepoScanner::new(repo.path(), vec!["swift"]);
        let found = scanner.scan().unwrap();
        assert_eq!(relative_to(repo.path(), &found), vec!["tracked.swift"]);
    }

    #[test]
    fn files_over_line_limit_are_dropped() {
        // Use a tiny limit so we don't have to write a 2000-line fixture.
        let small = "func a() {}\n";
        let big = "x\n".repeat(50); // 50 lines
        let repo = make_repo(&[("small.swift", small), ("big.swift", &big)]);
        let scanner = RepoScanner::new(repo.path(), vec!["swift"]).with_max_lines(10);
        let found = scanner.scan().unwrap();
        assert_eq!(relative_to(repo.path(), &found), vec!["small.swift"]);
    }

    #[test]
    fn paths_returned_are_absolute() {
        let repo = make_repo(&[("a.swift", "func a() {}")]);
        let scanner = RepoScanner::new(repo.path(), vec!["swift"]);
        let found = scanner.scan().unwrap();
        assert_eq!(found.len(), 1);
        assert!(
            found[0].is_absolute(),
            "expected absolute path, got {:?}",
            found[0]
        );
    }

    #[test]
    fn files_in_subdirectories_are_found() {
        let repo = make_repo(&[
            ("Sources/Order.swift", "struct Order {}"),
            ("Sources/Sub/Item.swift", "struct Item {}"),
            ("Tests/OrderTests.swift", "struct OrderTests {}"),
        ]);
        let scanner = RepoScanner::new(repo.path(), vec!["swift"]);
        assert_eq!(
            relative_to(repo.path(), &scanner.scan().unwrap()),
            vec![
                "Sources/Order.swift",
                "Sources/Sub/Item.swift",
                "Tests/OrderTests.swift",
            ]
        );
    }

    #[test]
    fn multiple_extensions_supported() {
        let repo = make_repo(&[
            ("a.swift", "func a() {}"),
            ("b.py", "def b(): pass"),
            ("c.rs", "fn c() {}"),
        ]);
        let scanner = RepoScanner::new(repo.path(), vec!["swift", "py"]);
        assert_eq!(
            relative_to(repo.path(), &scanner.scan().unwrap()),
            vec!["a.swift", "b.py"]
        );
    }

    // -- count_lines: pure helper --

    #[test]
    fn count_lines_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.txt");
        fs::write(&p, "").unwrap();
        assert_eq!(count_lines(&p).unwrap(), 0);
    }

    #[test]
    fn count_lines_with_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, "one\ntwo\n").unwrap();
        assert_eq!(count_lines(&p).unwrap(), 2);
    }

    #[test]
    fn count_lines_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, "one\ntwo").unwrap();
        assert_eq!(count_lines(&p).unwrap(), 2);
    }
}
