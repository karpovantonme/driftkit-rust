//! The contract every check in this kit obeys.
//!
//! Ported from the Python kit, where it was written after eight tools had
//! quietly drifted apart: one called its confidence flag `confident` in JSON
//! while the rest called it `hard`, and the runner read `hard` defaulting to
//! true, counting soft findings as hard. The kit is one thing, not a pile of
//! separate binaries.
//!
//! 1. `--json FILE` and `-v/--verbose` exist on every subcommand.
//! 2. `--json` writes a list of objects, each carrying a boolean `hard`.
//!    A hard finding is one the tool stands behind; a soft one is where a
//!    human decides.
//! 3. The report ends with a `=== Coverage ===` block whose last line is
//!    `findings: N hard, M soft`.
//! 4. Exit code is 1 if and only if there is at least one hard finding, so
//!    the tool works inside a shell `if`.

use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Directories no check in this kit reads.
///
/// The single place this list is declared: in the Python kit three separate
/// copies had already drifted apart before anybody noticed.
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "vendor",
    "node_modules",
    "testdata",
    "third_party",
    "dist",
    "build",
    "_build",
    "target",
    "__pycache__",
    ".venv",
    "venv",
];

/// `.github` starts with a dot but is needed: CI matrices live there. A
/// blanket hidden-directory mask once dropped it and a survey quietly
/// declared a project unusable while it had nineteen workflows.
pub const KEEP_HIDDEN: &[&str] = &[".github"];

/// A file over this size is not read. Counted, never silently skipped.
pub const SIZE_CEILING: u64 = 4_000_000;

fn is_skipped(name: &str) -> bool {
    if SKIP_DIRS.contains(&name) {
        return true;
    }
    name.starts_with('.') && !KEEP_HIDDEN.contains(&name)
}

/// Walk a tree using the shared skip list, yielding files only.
pub fn walk(root: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            !is_skipped(&e.file_name().to_string_lossy())
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
}

/// Read a file, tolerating broken encodings. Returns None for anything over
/// the ceiling or unreadable, so the caller can count it.
pub fn read_text(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > SIZE_CEILING || meta.len() == 0 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The last line of every report.
pub fn findings_line(hard: usize, soft: usize) -> String {
    format!("  findings:               {hard} hard, {soft} soft")
}

/// Where a check belongs. A local check reads text and is safe anywhere; a
/// network check talks to other people's servers; a build check executes
/// foreign code and belongs on a disposable runner rather than a laptop.
///
/// Declared before the second check exists, on purpose: in the Python kit an
/// undeclared tool defaulted to local, which is how a laptop ends up
/// compiling somebody else's test suite.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Local,
    Network,
    Build,
}

#[allow(dead_code)]
pub fn place_of(check: &str) -> Place {
    match check {
        "env" | "mcp" => Place::Local,
        _ => Place::Local,
    }
}
