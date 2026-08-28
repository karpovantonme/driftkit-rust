//! Reading the Python side of the species with a real parse tree.
//!
//! 🔴 Regular expressions cost the Python kit 25% binding on TypeScript
//! against 100% on Python with a tree. Here the tree also answers the one
//! question regexes cannot: whether a default was passed, which is the whole
//! difference between a crash and a shrug.
//!
//! ```text
//! os.environ["FOO"]         raises KeyError      -> required
//! os.getenv("FOO")          returns None         -> optional
//! os.getenv("FOO", "bar")   returns "bar"        -> optional
//! ```

use rustpython_parser::{ast, Parse};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct PyRead {
    pub name: String,
    pub required: bool,
    pub file: String,
    pub line: usize,
}

#[derive(Default)]
pub struct PyOutcome {
    pub reads: Vec<PyRead>,
    /// Names the code puts into the environment itself before reading them
    /// back: `os.environ["X"] = ...` and `os.environ.setdefault("X", ...)`.
    pub assigned: HashSet<String>,
}

/// Parse one file. Returns None when the file does not parse, so the caller
/// can count it rather than pretend it was clean.
pub fn read_file(rel: &str, src: &str) -> Option<PyOutcome> {
    let suite = ast::Suite::parse(src, rel).ok()?;
    let mut v = Visitor {
        file: rel.to_string(),
        out: PyOutcome::default(),
        lines: line_starts(src),
        guarded: Vec::new(),
    };
    v.block(&suite, 0);
    Some(v.out)
}

fn line_starts(src: &str) -> Vec<usize> {
    let mut v = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            v.push(i + 1);
        }
    }
    v
}

struct Visitor {
    file: String,
    out: PyOutcome,
    lines: Vec<usize>,
    /// Names a surrounding condition has proved present.
    guarded: Vec<String>,
}

impl Visitor {
    /// Is a missing `name` harmless at this point.
    ///
    /// Two different things make it so, and they are not the same shape:
    /// `try/except` catches whatever the block raises, so it covers every
    /// name inside; a condition proves one specific name present, and says
    /// nothing about the others.
    fn safe(&self, name: &str, guard: usize) -> bool {
        guard > 0 || self.guarded.iter().any(|n| n == name)
    }

    fn line_of(&self, offset: usize) -> usize {
        match self.lines.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }

    /// `guard` counts enclosing constructs that make a missing variable
    /// harmless. Both shapes below are everywhere, and counting them as
    /// required would put a finding on code that already handles absence:
    ///
    /// ```text
    /// try:                          if "X" in os.environ:
    ///     v = os.environ["X"]           v = os.environ["X"]
    /// except KeyError:
    ///     v = default
    /// ```
    fn block(&mut self, stmts: &[ast::Stmt], guard: usize) {
        for st in stmts {
            self.stmt(st, guard);
        }
    }

