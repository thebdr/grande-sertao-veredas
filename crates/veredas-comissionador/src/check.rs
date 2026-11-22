//! The whole check of a test and the function files it uses (`docs/test-schema.md`, "What a
//! check refuses"), before any run: every problem named with its file, line and column.
//!
//! The check also settles what each `<name>.<column>` is: a row's column when the name is a
//! variable, plant data otherwise. The model it returns is the one the engine runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::Problem;
use crate::model::*;
use crate::parse::{parse_functions, parse_test};

/// A test that passed its check, with the function files it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub test: TestFile,
    /// The test file's name, as problems name it.
    pub file: String,
    pub files: Vec<UsedFile>,
    pub counts: Counts,
}

impl Checked {
    /// A function of the files the test uses, by name.
    pub fn function(&self, name: &str) -> Option<&Function> {
        self.files.iter().flat_map(|f| f.functions.functions.iter()).find(|f| f.name.value == name)
    }
}

/// A function file a test uses.
#[derive(Clone, Debug, PartialEq)]
pub struct UsedFile {
    /// As the test's `use` writes it.
    pub path: String,
    /// As problems name it: the path from the test file's folder.
    pub file: String,
    pub text: String,
    pub functions: FunctionFile,
}

/// What the designer's problems bar counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub steps: usize,
    pub branches: usize,
    /// The functions the test calls, each once.
    pub functions: usize,
    /// Its writes and sends: every action that touches the plant through portal's gate.
    pub gated_writes: usize,
}

/// Read and check a test file on disk, with the function files it uses, each `use` read
/// relative to the test file's folder.
pub fn load_test(path: &Path) -> Result<Checked, Vec<Problem>> {
    let file = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| vec![Problem { file: file.clone(), line: 1, column: 1, message: format!("cannot be read: {e}") }])?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    check_test(&text, &file, &|p| std::fs::read_to_string(dir.join(p)).map_err(|e| e.to_string()))
}

/// Read and check a function file on disk, alone.
pub fn load_functions(path: &Path) -> Result<FunctionFile, Vec<Problem>> {
    let file = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| vec![Problem { file: file.clone(), line: 1, column: 1, message: format!("cannot be read: {e}") }])?;
    check_functions(&text, &file)
}

/// Check a function file alone: read, then every function checked against its own file.
pub fn check_functions(text: &str, file: &str) -> Result<FunctionFile, Vec<Problem>> {
    let mut ff = parse_functions(text, file).map_err(|p| vec![p])?;
    let mut out = Vec::new();
    check_function_file(&mut ff, file, text, &mut out);
    if out.is_empty() { Ok(ff) } else { Err(out) }
}

