//! Is this MCP server capable of holding the species at all.
//!
//! One question: **is the input schema and the handler fed by one source?**
//! If they are, a name mismatch is inexpressible, and the server need not be
//! read at all. Measured on 20 servers: 1 of 20 was susceptible. Without this
//! filter the scanner spends 19 runs out of 20 on nothing.
//!
//! 🔴 The asymmetry is deliberate. Erring towards "susceptible" costs one
//! wasted parse; missing a susceptible server costs a finding. So the verdict
//! "protected" requires a protective signal AND the absence of a risk signal,
//! never just the first.

use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

use crate::core;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Property names are string literals on both sides. Worth reading.
    Susceptible,
    /// Schema and handler come from one source. A mismatch cannot exist.
    Protected,
    /// Neither signal. A human decides.
    Unclear,
    NotAServer,
    NoSources,
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Susceptible => "SUSCEPTIBLE",
            Verdict::Protected => "protected",
            Verdict::Unclear => "unclear",
            Verdict::NotAServer => "not an MCP server",
            Verdict::NoSources => "no sources",
        }
    }
}

#[derive(Debug)]
pub struct Classification {
    pub verdict: Verdict,
    pub why: String,
    pub files: usize,
    pub protect: Vec<String>,
    pub risk: Vec<String>,
}

const EXT: &[&str] = &[
    "ts", "tsx", "js", "mjs", "py", "go", "java", "kt", "rb", "cs", "rs", "swift",
];

fn skip_path(rel: &str) -> bool {
    static RX: OnceLock<Regex> = OnceLock::new();
    static RX_FILE: OnceLock<Regex> = OnceLock::new();
    let dirs = RX.get_or_init(|| {
        Regex::new(r"(^|/)(node_modules|dist|build|out|vendor|testdata|__pycache__|tests?|__tests__|spec|fixtures|examples?|docs?)(/|$)").unwrap()
    });
    let files = RX_FILE.get_or_init(|| {
        Regex::new(r"(_test\.go|\.test\.[tj]sx?|\.spec\.[tj]sx?|\.d\.ts|(^|/)test_[^/]*\.py|_test\.py)$").unwrap()
    });
    dirs.is_match(rel) || files.is_match(rel)
}

/// Schema and handler fed by one source: a mismatch is inexpressible.
fn protective_signals() -> &'static [(&'static str, Regex)] {
    static SIGNALS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    SIGNALS.get_or_init(|| {
        vec![
            (
                "FastMCP: the schema is derived from the function signature",
                Regex::new(r"@\w+\.tool\s*[(\n]|@tool\b|FastMCP\s*\(|from\s+fastmcp|from\s+mcp\.server\.fastmcp").unwrap(),
            ),
            (
                "zodToJsonSchema from the same type the handler parses",
                Regex::new(r"zodToJsonSchema\s*\(").unwrap(),
            ),
            (
                "z.infer: the handler's type is derived from the schema",
                Regex::new(r"z\.infer\s*<").unwrap(),
            ),
            (
                "schema.parse/safeParse inside the handler",
                Regex::new(r"\.\s*(?:safeParse|parse)\s*\(\s*(?:args|params|arguments|request\.params)").unwrap(),
            ),
            (
                "registerTool with a zod object schema",
                Regex::new(r"registerTool\s*\(").unwrap(),
            ),
            (
                "the Python SDK tool decorator",
                Regex::new(r"@(?:mcp|server|app)\.(?:tool|call_tool)\b").unwrap(),
            ),
        ]
    })
}

