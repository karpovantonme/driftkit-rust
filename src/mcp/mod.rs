//! `driftkit mcp`: an MCP tool's input schema against what its handler reads.
//!
//! The schema declares a property the handler never looks at, or the handler
//! reads a key the schema never declares. The agent obeys the schema, so
//! either a capability is silently unreachable or the call fails outright.
//!
//! Measured on 265 servers in August 2026: 39 susceptible, and the findings
//! come in batches -- one KiCAD server had 15 tools that could not work at
//! all, because their schemas required `layerName` while the handlers read
//! `layer`.
//!
//! 🔴 Susceptibility is decided before scanning, not after filtering. Where a
//! single source feeds both the schema and the handler -- FastMCP, zod with
//! `z.infer`, `zodToJsonSchema` -- the mismatch is inexpressible, and the
//! server is skipped rather than scanned and filtered. That removed 60% of
//! the work.

pub mod classify;
pub mod go;
pub mod python;

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

use crate::core;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub hard: bool,
    pub kind: String,
    pub tool: String,
    pub names: Vec<String>,
    pub file: String,
    pub line: usize,
    pub note: String,
}

#[derive(Default)]
pub struct Report {
    pub verdict: String,
    pub why: String,
    pub tools_total: usize,
    pub tools_bound: usize,
    pub files: usize,
    pub findings: Vec<Finding>,
    pub tool_names: Vec<String>,
}

impl Report {
    pub fn hard(&self) -> usize {
        self.findings.iter().filter(|f| f.hard).count()
    }
    pub fn soft(&self) -> usize {
        self.findings.iter().filter(|f| !f.hard).count()
    }
}

/// Protocol-level keys a handler may read without the schema declaring them.
const PROTOCOL: &[&str] = &["method", "name", "arguments", "params", "_meta", "cursor"];

pub fn analyse(root: &Path, report: &mut Report) {
    let c = classify::classify(root);
    report.verdict = c.verdict.label().to_string();
    report.why = c.why.clone();

    // 🔴 A protected server is skipped, not scanned and then filtered. That
    // is what removed 60% of the work: the mismatch is inexpressible there,
    // so anything found would be noise by construction.
    if c.verdict == classify::Verdict::Protected
        || c.verdict == classify::Verdict::NotAServer
        || c.verdict == classify::Verdict::NoSources
    {
        return;
    }

    let scan = go::scan(root);
    let py = scan_python(root);
    report.files = scan.files + py.files;
    report.tools_total = scan.tools.len() + py.schemas.len();

    for t in &scan.tools {
        report.tools_bound += 1;
        report.tool_names.push(t.name.clone());
        let declared_unread: Vec<String> = t
            .props
            .difference(&t.reads)
            .filter(|n| !PROTOCOL.contains(&n.as_str()))
            .cloned()
            .collect();
        let read_undeclared: Vec<String> = t
            .reads
            .difference(&t.props)
            .filter(|n| !PROTOCOL.contains(&n.as_str()))
            .cloned()
            .collect();

        if !read_undeclared.is_empty() {
            let required_gap: BTreeSet<&String> =
                t.required.iter().filter(|n| !t.reads.contains(*n)).collect();
            let note = if required_gap.is_empty() {
                "the handler reads a key the schema never declares, so an agent cannot know to send it".to_string()
            } else {
                format!(
                    "the schema requires {} while the handler reads {}",
                    required_gap
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    read_undeclared.join(", ")
                )
            };
            report.findings.push(Finding {
                hard: true,
                kind: "read-undeclared".into(),
                tool: t.name.clone(),
                names: read_undeclared,
                file: t.file.clone(),
                line: t.line,
                note,
            });
        }

        // 🔴 An opaque handler passes `args` on somewhere this scanner cannot
        // follow, so "declared but never read" is not a claim it may make.
        if !declared_unread.is_empty() && !t.opaque {
            report.findings.push(Finding {
                hard: true,
                kind: "declared-unread".into(),
                tool: t.name.clone(),
                names: declared_unread,
                file: t.file.clone(),
                line: t.line,
                note: "the schema advertises a property the handler never looks at".into(),
            });
        }
    }
    python_findings(&py, report);
}