    fn stmt(&mut self, st: &ast::Stmt, guard: usize) {
        match st {
            ast::Stmt::Try(t) => {
                let catches = t.handlers.iter().any(|h| {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    match &h.type_ {
                        None => true,
                        Some(e) => {
                            let d = format!("{e:?}");
                            d.contains("KeyError")
                                || d.contains("Exception")
                                || d.contains("BaseException")
                        }
                    }
                });
                self.block(&t.body, guard + usize::from(catches));
                for h in &t.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    self.block(&h.body, guard);
                }
                self.block(&t.orelse, guard);
                self.block(&t.finalbody, guard);
            }
            ast::Stmt::If(i) => {
                // 🔴 Live false finding on sysreptor: the read sat under
                // `elif load_from_env and 'ENABLED_PLUGINS' in os.environ:`.
                // Typing the root of the condition missed it, because the
                // membership check lives inside a BoolOp. Walk the condition.
                let names = guarded_names(&i.test);
                let added = names.len();
                self.guarded.extend(names);
                self.block(&i.body, guard);
                self.guarded.truncate(self.guarded.len() - added);
                self.block(&i.orelse, guard);
                self.expr(&i.test, guard);
            }
            ast::Stmt::Assign(a) => {
                for target in &a.targets {
                    if let ast::Expr::Subscript(s) = target {
                        if is_environ(&s.value) {
                            if let Some(n) = literal(&s.slice) {
                                self.out.assigned.insert(n);
                            }
                        }
                    }
                }
                self.expr(&a.value, guard);
            }
            ast::Stmt::AnnAssign(a) => {
                if let Some(v) = &a.value {
                    self.expr(v, guard);
                }
            }
            ast::Stmt::AugAssign(a) => self.expr(&a.value, guard),
            ast::Stmt::Expr(e) => self.expr(&e.value, guard),
            ast::Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    self.expr(v, guard);
                }
            }
            ast::Stmt::FunctionDef(f) => self.block(&f.body, guard),
            ast::Stmt::AsyncFunctionDef(f) => self.block(&f.body, guard),
            ast::Stmt::ClassDef(c) => self.block(&c.body, guard),
            ast::Stmt::For(f) => {
                self.expr(&f.iter, guard);
                self.block(&f.body, guard);
                self.block(&f.orelse, guard);
            }
            ast::Stmt::AsyncFor(f) => {
                self.expr(&f.iter, guard);
                self.block(&f.body, guard);
                self.block(&f.orelse, guard);
            }
            ast::Stmt::While(w) => {
                self.expr(&w.test, guard);
                self.block(&w.body, guard);
                self.block(&w.orelse, guard);
            }
            ast::Stmt::With(w) => {
                for item in &w.items {
                    self.expr(&item.context_expr, guard);
                }
                self.block(&w.body, guard);
            }
            ast::Stmt::AsyncWith(w) => {
                for item in &w.items {
                    self.expr(&item.context_expr, guard);
                }
                self.block(&w.body, guard);
            }
            ast::Stmt::TryStar(t) => {
                self.block(&t.body, guard + 1);
                for h in &t.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    self.block(&h.body, guard);
                }
                self.block(&t.orelse, guard);
                self.block(&t.finalbody, guard);
            }
            ast::Stmt::Match(m) => {
                self.expr(&m.subject, guard);
                for case in &m.cases {
                    self.block(&case.body, guard);
                    if let Some(g) = &case.guard {
                        self.expr(g, guard);
                    }
                }
            }
            ast::Stmt::Raise(r) => {
                if let Some(e) = &r.exc {
                    self.expr(e, guard);
                }
                if let Some(e) = &r.cause {
                    self.expr(e, guard);
                }
            }
            ast::Stmt::Assert(a) => {
                self.expr(&a.test, guard);
                if let Some(m) = &a.msg {
                    self.expr(m, guard);
                }
            }
            ast::Stmt::Delete(d) => {
                for t in &d.targets {
                    self.expr(t, guard);
                }
            }
            ast::Stmt::TypeAlias(t) => self.expr(&t.value, guard),
            // nothing readable inside these
            ast::Stmt::Import(_)
            | ast::Stmt::ImportFrom(_)
            | ast::Stmt::Global(_)
            | ast::Stmt::Nonlocal(_)
            | ast::Stmt::Pass(_)
            | ast::Stmt::Break(_)
            | ast::Stmt::Continue(_) => {}
        }
    }

    fn expr(&mut self, e: &ast::Expr, guard: usize) {
        match e {
            ast::Expr::Subscript(s) => {
                if is_environ(&s.value) {
                    if let Some(name) = literal(&s.slice) {
                        let required = !self.safe(&name, guard);
                        self.push(name, required, s.range.start().to_usize());
                    }
                }
                self.expr(&s.value, guard);
                self.expr(&s.slice, guard);
            }
            ast::Expr::Call(c) => {
                self.call(c, guard);
                // 🔴 The callee is part of the tree too. Without this line
                // `os.getenv("X", "").strip()` loses the inner call entirely,
                // because the outer one holds it inside `func` rather than in
                // an argument. On one live project that was 49 distinct names
                // read against 11 seen.
                self.expr(&c.func, guard);
                for a in &c.args {
                    self.expr(a, guard);
                }
                for k in &c.keywords {
                    self.expr(&k.value, guard);
                }
            }
            ast::Expr::BoolOp(b) => {
                // 🔴 `env("MODEL") or "deepseek/deepseek-chat"`: a call on the
                // left of `or` has a fallback, so a missing variable cannot
                // stop anything. Live false finding on a 20-star project,
                // where `env` was the project's own helper returning None
                // rather than django-environ.
                //
                // A subscript is different: `os.environ["X"] or "y"` raises
                // while evaluating the left side, before `or` is reached.
                let fallback = matches!(b.op, ast::BoolOp::Or) && b.values.len() > 1;
                for (i, v) in b.values.iter().enumerate() {
                    let softened = fallback && i + 1 < b.values.len() && !contains_subscript(v);
                    self.expr(v, guard + usize::from(softened));
                }
            }
            ast::Expr::BinOp(b) => {
                self.expr(&b.left, guard);
                self.expr(&b.right, guard);
            }
            ast::Expr::IfExp(i) => {
                self.expr(&i.test, guard);
                // both arms of a conditional already handle the other case
                self.expr(&i.body, guard + 1);
                self.expr(&i.orelse, guard + 1);
            }
            ast::Expr::Tuple(t) => {
                for v in &t.elts {
                    self.expr(v, guard);
                }
            }
            ast::Expr::List(l) => {
                for v in &l.elts {
                    self.expr(v, guard);
                }
            }
            ast::Expr::Dict(d) => {
                for v in &d.values {
                    self.expr(v, guard);
                }
            }
            ast::Expr::Attribute(a) => self.expr(&a.value, guard),
            ast::Expr::Await(a) => self.expr(&a.value, guard),
            ast::Expr::Set(v) => {
                for e in &v.elts {
                    self.expr(e, guard);
                }
            }
            ast::Expr::ListComp(c) => {
                self.expr(&c.elt, guard);
                self.comprehensions(&c.generators, guard);
            }
            ast::Expr::SetComp(c) => {
                self.expr(&c.elt, guard);
                self.comprehensions(&c.generators, guard);
            }
            ast::Expr::GeneratorExp(c) => {
                self.expr(&c.elt, guard);
                self.comprehensions(&c.generators, guard);
            }
            ast::Expr::DictComp(c) => {
                self.expr(&c.key, guard);
                self.expr(&c.value, guard);
                self.comprehensions(&c.generators, guard);
            }
            ast::Expr::Lambda(l) => self.expr(&l.body, guard),
            ast::Expr::NamedExpr(n) => self.expr(&n.value, guard),
            ast::Expr::UnaryOp(u) => self.expr(&u.operand, guard),
            ast::Expr::Compare(c) => {
                self.expr(&c.left, guard);
                for e in &c.comparators {
                    self.expr(e, guard);
                }
            }
            ast::Expr::Starred(st) => self.expr(&st.value, guard),
            ast::Expr::Slice(sl) => {
                for e in [&sl.lower, &sl.upper, &sl.step].into_iter().flatten() {
                    self.expr(e, guard);
                }
            }
            ast::Expr::Yield(y) => {
                if let Some(e) = &y.value {
                    self.expr(e, guard);
                }
            }
            ast::Expr::YieldFrom(y) => self.expr(&y.value, guard),
            // 🔴 An f-string is a tree, not a string: `f"{os.environ['X']}"`
            // holds a real read, and the Python kit saw it through
            // `generic_visit` for free.
            ast::Expr::JoinedStr(j) => {
                for e in &j.values {
                    self.expr(e, guard);
                }
            }
            ast::Expr::FormattedValue(f) => self.expr(&f.value, guard),
            ast::Expr::Constant(_) | ast::Expr::Name(_) => {}
        }
    }

    fn comprehensions(&mut self, gens: &[ast::Comprehension], guard: usize) {
        for g in gens {
            self.expr(&g.iter, guard);
            for cond in &g.ifs {
                self.expr(cond, guard);
            }
        }
    }

    fn call(&mut self, c: &ast::ExprCall, guard: usize) {
        let offset = c.range.start().to_usize();
        match &*c.func {
            ast::Expr::Attribute(f) => {
                let attr = f.attr.as_str();
                // os.environ.setdefault("X", ...) puts the value there itself
                if attr == "setdefault" && is_environ(&f.value) {
                    if let Some(n) = c.args.first().and_then(literal) {
                        self.out.assigned.insert(n);
                        return;
                    }
                }
                let name = c.args.first().and_then(literal);
                let Some(name) = name else { return };
                let required = match attr {
                    "getenv" if is_os(&f.value) => false,
                    "get" if is_environ(&f.value) => false,
                    "pop" if is_environ(&f.value) => c.args.len() == 1 && c.keywords.is_empty(),
                    // django-environ: env.str("X"), env.bool("X", default=False)
                    "str" | "bool" | "int" | "float" | "list" | "dict" | "url" | "db"
                    | "cache" | "json" | "path"
                        if is_env_obj(&f.value) =>
                    {
                        !has_default(c)
                    }
                    _ => return,
                };
                let required = required && !self.safe(&name, guard);
                self.push(name, required, offset);
            }
            ast::Expr::Name(f)
                // env("X") / config("X") -- django-environ, decouple, starlette
                if (f.id.as_str() == "env" || f.id.as_str() == "config") => {
                    if let Some(name) = c.args.first().and_then(literal) {
                        let required = !has_default(c);
                        let required = required && !self.safe(&name, guard);
                self.push(name, required, offset);
                    }
                }
            _ => {}
        }
    }

    fn push(&mut self, name: String, required: bool, offset: usize) {
        let line = self.line_of(offset);
        self.out.reads.push(PyRead {
            name,
            required,
            file: self.file.clone(),
            line,
        });
    }
}