/// Check a test's text. `resolve` reads a `use`d file by the path the test writes, relative to
/// the test file's folder (from disk, or from the designer's unsaved buffers).
pub fn check_test(text: &str, file: &str, resolve: &dyn Fn(&str) -> Result<String, String>) -> Result<Checked, Vec<Problem>> {
    let mut test = parse_test(text, file).map_err(|p| vec![p])?;
    let mut out: Vec<Problem> = Vec::new();
    let at = |span: Span, msg: String| Problem::at(file, text, span.start, msg);
    let dir = Path::new(file).parent().map(Path::to_path_buf).unwrap_or_default();

    // The settings.
    let mut seen: BTreeMap<&'static str, Span> = BTreeMap::new();
    let mut lets: BTreeSet<String> = BTreeSet::new();
    let mut uses: BTreeSet<String> = BTreeSet::new();
    for s in &mut test.settings {
        let key = s.kind.key();
        if !matches!(key, "use" | "let") && seen.insert(key, s.key_span).is_some() {
            out.push(at(s.key_span, format!("`{key}` is given twice")));
        }
        match &mut s.kind {
            SettingKind::Id(id) => {
                if !id.value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    out.push(at(id.span, format!("`id` is `{}`: letters, digits, `-` and `_`", id.value)));
                }
            }
            SettingKind::Use(p) => {
                if !uses.insert(p.value.clone()) {
                    out.push(at(p.span, format!("`{}` is used twice", p.value)));
                }
            }
            SettingKind::Modes(modes) => {
                let mut m = BTreeSet::new();
                for mode in modes.iter() {
                    if !m.insert(mode.value) {
                        out.push(at(mode.span, format!("`{}` is named twice in `modes`", mode.value)));
                    }
                }
            }
            SettingKind::Let { name, value } => {
                let consts = lets.clone();
                resolve_expr(value, &consts, &Plants::Let, &[], &mut |span, msg| out.push(at(span, msg)));
                if !lets.insert(name.value.clone()) {
                    out.push(at(name.span, format!("`{}` is defined twice", name.value)));
                }
            }
            _ => {}
        }
    }
    if test.id().is_none() {
        out.push(at(test.head, "the test has no `id`".to_string()));
    }

    // The function files, each checked against itself, then their names across them.
    let mut files: Vec<UsedFile> = Vec::new();
    for u in test.uses() {
        if files.iter().any(|f| f.path == u.value) {
            continue;
        }
        let shown = display_path(&dir, &u.value);
        match resolve(&u.value) {
            Err(reason) => out.push(at(u.span, format!("`{}` cannot be read: {reason}", u.value))),
            Ok(ftext) => match parse_functions(&ftext, &shown) {
                Err(p) => out.push(p),
                Ok(mut ff) => {
                    check_function_file(&mut ff, &shown, &ftext, &mut out);
                    files.push(UsedFile { path: u.value.clone(), file: shown, text: ftext, functions: ff });
                }
            },
        }
    }
    let mut named: BTreeMap<String, (String, Place)> = BTreeMap::new();
    for f in &files {
        for func in &f.functions.functions {
            let place = Place::of(&f.text, func.name.span.start);
            match named.get(&func.name.value) {
                Some((other, p)) if other != &f.file => out.push(Problem::at(
                    &f.file,
                    &f.text,
                    func.name.span.start,
                    format!("two functions are named `{}`: the other is at {other}:{}:{}", func.name.value, p.line, p.column),
                )),
                Some(_) => {}
                None => {
                    named.insert(func.name.value.clone(), (f.file.clone(), place));
                }
            }
        }
    }
    // A name taken twice resolves to its first function, the one the problem above points back to.
    let mut functions: BTreeMap<String, &Function> = BTreeMap::new();
    for f in files.iter().flat_map(|f| f.functions.functions.iter()) {
        functions.entry(f.name.value.clone()).or_insert(f);
    }
    let fnames: Vec<&str> = functions.keys().map(String::as_str).collect();

    // The test's variables: its constants, and every name a step assigns.
    let mut vars = lets.clone();
    for step in &test.steps {
        for a in &step.actions {
            match &a.kind {
                ActionKind::Read { into, .. } => vars.extend(into.iter().map(|n| n.value.clone())),
                ActionKind::Await { bind, .. } => {
                    vars.insert(bind.value.clone());
                }
                ActionKind::Call { outputs, .. } => vars.extend(outputs.iter().map(|n| n.value.clone())),
                ActionKind::Script { body } => assigned(body, &mut vars),
                _ => {}
            }
        }
    }

    // The steps.
    let mut steps: BTreeMap<String, Span> = BTreeMap::new();
    for step in &test.steps {
        if steps.contains_key(&step.name.value) {
            out.push(at(step.name.span, format!("step `{}` is defined twice", step.name.value)));
        } else {
            steps.insert(step.name.value.clone(), step.name.span);
        }
    }
    let mut called: BTreeSet<String> = BTreeSet::new();
    let mut counts = Counts { steps: test.steps.len(), ..Counts::default() };
    for step in &mut test.steps {
        let step_name = step.name.value.clone();
        let is_branch = matches!(step.actions.last().map(|a| &a.kind), Some(ActionKind::Branch { .. }));
        for a in &mut step.actions {
            if a.is_gated() {
                counts.gated_writes += 1;
            }
            let mut push = |span: Span, msg: String| out.push(Problem::at(file, text, span.start, msg));
            match &mut a.kind {
                ActionKind::Write { value, .. } => resolve_expr(value, &vars, &Plants::Any, &fnames, &mut push),
                ActionKind::Send { fields, .. } => {
                    let mut names = BTreeSet::new();
                    for f in fields.iter_mut() {
                        if !names.insert(f.name.value.clone()) {
                            push(f.name.span, format!("field `{}` is given twice", f.name.value));
                        }
                        resolve_expr(&mut f.value, &vars, &Plants::Any, &fnames, &mut push);
                    }
                }
                ActionKind::Wait { condition, .. } => resolve_expr(condition, &vars, &Plants::Any, &fnames, &mut push),
                ActionKind::Await { filter, bind, .. } => {
                    constant_assigned(bind, &lets, &mut push);
                    if let Some(f) = filter {
                        resolve_expr(f, &vars, &Plants::Any, &fnames, &mut push);
                    }
                }
                ActionKind::Read { into, .. } => into.iter().for_each(|n| constant_assigned(n, &lets, &mut push)),
                ActionKind::Check { condition, .. } => resolve_expr(condition, &vars, &Plants::Any, &fnames, &mut push),
                ActionKind::Call { function, args, outputs } => {
                    for arg in args.iter_mut() {
                        resolve_expr(&mut arg.value, &vars, &Plants::Any, &fnames, &mut push);
                    }
                    outputs.iter().for_each(|n| constant_assigned(n, &lets, &mut push));
                    match functions.get(&function.value) {
                        None => push(function.span, format!("`{}` is no function of the files this test uses", function.value)),
                        Some(f) => {
                            called.insert(function.value.clone());
                            check_call(function, args, outputs, f, &mut push);
                        }
                    }
                }
                ActionKind::Script { body } => {
                    let ctx = StmtCtx::Script { consts: &lets, functions: &fnames };
                    check_stmts(body, &vars, &ctx, &mut push);
                }
                ActionKind::Branch { value, arms, .. } => {
                    counts.branches += 1;
                    resolve_expr(value, &vars, &Plants::Any, &fnames, &mut push);
                    for arm in arms.iter_mut() {
                        for p in arm.patterns.iter_mut() {
                            match p {
                                Pattern::Compare { value, .. } | Pattern::Value(value) => {
                                    resolve_expr(value, &vars, &Plants::Any, &fnames, &mut push)
                                }
                                Pattern::Range { low, high } => {
                                    resolve_expr(low, &vars, &Plants::Any, &fnames, &mut push);
                                    resolve_expr(high, &vars, &Plants::Any, &fnames, &mut push);
                                }
                            }
                        }
                    }
                }
                ActionKind::Report { items } => {
                    for item in items.iter_mut() {
                        resolve_expr(item, &vars, &Plants::Any, &fnames, &mut push);
                    }
                }
                ActionKind::Stop { reason, .. } => {
                    if let Some(r) = reason {
                        resolve_expr(r, &vars, &Plants::Any, &fnames, &mut push);
                    }
                }
                ActionKind::Delay { .. } => {}
            }
        }
        // Routes.
        let mut routes: Vec<&Spanned<Route>> = step.on.iter().map(|o| &o.route).collect();
        for a in &step.actions {
            if let ActionKind::Branch { arms, otherwise, .. } = &a.kind {
                routes.extend(arms.iter().map(|arm| &arm.route));
                routes.extend(otherwise.iter());
            }
        }
        for r in routes {
            let target = match &r.value {
                Route::Step(s) => Some(s),
                Route::Retry { step, max } => {
                    if max.value == 0 {
                        out.push(at(max.span, format!("`retry {} max 0` never retries: `max` is at least 1", step.value)));
                    }
                    Some(step)
                }
                Route::Stop(_) => None,
            };
            if let Some(t) = target
                && !steps.contains_key(&t.value)
            {
                out.push(at(t.span, format!("`{}` is no step of this test", t.value)));
            }
        }
        let mut routed: BTreeSet<Outcome> = BTreeSet::new();
        for on in &step.on {
            for o in &on.outcomes {
                if o.value == Outcome::Pass && is_branch {
                    out.push(at(o.span, format!("a branch routes its `pass` by its arms: `on pass` in step `{step_name}`")));
                } else if !routed.insert(o.value) {
                    out.push(at(o.span, format!("`{}` is routed twice in step `{step_name}`", o.value)));
                }
            }
        }
    }
    counts.functions = called.len();

    // The layout.
    if let Some(layout) = &test.layout {
        let mut placed = BTreeSet::new();
        for p in &layout.positions {
            if !steps.contains_key(&p.step.value) {
                out.push(at(p.step.span, format!("`{}` in the layout is no step of this test", p.step.value)));
            } else if !placed.insert(p.step.value.clone()) {
                out.push(at(p.step.span, format!("`{}` is placed twice in the layout", p.step.value)));
            }
        }
    }

    if out.is_empty() { Ok(Checked { test, file: file.to_string(), files, counts }) } else { Err(out) }
}

