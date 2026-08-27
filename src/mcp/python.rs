//! The Python side of the species.
//!
//! The shape: `types.Tool(name="X", inputSchema={...})` in a list, and next to
//! it an `@app.call_tool()` that branches on the name and reads
//! `arguments.get("k")`. The only thing tying the schema to the handler is the
//! name string.
//!
//! 🔴 A schema built from a pydantic model (`inputSchema=Model.model_json_schema()`)
//! is deliberately out of scope: one source feeds both sides there, so the
//! species cannot exist. The official modelcontextprotocol/servers are built
//! that way.

use rustpython_parser::{ast, Parse};
use std::collections::{BTreeMap, BTreeSet};

/// Protocol housekeeping arrives in the arguments but is not in the schema.
const PROTOCOL_FIELDS: &[&str] = &["_meta", "_progressToken", "progressToken", "signal"];

/// Variable names that hold a tool's arguments.
const BASE_ARG_NAMES: &[&str] = &[
    "arguments",
    "args",
    "tool_args",
    "params",
    "kwargs",
    "tool_input",
    "input_data",
];

const HANDLER_PREFIXES: &[&str] = &[
    "_handle", "handle", "_do", "tool_", "_tool", "do_", "_run", "run_",
];

#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub props: BTreeSet<String>,
    pub required: BTreeSet<String>,
    pub file: String,
    pub line: usize,
}

/// What a body reads out of the arguments dictionary.
#[derive(Debug, Clone, Default)]
pub struct Reads {
    pub reads: BTreeSet<String>,
    /// 🔴 `params.get("boardPath") or params.get("path")`: the code is
    /// deliberately tolerant of several spellings while the schema declares
    /// the canonical one. Everything past the first alternative is not a
    /// finding.
    pub synonyms: BTreeSet<String>,
    /// The whole dictionary is handed on somewhere, so "declared but never
    /// read" cannot be claimed.
    pub opaque: bool,
}

impl Reads {
    pub fn merge(&mut self, other: &Reads) {
        self.reads.extend(other.reads.iter().cloned());
        self.synonyms.extend(other.synonyms.iter().cloned());
        self.opaque |= other.opaque;
    }
}

#[derive(Default)]
pub struct PyScan {
    pub schemas: BTreeMap<String, Schema>,
    /// 🔴 Keys the server writes into the arguments itself before reading
    /// them back: `call_params["_deferSave"] = True`. The agent is not meant
    /// to send those, so the schema is right to stay silent about them. Same
    /// rule as `os.environ.setdefault` in the env check.
    pub self_assigned: BTreeSet<String>,
    /// Branch bodies including the dispatcher preamble.
    pub branches: BTreeMap<String, Reads>,
    /// The same branches without the preamble.
    pub branches_own: BTreeMap<String, Reads>,
    /// `{"save_project": self._handle_save_project}`
    pub dispatch: BTreeMap<String, String>,
    pub funcs: BTreeMap<String, Reads>,
    pub files: usize,
    pub unparsed: Vec<String>,
}

fn literal(e: &ast::Expr) -> Option<String> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn name_of(e: &ast::Expr) -> Option<&str> {
    match e {
        ast::Expr::Name(n) => Some(n.id.as_str()),
        _ => None,
    }
}

fn dict_top_keys(e: &ast::Expr) -> Option<BTreeSet<String>> {
    match e {
        ast::Expr::Dict(d) => Some(
            d.keys
                .iter()
                .filter_map(|k| k.as_ref().and_then(literal))
                .collect(),
        ),
        _ => None,
    }
}

/// 🔴 Only `properties` out of `inputSchema`, never out of `outputSchema`.
/// Mixing the two gave 12 phantom properties on one server.
fn schema_props(e: &ast::Expr) -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let ast::Expr::Dict(d) = e else { return None };
    let mut props: Option<BTreeSet<String>> = None;
    let mut req = BTreeSet::new();
    for (k, v) in d.keys.iter().zip(d.values.iter()) {
        let Some(key) = k.as_ref().and_then(literal) else {
            continue;
        };
        if key == "properties" {
            if let Some(keys) = dict_top_keys(v) {
                props = Some(
                    keys.into_iter()
                        .filter(|n| !PROTOCOL_FIELDS.contains(&n.as_str()))
                        .collect(),
                );
            }
        } else if key == "required" {
            match v {
                ast::Expr::List(l) => req = l.elts.iter().filter_map(literal).collect(),
                ast::Expr::Tuple(t) => req = t.elts.iter().filter_map(literal).collect(),
                _ => {}
            }
        }
    }
    props.map(|p| (p, req))
}