/// Does this expression read the environment by subscript, which raises on
/// the spot rather than returning something falsy.
fn contains_subscript(e: &ast::Expr) -> bool {
    match e {
        ast::Expr::Subscript(s) => is_environ(&s.value),
        ast::Expr::Call(c) => c.args.iter().any(contains_subscript),
        ast::Expr::BoolOp(b) => b.values.iter().any(contains_subscript),
        _ => false,
    }
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

fn is_os(e: &ast::Expr) -> bool {
    matches!(e, ast::Expr::Name(n) if n.id.as_str() == "os")
}

fn is_environ(e: &ast::Expr) -> bool {
    match e {
        ast::Expr::Attribute(a) => a.attr.as_str() == "environ",
        ast::Expr::Name(n) => n.id.as_str() == "environ",
        _ => false,
    }
}

fn is_env_obj(e: &ast::Expr) -> bool {
    matches!(e, ast::Expr::Name(n) if matches!(n.id.as_str(), "env" | "config" | "Env"))
}

/// A second positional argument or a `default=` keyword means no crash.
fn has_default(c: &ast::ExprCall) -> bool {
    c.args.len() > 1
        || c.keywords
            .iter()
            .any(|k| k.arg.as_ref().map(|a| a.as_str()) == Some("default"))
}

/// Does this condition ask whether the variable is there at all.
///
/// Two shapes count, and the second cost 15 false findings on one live
/// project before it was added:
///
/// ```text
/// if "X" in os.environ:          if os.environ.get("X"):
///     v = os.environ["X"]            args.x = os.environ["X"]
/// ```
///
/// 🔴 The check is per name, not per branch. A blanket guard would silence
/// `os.environ["B"]` sitting under `if os.environ.get("A"):`, which is a
/// genuine crash.
/// Names this condition proves present.
fn guarded_names(test: &ast::Expr) -> Vec<String> {
    let mut out = Vec::new();
    collect_guarded(test, &mut out);
    out
}

fn collect_guarded(test: &ast::Expr, out: &mut Vec<String>) {
    match test {
        ast::Expr::Compare(c) => {
            let is_in = c
                .ops
                .iter()
                .any(|o| matches!(o, ast::CmpOp::In | ast::CmpOp::NotIn));
            if is_in && c.comparators.iter().any(is_environ) {
                if let Some(n) = literal(c.left.as_ref()) {
                    out.push(n);
                }
            }
        }
        ast::Expr::Call(c) => {
            // if os.environ.get("X"): / if os.getenv("X"):
            let reads_env = match &*c.func {
                ast::Expr::Attribute(f) => {
                    (f.attr.as_str() == "get" && is_environ(&f.value))
                        || (f.attr.as_str() == "getenv" && is_os(&f.value))
                }
                _ => false,
            };
            if reads_env {
                if let Some(n) = c.args.first().and_then(literal) {
                    out.push(n);
                }
            }
        }
        ast::Expr::BoolOp(b) => {
            for v in &b.values {
                collect_guarded(v, out);
            }
        }
        ast::Expr::UnaryOp(u) => collect_guarded(&u.operand, out),
        _ => {}
    }
}
