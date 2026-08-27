//! `driftkit env`: the example env file against what the code actually reads.
//!
//! Clone a project, copy `.env.example` to `.env`, run it, and it dies on a
//! variable the example never mentioned. That is the species. It is the first
//! hour of every newcomer and nobody checks it: `dotenv-linter` (2098 stars)
//! lints the files themselves, `sync-dotenv` (477) compares one file with
//! another file. Nothing compares the file with the code.
//!
//! THREE CLASSES, AND THEY ARE NOT EQUAL EVIDENCE.
//!
//! - **B**: read without a default, absent from every example file. A
//!   positive claim proved locally: `os.environ["SMTP_HOST"]` raises, so a
//!   fresh clone dies there. Hard.
//! - **A-near**: declared in the example, unread, but a name one typo away IS
//!   read. Name against name. Hard.
//! - **A-plain**: declared, unread, nothing similar. A negative claim from a
//!   source that cannot be complete, so soft, and hidden unless `--plain`.
//!
//! SUSCEPTIBILITY BELONGS TO SIDE A, AND ONLY TO IT. Where the name is
//! derived rather than written, "nothing reads this" cannot be said. Side B
//! is a different kind of claim: `os.environ["X"]` on line 40 raises whatever
//! else the project does elsewhere.
//!
//! 🔴 The first version softened both, and on the first eight projects seven
//! came back PROTECTED, usually off one `os.environ[name]` in `conftest.py`.
//! One helper file was silencing an entire repository.

mod python;
mod readers;
mod similar;

use crate::core;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

/// Names a fresh clone gets from the machine or from a launcher, not from an
/// example file. Nobody puts PATH in `.env.example`, and requiring it there
/// would be telling a maintainer off for something they decided on purpose.
const AMBIENT: &[&str] = &[
    "PATH", "HOME", "USER", "USERNAME", "LOGNAME", "PWD", "OLDPWD", "SHELL",
    "TERM", "TMPDIR", "TEMP", "TMP", "LANG", "LANGUAGE", "TZ", "HOSTNAME",
    "PYTHONPATH", "PYTHONHOME", "VIRTUAL_ENV", "CONDA_PREFIX", "CONDA_DEFAULT_ENV",
    "GOPATH", "GOROOT", "JAVA_HOME", "NODE_ENV", "NODE_OPTIONS", "NVM_DIR",
    "CI", "GITHUB_ACTIONS", "GITHUB_TOKEN", "GITHUB_REPOSITORY", "GITHUB_SHA",
    "GITHUB_REF", "GITHUB_WORKSPACE", "GITHUB_EVENT_NAME", "GITHUB_OUTPUT",
    "RUNNER_OS", "RUNNER_TEMP", "READTHEDOCS", "COLUMNS", "LINES", "EDITOR",
    "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
    "APPDATA", "LOCALAPPDATA", "PROGRAMFILES", "SYSTEMROOT", "WINDIR", "COMSPEC",
    "PATHEXT", "SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "HTTP_PROXY", "HTTPS_PROXY",
    "NO_PROXY", "ALL_PROXY", "DISPLAY", "SHLVL", "TERM_PROGRAM",
    // 🔴 Set by a launcher rather than by a person: torchrun exports
    // LOCAL_RANK, the ML libraries export their own cache paths. Live false
    // findings on EmpaAva_System were LOCAL_RANK and MODELSCOPE_CACHE.
    "LOCAL_RANK", "RANK", "WORLD_SIZE", "LOCAL_WORLD_SIZE", "MASTER_ADDR",
    "MASTER_PORT", "GROUP_RANK", "ROLE_RANK", "TORCHELASTIC_RUN_ID",
    "CUDA_VISIBLE_DEVICES", "NVIDIA_VISIBLE_DEVICES", "OMP_NUM_THREADS",
    "MKL_NUM_THREADS", "TOKENIZERS_PARALLELISM", "HF_HOME", "HF_HUB_CACHE",
    "TRANSFORMERS_CACHE", "MODELSCOPE_CACHE", "TORCH_HOME",
    "PYTORCH_CUDA_ALLOC_CONF", "ACCELERATE_MIXED_PRECISION",
];

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub hard: bool,
    pub kind: String,
    pub name: String,
    pub file: String,
    pub line: usize,
    pub note: String,
}