// ---------------------------------------------------------------------------
// Walking a module
// ---------------------------------------------------------------------------

fn walk_exprs<'a>(stmts: &'a [ast::Stmt], out: &mut Vec<&'a ast::Expr>) {
    for st in stmts {
        stmt_exprs(st, out);
    }
}

fn stmt_exprs<'a>(st: &'a ast::Stmt, out: &mut Vec<&'a ast::Expr>) {
    match st {
        ast::Stmt::Expr(e) => expr_tree(&e.value, out),
        ast::Stmt::Assign(a) => expr_tree(&a.value, out),
        ast::Stmt::AnnAssign(a) => {
            if let Some(v) = &a.value {
                expr_tree(v, out)
            }
        }
        ast::Stmt::AugAssign(a) => expr_tree(&a.value, out),
        ast::Stmt::Return(r) => {
            if let Some(v) = &r.value {
                expr_tree(v, out)
            }
        }
        ast::Stmt::FunctionDef(f) => walk_exprs(&f.body, out),
        ast::Stmt::AsyncFunctionDef(f) => walk_exprs(&f.body, out),
        ast::Stmt::ClassDef(c) => walk_exprs(&c.body, out),
        ast::Stmt::If(i) => {
            expr_tree(&i.test, out);
            walk_exprs(&i.body, out);
            walk_exprs(&i.orelse, out);
        }
        ast::Stmt::For(f) => {
            expr_tree(&f.iter, out);
            walk_exprs(&f.body, out);
            walk_exprs(&f.orelse, out);
        }
        ast::Stmt::AsyncFor(f) => {
            expr_tree(&f.iter, out);
            walk_exprs(&f.body, out);
            walk_exprs(&f.orelse, out);
        }
        ast::Stmt::While(w) => {
            expr_tree(&w.test, out);
            walk_exprs(&w.body, out);
            walk_exprs(&w.orelse, out);
        }
        ast::Stmt::With(w) => walk_exprs(&w.body, out),
        ast::Stmt::AsyncWith(w) => walk_exprs(&w.body, out),
        ast::Stmt::Try(t) => {
            walk_exprs(&t.body, out);
            for h in &t.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = h;
                walk_exprs(&h.body, out);
            }
            walk_exprs(&t.orelse, out);
            walk_exprs(&t.finalbody, out);
        }
        ast::Stmt::TryStar(t) => {
            walk_exprs(&t.body, out);
            for h in &t.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = h;
                walk_exprs(&h.body, out);
            }
        }
        ast::Stmt::Match(m) => {
            expr_tree(&m.subject, out);
            for c in &m.cases {
                walk_exprs(&c.body, out);
            }
        }
        _ => {}
    }
}

fn expr_tree<'a>(e: &'a ast::Expr, out: &mut Vec<&'a ast::Expr>) {
    out.push(e);
    match e {
        ast::Expr::Call(c) => {
            expr_tree(&c.func, out);
            for a in &c.args {
                expr_tree(a, out);
            }
            for k in &c.keywords {
                expr_tree(&k.value, out);
            }
        }
        ast::Expr::Dict(d) => {
            for v in &d.values {
                expr_tree(v, out);
            }
            for k in d.keys.iter().flatten() {
                expr_tree(k, out);
            }
        }
        ast::Expr::List(l) => {
            for v in &l.elts {
                expr_tree(v, out)
            }
        }
        ast::Expr::Tuple(t) => {
            for v in &t.elts {
                expr_tree(v, out)
            }
        }
        ast::Expr::Set(s) => {
            for v in &s.elts {
                expr_tree(v, out)
            }
        }
        ast::Expr::BoolOp(b) => {
            for v in &b.values {
                expr_tree(v, out)
            }
        }
        ast::Expr::BinOp(b) => {
            expr_tree(&b.left, out);
            expr_tree(&b.right, out);
        }
        ast::Expr::UnaryOp(u) => expr_tree(&u.operand, out),
        ast::Expr::Subscript(s) => {
            expr_tree(&s.value, out);
            expr_tree(&s.slice, out);
        }
        ast::Expr::Attribute(a) => expr_tree(&a.value, out),
        ast::Expr::Await(a) => expr_tree(&a.value, out),
        ast::Expr::IfExp(i) => {
            expr_tree(&i.test, out);
            expr_tree(&i.body, out);
            expr_tree(&i.orelse, out);
        }
        ast::Expr::JoinedStr(j) => {
            for v in &j.values {
                expr_tree(v, out)
            }
        }
        ast::Expr::FormattedValue(f) => expr_tree(&f.value, out),
        ast::Expr::Lambda(l) => expr_tree(&l.body, out),
        _ => {}
    }
}