/// The path problems name a used file by: from the test file's folder.
fn display_path(dir: &Path, path: &str) -> String {
    if dir.as_os_str().is_empty() { path.to_string() } else { PathBuf::from(dir).join(path).display().to_string() }
}

/// Every name a body assigns: assignment targets, loop names, await rows, call outputs.
fn assigned(body: &[Stmt], into: &mut BTreeSet<String>) {
    for s in body {
        match &s.kind {
            StmtKind::Assign { target, .. } => {
                into.insert(target.value.clone());
            }
            StmtKind::If { arms, otherwise } => {
                for (_, b) in arms {
                    assigned(b, into);
                }
                if let Some(b) = otherwise {
                    assigned(b, into);
                }
            }
            StmtKind::For { var, body, .. } => {
                into.insert(var.value.clone());
                assigned(body, into);
            }
            StmtKind::Await { bind, .. } => {
                into.insert(bind.value.clone());
            }
            StmtKind::Call { outputs, .. } => into.extend(outputs.iter().map(|o| o.value.clone())),
            StmtKind::Fail { .. } | StmtKind::Wait { .. } | StmtKind::Delay { .. } => {}
        }
    }
}

fn constant_assigned(name: &Spanned<String>, consts: &BTreeSet<String>, push: &mut dyn FnMut(Span, String)) {
    if consts.contains(&name.value) {
        push(name.span, format!("`{}` is a constant (`let`) and is never assigned", name.value));
    }
}