/// One walk over the Python files, then schemas matched to handlers.
fn scan_python(root: &Path) -> python::PyScan {
    let mut scan = python::PyScan::default();
    for path in core::walk(root) {
        if path.extension().map(|e| e != "py").unwrap_or(true) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let base = rel.rsplit('/').next().unwrap_or(&rel);
        if rel.contains("/test") || rel.contains("/examples/") || base.starts_with("test_")
            || base.ends_with("_test.py")
        {
            continue;
        }
        if let Some(src) = core::read_text(&path) {
            python::scan_file(&rel, &src, &mut scan);
        }
    }
    scan
}

fn python_findings(scan: &python::PyScan, report: &mut Report) {
    for (name, schema) in &scan.schemas {
        // 🔴 A name may appear BOTH in an `if name ==` branch AND in a dispatch
        // table. Taking the first source is wrong: in KiCAD the branch was an
        // empty stub while the real reading lived in the dispatcher's method,
        // and the tool looked like it read nothing at all.
        let mut full = python::Reads::default();
        let mut own = python::Reads::default();
        let mut bound = false;
        if let Some(r) = scan.branches.get(name) {
            full.merge(r);
            bound = true;
        }
        if let Some(r) = scan.branches_own.get(name) {
            own.merge(r);
        }
        if let Some(method) = scan.dispatch.get(name) {
            if let Some(r) = scan.funcs.get(method) {
                full.merge(r);
                own.merge(r);
                bound = true;
            }
        }
        if !bound {
            continue;
        }
        report.tools_bound += 1;
        report.tool_names.push(name.clone());

        let declared_unread: Vec<String> = schema.props.difference(&full.reads).cloned().collect();
        // 🔴 The dispatcher preamble is shared by ALL its branches, so a branch
        // that does not use it was getting a false "read but not declared".
        // The preamble silences "declared but unread" and takes no part in the
        // extras.
        let read_undeclared: Vec<String> = own
            .reads
            .difference(&schema.props)
            .filter(|n| !full.synonyms.contains(*n) && !own.synonyms.contains(*n))
            .filter(|n| !PROTOCOL.contains(&n.as_str()))
            .filter(|n| !scan.self_assigned.contains(*n))
            .cloned()
            .collect();

        if !read_undeclared.is_empty() {
            report.findings.push(Finding {
                hard: true,
                kind: "read-undeclared".into(),
                tool: name.clone(),
                names: read_undeclared,
                file: schema.file.clone(),
                line: schema.line,
                note: "the handler reads a key the schema never declares".into(),
            });
        }
        if !declared_unread.is_empty() && !full.opaque {
            report.findings.push(Finding {
                hard: true,
                kind: "declared-unread".into(),
                tool: name.clone(),
                names: declared_unread,
                file: schema.file.clone(),
                line: schema.line,
                note: "the schema advertises a property the handler never looks at".into(),
            });
        }
    }
}

pub fn print_report(report: &Report, verbose: bool) {
    for (kind, title) in [
        ("read-undeclared", "Read by the handler, absent from the schema"),
        ("declared-unread", "Advertised by the schema, never read"),
    ] {
        let rows: Vec<&Finding> = report.findings.iter().filter(|f| f.kind == kind).collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n--- {title} ({}) ---", rows.len());
        for f in rows {
            println!("  {}  {}:{}", f.tool, f.file, f.line);
            println!("     {}", f.names.join(", "));
            if verbose {
                println!("     {}", f.note);
            }
        }
    }

    println!("\n=== Coverage ===");
    println!("  susceptibility:         {} ({})", report.verdict, report.why);
    println!("  go files read:          {}", report.files);
    println!(
        "  tools bound to a handler: {} of {}",
        report.tools_bound, report.tools_total
    );
    println!("{}", core::findings_line(report.hard(), report.soft()));

    if verbose && !report.tool_names.is_empty() {
        println!("\n--- Tools bound ({}) ---", report.tool_names.len());
        for chunk in report.tool_names.chunks(4) {
            println!("  {}", chunk.join("  "));
        }
    }
}
