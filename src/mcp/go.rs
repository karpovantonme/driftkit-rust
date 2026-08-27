//! The Go side of the species: a tool schema against what its handler reads.
//!
//! The most susceptible substrate there is. The property name is a string
//! literal on **both** sides and nothing connects them:
//!
//! ```text
//! schema:   Properties: map[string]*jsonschema.Schema{"owner": {...}}
//!           mcp.WithString("owner", ...)
//! handler:  RequiredParam[string](args, "owner")
//!           args["owner"]
//! ```
//!
//! The compiler has no opinion about whether those two spellings match.

use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use crate::core;

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub props: BTreeSet<String>,
    pub required: BTreeSet<String>,
    pub reads: BTreeSet<String>,
    pub file: String,
    pub line: usize,
    /// The handler hands `args` on somewhere this scanner cannot follow, so
    /// "declared but never read" cannot be claimed for it.
    pub opaque: bool,
}

/// Replace comment contents with spaces, keeping length and newlines.
///
/// 🔴 Without this, commented-out code reads as live code: in
/// github-mcp-server a `// mcp.WithString("pullRequestReviewID", ...)` gave a
/// phantom schema property that does not exist anywhere.
pub fn blank_comments(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        let ch = b[i];
        if ch == '"' || ch == '`' || ch == '\'' {
            let quote = ch;
            out.push(ch);
            i += 1;
            while i < b.len() {
                if b[i] == '\\' && quote != '`' {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    continue;
                }
                out.push(b[i]);
                if b[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if ch == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if ch == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            while i < b.len() && !(b[i] == '*' && i + 1 < b.len() && b[i + 1] == '/') {
                out.push(if b[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            out.push(' ');
            out.push(' ');
            i += 2;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Index of the brace closing the one at `start`, skipping string literals.
fn balanced(src: &[char], start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut j = start;
    while j < src.len() {
        let ch = src[j];
        match in_str {
            Some(q) => {
                if ch == '\\' && q != '`' {
                    j += 2;
                    continue;
                }
                if ch == q {
                    in_str = None;
                }
            }
            None => {
                if ch == '"' || ch == '`' {
                    in_str = Some(ch);
                } else if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
            }
        }
        j += 1;
    }
    None
}

/// String keys of a map literal, first level only.
fn top_keys(inner: &str) -> BTreeSet<String> {
    static KEY: OnceLock<Regex> = OnceLock::new();
    let key = KEY.get_or_init(|| Regex::new(r#""([\w\-.]+)"\s*$"#).unwrap());

    let mut keys = BTreeSet::new();
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut buf = String::new();
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = in_str {
            if ch == '\\' && q != '`' {
                buf.push(ch);
                if i + 1 < chars.len() {
                    buf.push(chars[i + 1]);
                }
                i += 2;
                continue;
            }
            if ch == q {
                in_str = None;
            }
            buf.push(ch);
            i += 1;
            continue;
        }
        if ch == '"' || ch == '`' {
            in_str = Some(ch);
            buf.push(ch);
            i += 1;
            continue;
        }
        if "{[(".contains(ch) {
            depth += 1;
        } else if "}])".contains(ch) {
            depth -= 1;
        } else if ch == ':' && depth == 0 {
            if let Some(c) = key.captures(&buf) {
                keys.insert(c[1].to_string());
            }
        }
        if depth == 0 && ch == ',' {
            buf.clear();
        } else {
            buf.push(ch);
        }
        i += 1;
    }
    keys
}

/// Cut a file into top-level functions.
///
/// 🔴 The boundary has to be a function, not "the next N characters". A
/// sliding window overlapped neighbouring tools and produced findings in
/// batches: the same `actions_run_trigger` came out twice with two different
/// schemas.
fn go_funcs(src: &str) -> Vec<(usize, String)> {
    static RX: OnceLock<Regex> = OnceLock::new();
    let rx = RX.get_or_init(|| Regex::new(r"(?m)^func\b").unwrap());
    let starts: Vec<usize> = rx.find_iter(src).map(|m| m.start()).collect();
    let mut out = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).copied().unwrap_or(src.len());
        out.push((s, src[s..e].to_string()));
    }
    out
}

#[derive(Default)]
struct Helpers {
    /// Functions that add properties to a schema after the literal.
    enrichers: BTreeMap<String, BTreeSet<String>>,
    /// Functions that are handed `args` whole and read properties themselves.
    readers: BTreeMap<String, BTreeSet<String>>,
}

fn schema_props_in(fnbody: &str) -> BTreeSet<String> {
    static PROP_ASSIGN: OnceLock<Regex> = OnceLock::new();
    static PROP_MAP: OnceLock<Regex> = OnceLock::new();
    let assign = PROP_ASSIGN
        .get_or_init(|| Regex::new(r#"\.Properties\s*\[\s*"([\w\-.]+)"\s*\]\s*="#).unwrap());
    let map = PROP_MAP.get_or_init(|| {
        Regex::new(r"Properties\s*:\s*map\[string\]\*?\w*\.?\w*Schema\s*\{").unwrap()
    });

    let mut props: BTreeSet<String> = assign
        .captures_iter(fnbody)
        .map(|c| c[1].to_string())
        .collect();

    if let Some(m) = map.find(fnbody) {
        let chars: Vec<char> = fnbody.chars().collect();
        // byte offset to char offset
        let start_char = fnbody[..m.end()].chars().count() - 1;
        if let Some(open) = (start_char..chars.len()).find(|&i| chars[i] == '{') {
            if let Some(end) = balanced(&chars, open, '{', '}') {
                let inner: String = chars[open + 1..end].iter().collect();
                props.extend(top_keys(&inner));
            }
        }
    }
    props
}

/// 🔴 The schema is built up by code after the literal, and that has to be
/// accounted for. In github-mcp-server `WithPagination(schema)` adds
/// page/perPage and `schema.Properties["fields"] = ...` adds fields. Reading
/// the literal alone declares them missing and puts a phantom on every
/// paginated tool, which is a third of the server.
fn collect_helpers(sources: &[String]) -> Helpers {
    static IS_ENRICHER: OnceLock<Regex> = OnceLock::new();
    static TAKES_ARGS: OnceLock<Regex> = OnceLock::new();
    static FN_NAME: OnceLock<Regex> = OnceLock::new();
    let is_enricher =
        IS_ENRICHER.get_or_init(|| Regex::new(r"func\s+\w+\([^)]*\*?\w*\.?\w*Schema").unwrap());
    let takes_args = TAKES_ARGS
        .get_or_init(|| Regex::new(r"func\s+\w+\([^)]*\b(?:args|arguments)\s+map\[string\]").unwrap());
    let fn_name = FN_NAME.get_or_init(|| Regex::new(r"func\s+(\w+)").unwrap());

    let mut h = Helpers::default();
    for src in sources {
        for (_, body) in go_funcs(src) {
            let Some(name) = fn_name.captures(&body).map(|c| c[1].to_string()) else {
                continue;
            };
            let props = schema_props_in(&body);
            if !props.is_empty() && is_enricher.is_match(&body) {
                h.enrichers.insert(name.clone(), props);
            }
            // 🔴 Functions taking `args map[string]any`: the handler passes
            // args on whole and THEY read the property. Without this, ui_get
            // looks like it never reads `repo`, while `uiGetLabels(.., args,
            // ..)` does.
            if takes_args.is_match(&body) {
                let (reads, _) = reads_of(&body);
                if !reads.is_empty() {
                    h.readers.insert(name, reads);
                }
            }
        }
    }
    h
}

/// What a handler body reads out of `args`.
///
/// 🔴 Helpers cannot be listed by name: every project writes its own.
/// `RequiredBigInt` was not on the list and produced a phantom on
/// `add_issue_comment_reaction`. Match the shape instead: any `f(args, "x")`
/// and `f[T](args, "x")`.
fn reads_of(body: &str) -> (BTreeSet<String>, bool) {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    static OPAQUE: OnceLock<Regex> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r#"\w+(?:\[[^\]]*\])?\(\s*(?:args|arguments|params|a)\s*,\s*"([\w\-.]+)""#).unwrap(),
            Regex::new(r#"\bargs\s*\[\s*"([\w\-.]+)"\s*\]"#).unwrap(),
            Regex::new(r#"request\.(?:GetString|GetInt|GetBool|GetFloat|RequireString|RequireInt|RequireBool)\(\s*"([\w\-.]+)""#).unwrap(),
            Regex::new(r#"req\.(?:GetString|GetInt|GetBool)\(\s*"([\w\-.]+)""#).unwrap(),
        ]
    });
    let opaque_rx =
        OPAQUE.get_or_init(|| Regex::new(r"\bargs\b\s*\)|\.\.\.args\b|json\.Marshal\(\s*args\s*\)").unwrap());

    let mut reads = BTreeSet::new();
    for rx in patterns {
        for c in rx.captures_iter(body) {
            reads.insert(c[1].to_string());
        }
    }
    (reads, opaque_rx.is_match(body))
}

fn tools_in(src: &str, path: &str, helpers: &Helpers) -> Vec<Tool> {
    static WITH: OnceLock<Regex> = OnceLock::new();
    static NAME: OnceLock<Regex> = OnceLock::new();
    static NEWTOOL: OnceLock<Regex> = OnceLock::new();
    static REQUIRED: OnceLock<Regex> = OnceLock::new();
    static CALL: OnceLock<Regex> = OnceLock::new();
    static LITERAL: OnceLock<Regex> = OnceLock::new();
    let with = WITH.get_or_init(|| Regex::new(r#"mcp\.With\w+\(\s*"([\w\-.]+)""#).unwrap());
    let name_rx = NAME.get_or_init(|| Regex::new(r#"Name\s*:\s*"([\w\-.]+)""#).unwrap());
    let newtool = NEWTOOL.get_or_init(|| Regex::new(r#"mcp\.NewTool\s*\(\s*"([\w\-.]+)""#).unwrap());
    let required = REQUIRED.get_or_init(|| Regex::new(r"Required\s*:\s*\[\]string\{([^}]*)\}").unwrap());
    let call = CALL.get_or_init(|| Regex::new(r"\b(\w+)\s*\(").unwrap());
    let literal = LITERAL.get_or_init(|| Regex::new(r#""([\w\-.]+)""#).unwrap());

    let mut out = Vec::new();
    for (off, body) in go_funcs(src) {
        let mut props = schema_props_in(&body);
        props.extend(with.captures_iter(&body).map(|c| c[1].to_string()));
        // enrichers called from this function
        for c in call.captures_iter(&body) {
            if let Some(extra) = helpers.enrichers.get(&c[1]) {
                props.extend(extra.iter().cloned());
            }
        }
        if props.is_empty() {
            continue;
        }

        let name = name_rx
            .captures(&body)
            .or_else(|| newtool.captures(&body))
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| "?".to_string());

        let req: BTreeSet<String> = required
            .captures(&body)
            .map(|c| {
                literal
                    .captures_iter(&c[1])
                    .map(|m| m[1].to_string())
                    .collect()
            })
            .unwrap_or_default();

        let (mut reads, opaque) = reads_of(&body);
        // Two levels of expansion into helpers, and no deeper.
        //
        // 🔴 Only calls where `args` is ACTUALLY passed. Expanding by function
        // name alone mixed the shared pagination `after` into everything and
        // blew the findings up from 8 to 24 in one run.
        static PASSES_ARGS: OnceLock<Regex> = OnceLock::new();
        let passes = PASSES_ARGS
            .get_or_init(|| Regex::new(r"\b(\w+)\s*\(\s*(?:[^()]*?,\s*)?args\s*[,)]").unwrap());
        for _ in 0..2 {
            let mut grew = BTreeSet::new();
            for c in passes.captures_iter(&body) {
                if let Some(more) = helpers.readers.get(&c[1]) {
                    grew.extend(more.iter().cloned());
                }
            }
            if grew.is_subset(&reads) {
                break;
            }
            reads.extend(grew);
        }
        if reads.is_empty() {
            continue;
        }

        out.push(Tool {
            name,
            props,
            required: req,
            reads,
            file: path.to_string(),
            line: src[..off].matches('\n').count() + 1,
            opaque,
        });
    }
    out
}

pub struct GoScan {
    pub tools: Vec<Tool>,
    pub files: usize,
}

pub fn scan(root: &Path) -> GoScan {
    let mut sources: Vec<(String, String)> = Vec::new();
    for path in core::walk(root) {
        if path.extension().map(|e| e != "go").unwrap_or(true) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.contains("_test.go") || rel.contains("/vendor/") || rel.contains("/testdata/") {
            continue;
        }
        if let Some(src) = core::read_text(&path) {
            sources.push((rel, blank_comments(&src)));
        }
    }

    let blanked: Vec<String> = sources.iter().map(|(_, s)| s.clone()).collect();
    let helpers = collect_helpers(&blanked);

    let mut tools = Vec::new();
    for (rel, src) in &sources {
        if !src.contains("Properties") && !src.contains("mcp.NewTool") {
            continue;
        }
        tools.extend(tools_in(src, rel, &helpers));
    }

    GoScan {
        tools,
        files: sources.len(),
    }
}