/// Where plant data may be read.
enum Plants<'a> {
    /// A step: anywhere.
    Any,
    /// A script box: nowhere.
    Script,
    /// A `let`: nowhere.
    Let,
    /// A function: the tables of its `reads`.
    Reads { function: &'a str, tables: &'a [Spanned<String>] },
}

/// Settle every name of an expression: a variable, a row's column, or plant data checked
/// against where it is read; every built-in call against its arity. `functions` are the names a
/// call in an expression may wrongly reach for.
fn resolve_expr(e: &mut Expr, vars: &BTreeSet<String>, plants: &Plants, functions: &[&str], push: &mut dyn FnMut(Span, String)) {
    e.walk_mut(&mut |e: &mut Expr| {
        let span = e.span;
        match &mut e.kind {
            ExprKind::Name(n) => {
                if !vars.contains(n.as_str()) {
                    match plants {
                        Plants::Let => push(span, format!("a `let` is made of literals and the constants above it: `{n}`")),
                        _ => push(span, format!("`{n}` is read but nothing assigns it")),
                    }
                }
            }
            ExprKind::Field { base, field } if !vars.contains(&base.value) => {
                let (table, column) = (base.clone(), field.clone());
                e.kind = ExprKind::Plant { table, column };
                plant(e, plants, push);
            }
            ExprKind::Plant { .. } => plant(e, plants, push),
            ExprKind::Call { name, .. } if matches!(plants, Plants::Let) => {
                push(span, format!("a `let` is made of literals and the constants above it: `{}(...)`", name.value));
            }
            ExprKind::Call { name, args } => match BUILTINS.iter().find(|(b, _)| *b == name.value) {
                Some((b, arity)) => {
                    let n = args.len();
                    let bad = match arity {
                        Some((lo, hi)) => n < *lo || n > *hi,
                        None => n == 0,
                    };
                    if bad {
                        let takes = match arity {
                            Some((0, 0)) => "no value".to_string(),
                            Some((lo, hi)) if lo == hi => format!("{lo} value{}", if *lo == 1 { "" } else { "s" }),
                            Some((lo, hi)) => format!("{lo} or {hi} values"),
                            None => "one value or more".to_string(),
                        };
                        push(span, format!("`{b}` takes {takes}, not {n}"));
                    }
                }
                None if functions.contains(&name.value.as_str()) => push(
                    name.span,
                    format!("`{0}` is a function: a step calls it with `call`, a function by `<outputs> = {0}(...)`", name.value),
                ),
                None => push(
                    name.span,
                    format!(
                        "`{}` is no built-in call ({})",
                        name.value,
                        BUILTINS.iter().map(|(b, _)| *b).collect::<Vec<_>>().join(", ")
                    ),
                ),
            },
            _ => {}
        }
    });
}