fn find_tools(body: &[ast::Stmt], file: &str, line_of: &dyn Fn(usize) -> usize) -> BTreeMap<String, Schema> {
    let mut out = BTreeMap::new();
    let mut exprs = Vec::new();
    walk_exprs(body, &mut exprs);

    for e in &exprs {
        // types.Tool(name="X", inputSchema={...})
        if let ast::Expr::Call(c) = e {
            let fname = match &*c.func {
                ast::Expr::Name(n) => Some(n.id.to_string()),
                ast::Expr::Attribute(a) => Some(a.attr.to_string()),
                _ => None,
            };
            if fname.as_deref() != Some("Tool") {
                continue;
            }
            let mut tool_name = None;
            let mut schema = None;
            for kw in &c.keywords {
                match kw.arg.as_ref().map(|a| a.as_str()) {
                    Some("name") => tool_name = literal(&kw.value),
                    Some("inputSchema") => schema = Some(&kw.value),
                    _ => {}
                }
            }
            if let (Some(name), Some(sch)) = (tool_name, schema) {
                if let Some((props, required)) = schema_props(sch) {
                    out.entry(name).or_insert(Schema {
                        props,
                        required,
                        file: file.to_string(),
                        line: line_of(c.range.start().to_usize()),
                    });
                }
            }
        }
    }

    // {"name": "X", "inputSchema": {...}} as a plain dict
    for e in &exprs {
        let ast::Expr::Dict(d) = e else { continue };
        let mut name = None;
        let mut schema = None;
        for (k, v) in d.keys.iter().zip(d.values.iter()) {
            match k.as_ref().and_then(literal).as_deref() {
                Some("name") => name = literal(v),
                Some("inputSchema") => schema = Some(v),
                _ => {}
            }
        }
        if let (Some(name), Some(sch)) = (name, schema) {
            if let Some((props, required)) = schema_props(sch) {
                out.entry(name).or_insert(Schema {
                    props,
                    required,
                    file: file.to_string(),
                    line: line_of(d.range.start().to_usize()),
                });
            }
        }
    }
    out
}

/// 🔴 Arguments get renamed before being handed on: `call_params = dict(params)`,
/// `safe = sanitize(arguments)`.
///
/// 🔴 But the chain may not be pulled through an arbitrary call:
/// `result = self._do_thing(params)` returns the ANSWER, not the arguments,
/// and its keys (`success`, `project`) were read as schema properties. Only
/// known wrappers. And short names (`r`, `d`, `x`) are never inherited: in
/// opencaselaw `r` was a database row and its keys became phantom properties.
fn arg_names_in(body: &[ast::Stmt]) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = BASE_ARG_NAMES.iter().map(|s| s.to_string()).collect();
    let mut assigns = Vec::new();
    collect_assigns(body, &mut assigns);

    for _ in 0..2 {
        let mut grew = BTreeSet::new();
        for (target, value) in &assigns {
            if target.len() < 4 {
                continue;
            }
            let inherits = match value {
                ast::Expr::Name(n) => names.contains(n.id.as_str()),
                ast::Expr::Call(c) => {
                    let fname = match &*c.func {
                        ast::Expr::Name(n) => n.id.to_string(),
                        ast::Expr::Attribute(a) => a.attr.to_string(),
                        _ => String::new(),
                    };
                    let low = fname.to_lowercase();
                    let wrapper = matches!(fname.as_str(), "dict" | "copy" | "deepcopy")
                        || ["sanitiz", "normaliz", "clean", "merge", "coerce", "validate", "copy"]
                            .iter()
                            .any(|w| low.contains(w));
                    wrapper
                        && c.args
                            .iter()
                            .any(|a| name_of(a).map(|n| names.contains(n)).unwrap_or(false))
                }
                // {**arguments}
                ast::Expr::Dict(d) => d.keys.iter().any(|k| k.is_none()),
                _ => false,
            };
            if inherits {
                grew.insert(target.clone());
            }
        }
        if grew.is_subset(&names) {
            break;
        }
        names.extend(grew);
    }
    names
}

/// Keys assigned into an arguments dictionary: `call_params["k"] = ...`
fn collect_self_assigned(stmts: &[ast::Stmt], names: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    walk_assign_targets(stmts, out, names);
}