#[derive(Default)]
pub struct Report {
    pub example_files: Vec<String>,
    pub declared: BTreeMap<String, String>,
    pub commented: HashSet<String>,
    pub known: HashSet<String>,
    pub mentioned: HashSet<String>,
    pub assigned: HashSet<String>,
    pub py_reads: Vec<python::PyRead>,
    pub other_reads: BTreeMap<String, String>,
    pub protections: Vec<String>,
    pub unparsed: Vec<String>,
    pub files_skipped: usize,
    pub py_files: usize,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn protected(&self) -> bool {
        !self.protections.is_empty()
    }
    pub fn hard(&self) -> usize {
        self.findings.iter().filter(|f| f.hard).count()
    }
    pub fn soft(&self) -> usize {
        self.findings.iter().filter(|f| !f.hard).count()
    }
}

/// Does a reader here speak for the running application.
fn is_app_file(rel: &str) -> bool {
    static NOT_APP: OnceLock<Regex> = OnceLock::new();
    static NOT_APP_FILE: OnceLock<Regex> = OnceLock::new();
    let not_app = NOT_APP.get_or_init(|| {
        Regex::new(
            r"(?i)(^|/)(tests?|testing|spec|specs|examples?|samples?|benchmarks?|docs?|scripts?|tools?|ci|\.github|migrations?|fixtures?)(/|$)",
        )
        .unwrap()
    });
    // 🔴 Live false finding: `e2e_suite.py` at the root of Agent-MemoryForge
    // reads ADMIN_PW. Its own docstring calls it an end-to-end test suite.
    let not_app_file = NOT_APP_FILE.get_or_init(|| {
        Regex::new(r"(?i)^(test_|conftest)|(_test|\.test|\.spec)\.\w+$|(^|_)(e2e|smoke|bench|stress|load)(_|\.)|_suite\.\w+$").unwrap()
    });
    let base = rel.rsplit('/').next().unwrap_or(rel);
    !not_app.is_match(rel) && !not_app_file.is_match(base)
}

pub fn analyse(root: &Path, report: &mut Report, plain: bool) {
    collect(root, report);

    let read_names: HashSet<String> = report
        .py_reads
        .iter()
        .map(|r| r.name.clone())
        .chain(report.other_reads.keys().cloned())
        .collect();

    // --- B: required in the code, absent from every example file
    if !report.example_files.is_empty() {
        let mut seen: HashSet<String> = HashSet::new();
        let declared_names: Vec<String> = report.declared.keys().cloned().collect();
        for r in &report.py_reads {
            if !r.required || seen.contains(&r.name) {
                continue;
            }
            if report.declared.contains_key(&r.name)
                || report.commented.contains(&r.name)
                || report.known.contains(&r.name)
                || report.assigned.contains(&r.name)
                || AMBIENT.contains(&r.name.as_str())
                || !is_app_file(&r.file)
            {
                continue;
            }
            seen.insert(r.name.clone());
            let note = match similar::similar(&r.name, declared_names.iter()) {
                Some(near) => format!("the example file declares {near}, one edit away"),
                None => "a fresh clone raises KeyError here".to_string(),
            };
            report.findings.push(Finding {
                hard: true,
                kind: "missing-from-example".into(),
                name: r.name.clone(),
                file: r.file.clone(),
                line: r.line,
                note,
            });
        }
    }

    // --- A: declared but unread. The negative claim, and it is weaker.
    let protected = report.protected();
    let declared: Vec<(String, String)> = report
        .declared
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let read_pool: Vec<String> = read_names.iter().cloned().collect();
    for (name, where_) in declared {
        if read_names.contains(&name) || report.mentioned.contains(&name) {
            continue;
        }
        match similar::similar(&name, read_pool.iter()) {
            Some(near) => report.findings.push(Finding {
                hard: !protected,
                kind: "near-miss".into(),
                name,
                file: where_,
                line: 0,
                note: format!("the code reads {near}"),
            }),
            None if plain => report.findings.push(Finding {
                hard: false,
                kind: "unread".into(),
                name,
                file: where_,
                line: 0,
                note: "no reader found in any language this tool parses".into(),
            }),
            None => {}
        }
    }
}