fn plant(e: &Expr, plants: &Plants, push: &mut dyn FnMut(Span, String)) {
    let ExprKind::Plant { table, column } = &e.kind else { return };
    match plants {
        Plants::Any => {}
        Plants::Script => push(e.span, format!("a script box reads no plant data: `{}.{}`", table.value, column.value)),
        Plants::Let => push(e.span, format!("a `let` is made of literals and the constants above it: `{}.{}`", table.value, column.value)),
        Plants::Reads { function, tables } => {
            if !tables.iter().any(|t| t.value == table.value) {
                push(table.span, format!("`{}` is not among `{function}`'s `reads`", table.value));
            }
        }
    }
}

/// A call's inputs and outputs against the function it calls.
fn check_call(function: &Spanned<String>, args: &[Arg], outputs: &[Spanned<String>], f: &Function, push: &mut dyn FnMut(Span, String)) {
    let mut given = BTreeSet::new();
    for a in args {
        if !f.inputs.iter().any(|i| i.name.value == a.name.value) {
            push(a.name.span, format!("`{}` has no input `{}`", function.value, a.name.value));
        } else if !given.insert(a.name.value.clone()) {
            push(a.name.span, format!("input `{}` is given twice", a.name.value));
        }
    }
    for i in &f.inputs {
        if !given.contains(&i.name.value) && !args.iter().any(|a| a.name.value == i.name.value) {
            push(function.span, format!("`{}` needs its input `{}`", function.value, i.name.value));
        }
    }
    if outputs.len() > f.outputs.len() {
        push(
            outputs[f.outputs.len()].span,
            format!("`{}` gives {} output{}, not {}", function.value, f.outputs.len(), if f.outputs.len() == 1 { "" } else { "s" }, outputs.len()),
        );
    }
}