fn walk_assign_targets(stmts: &[ast::Stmt], out: &mut BTreeSet<String>, names: &BTreeSet<String>) {
    for st in stmts {
        match st {
            ast::Stmt::Assign(a) => {
                for t in &a.targets {
                    if let ast::Expr::Subscript(sub) = t {
                        if name_of(&sub.value).map(|n| names.contains(n)).unwrap_or(false) {
                            if let Some(k) = literal(&sub.slice) {
                                out.insert(k);
                            }
                        }
                    }
                }
            }
            ast::Stmt::FunctionDef(f) => walk_assign_targets(&f.body, out, names),
            ast::Stmt::AsyncFunctionDef(f) => walk_assign_targets(&f.body, out, names),
            ast::Stmt::ClassDef(c) => walk_assign_targets(&c.body, out, names),
            ast::Stmt::If(i) => {
                walk_assign_targets(&i.body, out, names);
                walk_assign_targets(&i.orelse, out, names);
            }
            ast::Stmt::For(f) => walk_assign_targets(&f.body, out, names),
            ast::Stmt::AsyncFor(f) => walk_assign_targets(&f.body, out, names),
            ast::Stmt::While(w) => walk_assign_targets(&w.body, out, names),
            ast::Stmt::With(w) => walk_assign_targets(&w.body, out, names),
            ast::Stmt::AsyncWith(w) => walk_assign_targets(&w.body, out, names),
            ast::Stmt::Try(t) => {
                walk_assign_targets(&t.body, out, names);
                for h in &t.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    walk_assign_targets(&h.body, out, names);
                }
                walk_assign_targets(&t.finalbody, out, names);
            }
            _ => {}
        }
    }
}

fn collect_assigns<'a>(stmts: &'a [ast::Stmt], out: &mut Vec<(String, &'a ast::Expr)>) {
    for st in stmts {
        match st {
            ast::Stmt::Assign(a) => {
                if a.targets.len() == 1 {
                    if let ast::Expr::Name(n) = &a.targets[0] {
                        out.push((n.id.to_string(), &a.value));
                    }
                }
            }
            ast::Stmt::FunctionDef(f) => collect_assigns(&f.body, out),
            ast::Stmt::AsyncFunctionDef(f) => collect_assigns(&f.body, out),
            ast::Stmt::ClassDef(c) => collect_assigns(&c.body, out),
            ast::Stmt::If(i) => {
                collect_assigns(&i.body, out);
                collect_assigns(&i.orelse, out);
            }
            ast::Stmt::For(f) => collect_assigns(&f.body, out),
            ast::Stmt::AsyncFor(f) => collect_assigns(&f.body, out),
            ast::Stmt::While(w) => collect_assigns(&w.body, out),
            ast::Stmt::With(w) => collect_assigns(&w.body, out),
            ast::Stmt::AsyncWith(w) => collect_assigns(&w.body, out),
            ast::Stmt::Try(t) => {
                collect_assigns(&t.body, out);
                for h in &t.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    collect_assigns(&h.body, out);
                }
                collect_assigns(&t.finalbody, out);
            }
            _ => {}
        }
    }
}

/// Reads performed by a list of statements.
fn reads_of(stmts: &[ast::Stmt], names: &BTreeSet<String>) -> Reads {
    let mut exprs = Vec::new();
    walk_exprs(stmts, &mut exprs);
    reads_of_exprs(&exprs, names)
}

fn reads_of_exprs(exprs: &[&ast::Expr], names: &BTreeSet<String>) -> Reads {
    let mut r = Reads::default();
    for e in exprs {
        match e {
            ast::Expr::Subscript(s) => {
                if name_of(&s.value).map(|n| names.contains(n)).unwrap_or(false) {
                    if let Some(k) = literal(&s.slice) {
                        r.reads.insert(k);
                    }
                }
            }
            ast::Expr::Call(c) => {
                if let ast::Expr::Attribute(f) = &*c.func {
                    if matches!(f.attr.as_str(), "get" | "pop" | "setdefault")
                        && name_of(&f.value).map(|n| names.contains(n)).unwrap_or(false)
                    {
                        if let Some(k) = c.args.first().and_then(literal) {
                            r.reads.insert(k);
                            // arguments.get("a", arguments.get("b")): synonyms
                            if c.args.len() > 1 {
                                let mut inner = Vec::new();
                                expr_tree(&c.args[1], &mut inner);
                                let sub = reads_of_exprs(&inner, names);
                                r.synonyms.extend(sub.reads);
                            }
                        }
                    }
                }
                // the dictionary handed on whole
                for a in &c.args {
                    if name_of(a).map(|n| names.contains(n)).unwrap_or(false) {
                        r.opaque = true;
                    }
                }
                for kw in &c.keywords {
                    if kw.arg.is_none()
                        && name_of(&kw.value).map(|n| names.contains(n)).unwrap_or(false)
                    {
                        r.opaque = true;
                    }
                }
            }
            ast::Expr::BoolOp(b) => {
                if matches!(b.op, ast::BoolOp::Or) && b.values.len() > 1 {
                    for v in &b.values[1..] {
                        let mut inner = Vec::new();
                        expr_tree(v, &mut inner);
                        let sub = reads_of_exprs(&inner, names);
                        r.synonyms.extend(sub.reads);
                    }
                }
            }
            _ => {}
        }
    }
    r
}