/// The property name is a string literal on both sides, and nothing ties the
/// two literals together.
fn risk_signals() -> &'static [(&'static str, Regex, Regex)] {
    static SIGNALS: OnceLock<Vec<(&'static str, Regex, Regex)>> = OnceLock::new();
    SIGNALS.get_or_init(|| {
        vec![
            (
                "Go: schema properties as literals, args read by literal",
                Regex::new(r"Properties\s*:\s*map\[string\]").unwrap(),
                Regex::new(r#"args\s*\[\s*"|\(\s*args\s*,\s*""#).unwrap(),
            ),
            (
                "Go: mcp.WithString and friends, read back by literal",
                Regex::new(r#"mcp\.With(?:String|Number|Boolean|Array|Object)\s*\(\s*""#).unwrap(),
                Regex::new(r#"(?:Request|request|req)\.(?:GetString|GetInt|GetBool|RequireString)\s*\(\s*"|args\s*\[\s*""#).unwrap(),
            ),
            (
                "a literal JSON Schema, and args.X read without zod",
                Regex::new(r#"(?s)inputSchema\s*:\s*\{[^}]*?(?:type\s*:\s*['"]object|properties\s*:)"#).unwrap(),
                Regex::new(r#"\bargs\.\w+|arguments\s*\[\s*['"]|\bargs\s*\[\s*['"]"#).unwrap(),
            ),
            (
                "Python: a hand-written inputSchema, arguments read by key",
                Regex::new(r#"inputSchema\s*=\s*\{|"inputSchema"\s*:\s*\{|types\.Tool\s*\("#).unwrap(),
                Regex::new(r#"arguments\s*\[\s*['"]|arguments\.get\s*\(|args\s*\[\s*['"]"#).unwrap(),
            ),
            (
                "the handler takes `any` and reads by name",
                Regex::new(r"\(\s*args\s*:\s*any").unwrap(),
                Regex::new(r"\bargs\.\w+").unwrap(),
            ),
        ]
    })
}

pub fn classify(root: &Path) -> Classification {
    let mut sources: Vec<(String, String)> = Vec::new();
    for path in core::walk(root) {
        if sources.len() >= 4000 {
            break;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !EXT.contains(&ext.as_str()) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if skip_path(&rel) {
            continue;
        }
        if let Some(src) = core::read_text(&path) {
            sources.push((rel, src));
        }
    }

    if sources.is_empty() {
        return Classification {
            verdict: Verdict::NoSources,
            why: "no files in any language this tool reads".into(),
            files: 0,
            protect: vec![],
            risk: vec![],
        };
    }

    let looks_like_server = sources
        .iter()
        .any(|(_, s)| s.contains("inputSchema") || s.to_lowercase().contains("tool"));
    if !looks_like_server {
        return Classification {
            verdict: Verdict::NotAServer,
            why: "neither inputSchema nor a tool registration".into(),
            files: sources.len(),
            protect: vec![],
            risk: vec![],
        };
    }

    let protect: Vec<String> = protective_signals()
        .iter()
        .filter(|(_, rx)| sources.iter().any(|(_, s)| rx.is_match(s)))
        .map(|(name, _)| (*name).to_string())
        .collect();

    // 🔴 Both halves of a risk signal must meet IN ONE FILE. Gluing the whole
    // repository into one string married an `inputSchema` from one file to an
    // `args.foo` from another and gave 4 false susceptibles out of 5 on the
    // calibration set.
    let mut risk = Vec::new();
    for (name, rx_schema, rx_read) in risk_signals() {
        if let Some((rel, _)) = sources
            .iter()
            .find(|(_, s)| rx_schema.is_match(s) && rx_read.is_match(s))
        {
            risk.push(format!("{name} ({rel})"));
        }
    }

    let (verdict, why) = if !risk.is_empty() {
        (
            Verdict::Susceptible,
            "the property name is a literal on both sides".to_string(),
        )
    } else if !protect.is_empty() {
        (Verdict::Protected, protect[0].clone())
    } else {
        (
            Verdict::Unclear,
            "no protective signal and no risk signal, read it by hand".to_string(),
        )
    };

    Classification {
        verdict,
        why,
        files: sources.len(),
        protect,
        risk,
    }
}