/// Where statements stand.
enum StmtCtx<'a> {
    Script { consts: &'a BTreeSet<String>, functions: &'a [&'a str] },
    Function { f: &'a Function, file_functions: &'a BTreeMap<String, Function> },
}

fn check_stmts(body: &mut [Stmt], vars: &BTreeSet<String>, ctx: &StmtCtx, push: &mut dyn FnMut(Span, String)) {
    let (plants, fnames): (Plants, Vec<&str>) = match ctx {
        StmtCtx::Script { functions, .. } => (Plants::Script, functions.to_vec()),
        StmtCtx::Function { f, file_functions } => {
            (Plants::Reads { function: &f.name.value, tables: &f.reads }, file_functions.keys().map(String::as_str).collect())
        }
    };
    for s in body {
        let span = s.span;
        match &mut s.kind {
            StmtKind::Assign { target, value, .. } => {
                match ctx {
                    StmtCtx::Script { consts, .. } => constant_assigned(target, consts, push),
                    StmtCtx::Function { f, .. } => {
                        if f.inputs.iter().any(|i| i.name.value == target.value) {
                            push(target.span, format!("`{}` is an input of `{}`, read only", target.value, f.name.value));
                        }
                    }
                }
                resolve_expr(value, vars, &plants, &fnames, push);
            }
            StmtKind::If { arms, otherwise } => {
                for (cond, b) in arms.iter_mut() {
                    resolve_expr(cond, vars, &plants, &fnames, push);
                    check_stmts(b, vars, ctx, push);
                }
                if let Some(b) = otherwise {
                    check_stmts(b, vars, ctx, push);
                }
            }
            StmtKind::For { var, from, to, body } => {
                if let StmtCtx::Function { f, .. } = ctx
                    && f.inputs.iter().any(|i| i.name.value == var.value)
                {
                    push(var.span, format!("`{}` is an input of `{}`, read only", var.value, f.name.value));
                }
                if let StmtCtx::Script { consts, .. } = ctx {
                    constant_assigned(var, consts, push);
                }
                resolve_expr(from, vars, &plants, &fnames, push);
                resolve_expr(to, vars, &plants, &fnames, push);
                check_stmts(body, vars, ctx, push);
            }
            StmtKind::Fail { reason } => {
                if let Some(r) = reason {
                    resolve_expr(r, vars, &plants, &fnames, push);
                }
            }
            StmtKind::Wait { condition, .. } => match ctx {
                StmtCtx::Script { .. } => push(span, "a script box does not wait: `wait`".to_string()),
                StmtCtx::Function { .. } => resolve_expr(condition, vars, &plants, &fnames, push),
            },
            StmtKind::Delay { .. } => {
                if let StmtCtx::Script { .. } = ctx {
                    push(span, "a script box does not wait: `wait`".to_string());
                }
            }
            StmtKind::Await { table, filter, bind, .. } => match ctx {
                StmtCtx::Script { .. } => push(span, "a script box does not wait: `await`".to_string()),
                StmtCtx::Function { f, .. } => {
                    if f.inputs.iter().any(|i| i.name.value == bind.value) {
                        push(bind.span, format!("`{}` is an input of `{}`, read only", bind.value, f.name.value));
                    }
                    if !f.reads.iter().any(|t| t.value == table.value) {
                        push(table.span, format!("`{}` is not among `{}`'s `reads`", table.value, f.name.value));
                    }
                    if let Some(c) = filter {
                        resolve_expr(c, vars, &plants, &fnames, push);
                    }
                }
            },
            StmtKind::Call { outputs, function, args } => match ctx {
                StmtCtx::Script { .. } => {
                    push(function.span, format!("a script box calls no function: `{}` is called by a step (`call`)", function.value));
                }
                StmtCtx::Function { f, file_functions } => {
                    for a in args.iter_mut() {
                        resolve_expr(&mut a.value, vars, &plants, &fnames, push);
                    }
                    for o in outputs.iter() {
                        if f.inputs.iter().any(|i| i.name.value == o.value) {
                            push(o.span, format!("`{}` is an input of `{}`, read only", o.value, f.name.value));
                        }
                    }
                    match file_functions.get(&function.value) {
                        None => push(function.span, format!("`{}` is no function of this file", function.value)),
                        Some(g) => check_call(function, args, outputs, g, push),
                    }
                }
            },
        }
    }
}

/// Every function of a file checked against the file: its ports, its names, its reads, its
/// calls, and no call back to itself.
fn check_function_file(ff: &mut FunctionFile, file: &str, text: &str, out: &mut Vec<Problem>) {
    let mut by_name: BTreeMap<String, Function> = BTreeMap::new();
    for f in &ff.functions {
        if by_name.contains_key(&f.name.value) {
            let first = &by_name[&f.name.value];
            let p = Place::of(text, first.name.span.start);
            out.push(Problem::at(
                file,
                text,
                f.name.span.start,
                format!("two functions are named `{}`: the other is at {file}:{}:{}", f.name.value, p.line, p.column),
            ));
        } else {
            by_name.insert(f.name.value.clone(), f.clone());
        }
    }
    for f in &mut ff.functions {
        let mut push = |span: Span, msg: String| out.push(Problem::at(file, text, span.start, msg));
        let mut ports = BTreeSet::new();
        for p in f.inputs.iter().chain(f.outputs.iter()) {
            if !ports.insert(p.name.value.clone()) {
                push(p.name.span, format!("`{}` is named twice in `{}`'s ports", p.name.value, f.name.value));
            }
        }
        let mut scope: BTreeSet<String> = ports.clone();
        assigned(&f.body, &mut scope);
        let mut set = BTreeSet::new();
        assigned(&f.body, &mut set);
        for o in &f.outputs {
            if !set.contains(&o.name.value) {
                push(o.name.span, format!("`{}` is an output of `{}` that nothing assigns", o.name.value, f.name.value));
            }
        }
        let snapshot = f.clone();
        let ctx = StmtCtx::Function { f: &snapshot, file_functions: &by_name };
        check_stmts(&mut f.body, &scope, &ctx, &mut push);
    }
    // No function calls itself, directly or through others.
    let calls: BTreeMap<String, BTreeSet<String>> = ff
        .functions
        .iter()
        .map(|f| {
            let mut c = BTreeSet::new();
            calls_in(&f.body, &mut c);
            (f.name.value.clone(), c)
        })
        .collect();
    for f in &ff.functions {
        if let Some(path) = cycle(&f.name.value, &calls) {
            let msg = if path.is_empty() {
                format!("`{}` calls itself", f.name.value)
            } else {
                format!("`{}` calls itself through {}", f.name.value, path.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", "))
            };
            out.push(Problem::at(file, text, f.name.span.start, msg));
        }
    }
}

fn calls_in(body: &[Stmt], into: &mut BTreeSet<String>) {
    for s in body {
        match &s.kind {
            StmtKind::Call { function, .. } => {
                into.insert(function.value.clone());
            }
            StmtKind::If { arms, otherwise } => {
                for (_, b) in arms {
                    calls_in(b, into);
                }
                if let Some(b) = otherwise {
                    calls_in(b, into);
                }
            }
            StmtKind::For { body, .. } => calls_in(body, into),
            _ => {}
        }
    }
}

/// The functions between `start` and itself on the shortest way back, when there is one.
fn cycle(start: &str, calls: &BTreeMap<String, BTreeSet<String>>) -> Option<Vec<String>> {
    let mut queue: std::collections::VecDeque<(String, Vec<String>)> = std::collections::VecDeque::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for c in calls.get(start).into_iter().flatten() {
        queue.push_back((c.clone(), Vec::new()));
    }
    while let Some((at, path)) = queue.pop_front() {
        if at == start {
            return Some(path);
        }
        if !seen.insert(at.clone()) {
            continue;
        }
        for c in calls.get(&at).into_iter().flatten() {
            let mut p = path.clone();
            p.push(at.clone());
            queue.push_back((c.clone(), p));
        }
    }
    None
}