/// Branch bodies keyed by the tool name they test for, with and without the
/// dispatcher preamble.
///
/// 🔴 The reading is often lifted OUT of the branch:
///
/// ```text
/// name = arguments.get("name", "")
/// if tool_name == "create_timeline":
///     client.create_timeline(name)
/// ```
///
/// The branch uses a value from the enclosing scope. On one server that gave
/// 42 phantoms across 108 tools.
///
/// 🔴 A preamble is what precedes the branch IN ITS OWN BLOCK, not "everything
/// before the first if in the function". The first version stopped only at
/// `If`, while handler bodies usually open with `try:`, and the preamble
/// swallowed the whole function together with every branch.
fn branch_reads(
    body: &[ast::Stmt],
    names: &BTreeSet<String>,
    with_preamble: &mut BTreeMap<String, Reads>,
    own: &mut BTreeMap<String, Reads>,
) {
    fn walk_block(
        block: &[ast::Stmt],
        names: &BTreeSet<String>,
        with_preamble: &mut BTreeMap<String, Reads>,
        own: &mut BTreeMap<String, Reads>,
    ) {
        let mut seen: Vec<&ast::Stmt> = Vec::new();
        for node in block {
            match node {
                ast::Stmt::If(_) => {
                    let mut cur = node;
                    loop {
                        let ast::Stmt::If(i) = cur else { break };
                        if let Some(name) = name_from_test(&i.test) {
                            let preamble: Vec<ast::Stmt> =
                                seen.iter().map(|s| (*s).clone()).collect();
                            let mut full = reads_of(&preamble, names);
                            let branch = reads_of(&i.body, names);
                            full.merge(&branch);
                            with_preamble.entry(name.clone()).or_default().merge(&full);
                            own.entry(name).or_default().merge(&branch);
                        }
                        walk_block(&i.body, names, with_preamble, own);
                        if i.orelse.len() == 1 && matches!(i.orelse[0], ast::Stmt::If(_)) {
                            cur = &i.orelse[0];
                            continue;
                        }
                        if !i.orelse.is_empty() {
                            walk_block(&i.orelse, names, with_preamble, own);
                        }
                        break;
                    }
                }
                ast::Stmt::Match(m) => {
                    for case in &m.cases {
                        if let ast::Pattern::MatchValue(v) = &case.pattern {
                            if let Some(name) = literal(&v.value) {
                                let preamble: Vec<ast::Stmt> =
                                    seen.iter().map(|s| (*s).clone()).collect();
                                let mut full = reads_of(&preamble, names);
                                let branch = reads_of(&case.body, names);
                                full.merge(&branch);
                                with_preamble.entry(name.clone()).or_default().merge(&full);
                                own.entry(name).or_default().merge(&branch);
                            }
                        }
                        walk_block(&case.body, names, with_preamble, own);
                    }
                }
                ast::Stmt::Try(t) => {
                    walk_block(&t.body, names, with_preamble, own);
                    for h in &t.handlers {
                        let ast::ExceptHandler::ExceptHandler(h) = h;
                        walk_block(&h.body, names, with_preamble, own);
                    }
                    walk_block(&t.finalbody, names, with_preamble, own);
                    seen.push(node);
                }
                ast::Stmt::With(w) => {
                    walk_block(&w.body, names, with_preamble, own);
                    seen.push(node);
                }
                ast::Stmt::AsyncWith(w) => {
                    walk_block(&w.body, names, with_preamble, own);
                    seen.push(node);
                }
                ast::Stmt::For(f) => {
                    walk_block(&f.body, names, with_preamble, own);
                    seen.push(node);
                }
                ast::Stmt::While(w) => {
                    walk_block(&w.body, names, with_preamble, own);
                    seen.push(node);
                }
                other => seen.push(other),
            }
        }
    }

    fn descend(
        stmts: &[ast::Stmt],
        names: &BTreeSet<String>,
        with_preamble: &mut BTreeMap<String, Reads>,
        own: &mut BTreeMap<String, Reads>,
    ) {
        for st in stmts {
            match st {
                ast::Stmt::FunctionDef(f) => {
                    walk_block(&f.body, names, with_preamble, own);
                    descend(&f.body, names, with_preamble, own);
                }
                ast::Stmt::AsyncFunctionDef(f) => {
                    walk_block(&f.body, names, with_preamble, own);
                    descend(&f.body, names, with_preamble, own);
                }
                ast::Stmt::ClassDef(c) => descend(&c.body, names, with_preamble, own),
                _ => {}
            }
        }
    }

    descend(body, names, with_preamble, own);
}