fn collect(root: &Path, report: &mut Report) {
    for path in core::walk(root) {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if readers::is_example_file(&name) {
            report.example_files.push(rel.clone());
            if let Some(src) = core::read_text(&path) {
                for line in src.lines() {
                    if let Some(n) = readers::declaration(line) {
                        report.declared.entry(n).or_insert_with(|| rel.clone());
                    } else if let Some(n) = readers::commented_declaration(line) {
                        report.commented.insert(n);
                    }
                }
            }
            continue;
        }

        if readers::is_committed_env(&name) {
            if let Some(src) = core::read_text(&path) {
                for line in src.lines() {
                    if let Some(n) = readers::declaration(line) {
                        report.known.insert(n);
                    }
                }
            }
            continue;
        }

        let Some(src) = core::read_text(&path) else {
            // empty, unreadable or over the ceiling: counted, because a gap
            // that stays silent costs more than a finding that shouts
            report.files_skipped += 1;
            continue;
        };

        report.mentioned.extend(readers::shouty_tokens(&src));

        // 🔴 A dynamic read inside conftest.py says nothing about how the
        // application reads its configuration.
        if is_app_file(&rel) && readers::dynamic_read(&ext, &src) {
            let note = "a read whose key is a variable, not a literal";
            if !report.protections.iter().any(|p| p.starts_with(note)) {
                report.protections.push(format!("{note} ({rel})"));
            }
        }

        if let Some(why) = readers::protection(&ext, &src) {
            if !report.protections.iter().any(|p| p.starts_with(why)) {
                report.protections.push(format!("{why} ({rel})"));
            }
        }

        if ext == "py" || ext == "pyi" {
            report.py_files += 1;
            match python::read_file(&rel, &src) {
                Some(out) => {
                    report.py_reads.extend(out.reads);
                    report.assigned.extend(out.assigned);
                }
                None => report.unparsed.push(rel.clone()),
            }
            continue;
        }

        for n in readers::other_reads(&ext, &name, &src) {
            report.other_reads.entry(n).or_insert_with(|| rel.clone());
        }
    }
}

pub fn print_report(report: &Report, verbose: bool) {
    for (kind, title) in [
        ("missing-from-example", "Required by the code, missing from the example"),
        ("near-miss", "Declared, unread, and a near name is read"),
        ("unread", "Declared, no reader found (soft by construction)"),
    ] {
        let rows: Vec<&Finding> = report.findings.iter().filter(|f| f.kind == kind).collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n--- {title} ({}) ---", rows.len());
        for f in rows {
            let where_ = if f.line > 0 {
                format!("{}:{}", f.file, f.line)
            } else {
                f.file.clone()
            };
            println!("  {}  {}", f.name, where_);
            println!("     {}", f.note);
        }
    }

    let matched = report
        .declared
        .keys()
        .filter(|k| {
            report.other_reads.contains_key(*k) || report.py_reads.iter().any(|r| &&r.name == k)
        })
        .count();
    let mentioned = report
        .declared
        .keys()
        .filter(|k| report.mentioned.contains(*k))
        .count();

    println!("\n=== Coverage ===");
    println!(
        "  example files:          {} ({})",
        report.example_files.len(),
        report.example_files.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
    );
    println!(
        "  variables declared:     {} (+{} commented out)",
        report.declared.len(),
        report.commented.len()
    );
    println!("  python files read:      {}", report.py_files);
    println!(
        "  python reads:           {} ({} required)",
        report.py_reads.len(),
        report.py_reads.iter().filter(|r| r.required).count()
    );
    println!(
        "  reads in other langs:   {} (collected, never judged)",
        report.other_reads.len()
    );
    println!("  declared and read:      {matched} of {}", report.declared.len());
    println!("  declared and mentioned: {mentioned} of {}", report.declared.len());
    println!(
        "  susceptibility:         {}",
        if report.protected() { "PROTECTED" } else { "susceptible" }
    );
    for p in report.protections.iter().take(4) {
        println!("      {p}");
    }
    println!("  not parsed:             {} failed to parse", report.unparsed.len());
    println!(
        "  files skipped:          {} (empty or over the size ceiling)",
        report.files_skipped
    );
    println!("{}", core::findings_line(report.hard(), report.soft()));

    if verbose && !report.unparsed.is_empty() {
        println!("\n--- Not parsed ({}) ---", report.unparsed.len());
        for p in report.unparsed.iter().take(30) {
            println!("  {p}");
        }
    }
}