fn name_from_test(test: &ast::Expr) -> Option<String> {
    let ast::Expr::Compare(c) = test else { return None };
    if c.ops.len() != 1 || !matches!(c.ops[0], ast::CmpOp::Eq) || c.comparators.len() != 1 {
        return None;
    }
    literal(&c.comparators[0])
}

/// 🔴 `{"save_project": self._handle_save_project}`. Without following the
/// dispatch table, KiCAD bound 1 tool out of 123.
fn dispatch_map(body: &[ast::Stmt]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut exprs = Vec::new();
    walk_exprs(body, &mut exprs);
    for e in &exprs {
        let ast::Expr::Dict(d) = e else { continue };
        for (k, v) in d.keys.iter().zip(d.values.iter()) {
            let Some(key) = k.as_ref().and_then(literal) else {
                continue;
            };
            let method = match v {
                ast::Expr::Attribute(a) => Some(a.attr.to_string()),
                ast::Expr::Name(n) => Some(n.id.to_string()),
                _ => None,
            };
            if let Some(m) = method {
                if HANDLER_PREFIXES.iter().any(|p| m.starts_with(p)) || m.contains('_') {
                    out.insert(key, m);
                }
            }
        }
    }
    out
}

/// 🔴 Several functions may carry the same name, and which one the dispatcher
/// reaches is not decidable here. KiCAD has two `set_active_layer`: one in
/// `commands/board/__init__.py` that only forwards `params`, and the real one
/// in `commands/board/layers.py` that reads `layer`. Keeping the first hid the
/// server's headline defect; keeping the last would hide the mirror case. Both
/// are merged, on the same asymmetry the classifier uses: an extra read costs
/// a missed finding at worst, a missing read costs a false one.
fn funcs_reads(body: &[ast::Stmt], names: &BTreeSet<String>, out: &mut BTreeMap<String, Reads>) {
    for st in body {
        match st {
            ast::Stmt::FunctionDef(f) => {
                let r = reads_of(&f.body, names);
                out.entry(f.name.to_string()).or_default().merge(&r);
                funcs_reads(&f.body, names, out);
            }
            ast::Stmt::AsyncFunctionDef(f) => {
                let r = reads_of(&f.body, names);
                out.entry(f.name.to_string()).or_default().merge(&r);
                funcs_reads(&f.body, names, out);
            }
            ast::Stmt::ClassDef(c) => funcs_reads(&c.body, names, out),
            _ => {}
        }
    }
}

pub fn scan_file(rel: &str, src: &str, scan: &mut PyScan) {
    let suite = match ast::Suite::parse(src, rel) {
        Ok(s) => s,
        Err(_) => {
            scan.unparsed.push(rel.to_string());
            return;
        }
    };
    scan.files += 1;

    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(src.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: usize| match line_starts.binary_search(&offset) {
        Ok(i) => i + 1,
        Err(i) => i,
    };

    for (name, schema) in find_tools(&suite, rel, &line_of) {
        scan.schemas.entry(name).or_insert(schema);
    }

    let names = arg_names_in(&suite);
    // 🔴 The same tool name lives in several files. Taking the first body by
    // walk order lost the real reading; collect them all and merge.
    collect_self_assigned(&suite, &names, &mut scan.self_assigned);
    branch_reads(&suite, &names, &mut scan.branches, &mut scan.branches_own);
    scan.dispatch.extend(dispatch_map(&suite));
    funcs_reads(&suite, &names, &mut scan.funcs);
}
