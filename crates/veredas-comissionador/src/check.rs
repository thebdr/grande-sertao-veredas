//! The whole check of a test and the function files it uses (`docs/test-schema.md`, "What a
//! check refuses"), before any run: every problem named with its file, line and column.
//!
//! The check also settles what each `<name>.<column>` is: a row's column when the name is a
//! variable, plant data otherwise. The model it returns is the one the engine runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{Lines, Problem};
use crate::model::*;
use crate::parse::{parse_functions, parse_test};

/// The longest `id` a test may have: its runs are kept in a folder of its name (`runs/<id>/`),
/// which a file system bounds, beside the run's own folders and files.
pub const MAX_ID: usize = 64;

/// A test that passed its check, with the function files it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub test: TestFile,
    /// The test file's name, as problems name it.
    pub file: String,
    /// The test file's text: what a run quotes of it.
    pub text: String,
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
    let lines = Lines::new(text);
    let at = |span: Span, msg: String| lines.problem(file, span.start, msg);
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
                } else if id.value.len() > MAX_ID {
                    // Its runs are kept in a folder of its name, which a file system bounds.
                    out.push(at(id.span, format!("`id` is {} characters long: {MAX_ID} at most", id.value.len())));
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
            SettingKind::Sign(roles) => {
                let mut named = BTreeSet::new();
                for r in roles.iter() {
                    if !named.insert(r.value.as_str()) {
                        out.push(at(r.span, format!("`{}` is named twice in `sign`", r.value)));
                    }
                }
            }
            SettingKind::Let { name, value } => {
                resolve_expr(value, &lets, &Plants::Let, &BTreeSet::new(), &mut |span, msg| out.push(at(span, msg)));
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
    let mut read_paths: BTreeSet<String> = BTreeSet::new();
    for u in test.uses() {
        if !read_paths.insert(u.value.clone()) {
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
        let flines = Lines::new(&f.text);
        for func in &f.functions.functions {
            let place = flines.place(func.name.span.start);
            match named.get(&func.name.value) {
                Some((other, p)) if other != &f.file => out.push(flines.problem(
                    &f.file,
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
    let fnames: BTreeSet<&str> = functions.keys().map(String::as_str).collect();
    let inputs_of: BTreeMap<&str, Inputs> = functions.iter().map(|(n, f)| (n.as_str(), Inputs::of(f))).collect();

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
            let mut push = |span: Span, msg: String| out.push(lines.problem(file, span.start, msg));
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
                            check_call(function, args, outputs, f, &inputs_of[function.value.as_str()], &mut push);
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

    if out.is_empty() { Ok(Checked { test, file: file.to_string(), text: text.to_string(), files, counts }) } else { Err(out) }
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
    Reads { function: &'a str, tables: &'a BTreeSet<&'a str> },
}

/// Settle every name of an expression: a variable, a row's column, or plant data checked
/// against where it is read; every built-in call against its arity. `functions` are the names a
/// call in an expression may wrongly reach for.
fn resolve_expr(e: &mut Expr, vars: &BTreeSet<String>, plants: &Plants, functions: &BTreeSet<&str>, push: &mut dyn FnMut(Span, String)) {
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
                None if functions.contains(name.value.as_str()) => push(
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
            if !tables.contains(table.value.as_str()) {
                push(table.span, format!("`{}` is not among `{function}`'s `reads`", table.value));
            }
        }
    }
}

/// A function's inputs as its calls are checked against them, made once per function: their names
/// as a set, and each once in the order the function declares them.
struct Inputs<'a> {
    set: BTreeSet<&'a str>,
    order: Vec<&'a str>,
}

impl<'a> Inputs<'a> {
    fn of(f: &'a Function) -> Inputs<'a> {
        let mut set = BTreeSet::new();
        let order = f.inputs.iter().map(|i| i.name.value.as_str()).filter(|n| set.insert(*n)).collect();
        Inputs { set, order }
    }
}

/// A call's inputs and outputs against the function it calls, in a time its own arguments bound:
/// the inputs it leaves out are one problem, named by the first five and counted.
fn check_call(function: &Spanned<String>, args: &[Arg], outputs: &[Spanned<String>], f: &Function, inputs: &Inputs, push: &mut dyn FnMut(Span, String)) {
    let mut given: BTreeSet<&str> = BTreeSet::new();
    for a in args {
        if !inputs.set.contains(a.name.value.as_str()) {
            push(a.name.span, format!("`{}` has no input `{}`", function.value, a.name.value));
        } else if !given.insert(a.name.value.as_str()) {
            push(a.name.span, format!("input `{}` is given twice", a.name.value));
        }
    }
    // Every input given is one of the function's, so the rest are left out; the first five are
    // found past the given ones alone.
    let missing = inputs.order.len() - given.len();
    if missing > 0 {
        let shown: Vec<String> = inputs.order.iter().filter(|i| !given.contains(*i)).take(5).map(|i| format!("`{i}`")).collect();
        let msg = match missing {
            1 => format!("`{}` needs its input {}", function.value, shown[0]),
            2..=5 => format!("`{}` needs its inputs {}", function.value, shown.join(", ")),
            6 => format!("`{}` needs its inputs {} and 1 other", function.value, shown.join(", ")),
            n => format!("`{}` needs its inputs {} and {} others", function.value, shown.join(", "), n - 5),
        };
        push(function.span, msg);
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
    Script { consts: &'a BTreeSet<String>, functions: &'a BTreeSet<&'a str> },
    /// A function, its inputs and the tables of its `reads` as sets, each looked up at once.
    Function {
        f: &'a Function,
        inputs: &'a BTreeSet<&'a str>,
        reads: &'a BTreeSet<&'a str>,
        file_functions: &'a BTreeMap<String, Function>,
        /// The inputs of each of the file's functions, made once for the file.
        file_inputs: &'a BTreeMap<&'a str, Inputs<'a>>,
        names: &'a BTreeSet<&'a str>,
    },
}

fn check_stmts(body: &mut [Stmt], vars: &BTreeSet<String>, ctx: &StmtCtx, push: &mut dyn FnMut(Span, String)) {
    let (plants, fnames): (Plants, &BTreeSet<&str>) = match ctx {
        StmtCtx::Script { functions, .. } => (Plants::Script, functions),
        StmtCtx::Function { f, reads, names, .. } => (Plants::Reads { function: &f.name.value, tables: reads }, names),
    };
    for s in body {
        let span = s.span;
        match &mut s.kind {
            StmtKind::Assign { target, value, .. } => {
                match ctx {
                    StmtCtx::Script { consts, .. } => constant_assigned(target, consts, push),
                    StmtCtx::Function { f, inputs, .. } => {
                        if inputs.contains(target.value.as_str()) {
                            push(target.span, format!("`{}` is an input of `{}`, read only", target.value, f.name.value));
                        }
                    }
                }
                resolve_expr(value, vars, &plants, fnames, push);
            }
            StmtKind::If { arms, otherwise } => {
                for (cond, b) in arms.iter_mut() {
                    resolve_expr(cond, vars, &plants, fnames, push);
                    check_stmts(b, vars, ctx, push);
                }
                if let Some(b) = otherwise {
                    check_stmts(b, vars, ctx, push);
                }
            }
            StmtKind::For { var, from, to, body } => {
                if let StmtCtx::Function { f, inputs, .. } = ctx
                    && inputs.contains(var.value.as_str())
                {
                    push(var.span, format!("`{}` is an input of `{}`, read only", var.value, f.name.value));
                }
                if let StmtCtx::Script { consts, .. } = ctx {
                    constant_assigned(var, consts, push);
                }
                resolve_expr(from, vars, &plants, fnames, push);
                resolve_expr(to, vars, &plants, fnames, push);
                check_stmts(body, vars, ctx, push);
            }
            StmtKind::Fail { reason } => {
                if let Some(r) = reason {
                    resolve_expr(r, vars, &plants, fnames, push);
                }
            }
            StmtKind::Wait { condition, .. } => match ctx {
                StmtCtx::Script { .. } => push(span, "a script box does not wait: `wait`".to_string()),
                StmtCtx::Function { .. } => resolve_expr(condition, vars, &plants, fnames, push),
            },
            StmtKind::Delay { .. } => {
                if let StmtCtx::Script { .. } = ctx {
                    push(span, "a script box does not wait: `wait`".to_string());
                }
            }
            StmtKind::Await { table, filter, bind, .. } => match ctx {
                StmtCtx::Script { .. } => push(span, "a script box does not wait: `await`".to_string()),
                StmtCtx::Function { f, inputs, reads, .. } => {
                    if inputs.contains(bind.value.as_str()) {
                        push(bind.span, format!("`{}` is an input of `{}`, read only", bind.value, f.name.value));
                    }
                    if !reads.contains(table.value.as_str()) {
                        push(table.span, format!("`{}` is not among `{}`'s `reads`", table.value, f.name.value));
                    }
                    if let Some(c) = filter {
                        resolve_expr(c, vars, &plants, fnames, push);
                    }
                }
            },
            StmtKind::Call { outputs, function, args } => match ctx {
                StmtCtx::Script { .. } => {
                    push(function.span, format!("a script box calls no function: `{}` is called by a step (`call`)", function.value));
                }
                StmtCtx::Function { f, inputs, file_functions, file_inputs, .. } => {
                    for a in args.iter_mut() {
                        resolve_expr(&mut a.value, vars, &plants, fnames, push);
                    }
                    for o in outputs.iter() {
                        if inputs.contains(o.value.as_str()) {
                            push(o.span, format!("`{}` is an input of `{}`, read only", o.value, f.name.value));
                        }
                    }
                    match file_functions.get(&function.value) {
                        None => push(function.span, format!("`{}` is no function of this file", function.value)),
                        Some(g) => check_call(function, args, outputs, g, &file_inputs[function.value.as_str()], push),
                    }
                }
            },
        }
    }
}

/// Every function of a file checked against the file: its ports, its names, its reads, its
/// calls, and no call back to itself.
fn check_function_file(ff: &mut FunctionFile, file: &str, text: &str, out: &mut Vec<Problem>) {
    let lines = Lines::new(text);
    let mut by_name: BTreeMap<String, Function> = BTreeMap::new();
    for f in &ff.functions {
        if by_name.contains_key(&f.name.value) {
            let first = &by_name[&f.name.value];
            let p = lines.place(first.name.span.start);
            out.push(lines.problem(
                file,
                f.name.span.start,
                format!("two functions are named `{}`: the other is at {file}:{}:{}", f.name.value, p.line, p.column),
            ));
        } else {
            by_name.insert(f.name.value.clone(), f.clone());
        }
    }
    let file_names: BTreeSet<&str> = by_name.keys().map(String::as_str).collect();
    let file_inputs: BTreeMap<&str, Inputs> = by_name.iter().map(|(n, g)| (n.as_str(), Inputs::of(g))).collect();
    for f in &mut ff.functions {
        let mut push = |span: Span, msg: String| out.push(lines.problem(file, span.start, msg));
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
        let inputs: BTreeSet<&str> = snapshot.inputs.iter().map(|i| i.name.value.as_str()).collect();
        let reads: BTreeSet<&str> = snapshot.reads.iter().map(|t| t.value.as_str()).collect();
        let ctx = StmtCtx::Function { f: &snapshot, inputs: &inputs, reads: &reads, file_functions: &by_name, file_inputs: &file_inputs, names: &file_names };
        check_stmts(&mut f.body, &scope, &ctx, &mut push);
    }
    // No function calls itself, directly or through others: the cycles are the strongly
    // connected components of the call graph (one pass), each function in one named with the way
    // back to itself (a search inside its component, parent links, no path copied).
    let names: Vec<String> = ff.functions.iter().map(|f| f.name.value.clone()).collect();
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, n) in names.iter().enumerate() {
        index.entry(n.as_str()).or_insert(i);
    }
    let edges: Vec<Vec<usize>> = ff
        .functions
        .iter()
        .map(|f| {
            let mut c = BTreeSet::new();
            calls_in(&f.body, &mut c);
            c.iter().filter_map(|n| index.get(n.as_str()).copied()).collect()
        })
        .collect();
    let component = components(&edges);
    let (mut parent, mut seen) = (vec![0; edges.len()], vec![0; edges.len()]);
    for (i, f) in ff.functions.iter().enumerate() {
        if let Some(path) = way_back(i, &edges, &component, &mut parent, &mut seen) {
            // A long way back is named by its first five and counted.
            let shown: Vec<String> = path.iter().take(5).map(|&n| format!("`{}`", names[n])).collect();
            let msg = match path.len() {
                0 => format!("`{}` calls itself", f.name.value),
                1..=5 => format!("`{}` calls itself through {}", f.name.value, shown.join(", ")),
                6 => format!("`{}` calls itself through {} and 1 other", f.name.value, shown.join(", ")),
                n => format!("`{}` calls itself through {} and {} others", f.name.value, shown.join(", "), n - 5),
            };
            out.push(lines.problem(file, f.name.span.start, msg));
        }
    }
}

/// Every node's strongly connected component, by Tarjan's method without recursion.
fn components(edges: &[Vec<usize>]) -> Vec<usize> {
    let n = edges.len();
    let (mut index, mut low, mut comp) = (vec![usize::MAX; n], vec![0; n], vec![usize::MAX; n]);
    let (mut on, mut stack, mut next, mut count) = (vec![false; n], Vec::new(), 0, 0);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&mut (v, ref mut e)) = work.last_mut() {
            if *e < edges[v].len() {
                let w = edges[v][*e];
                *e += 1;
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = count;
                        if w == v {
                            break;
                        }
                    }
                    count += 1;
                }
            }
        }
    }
    comp
}

/// The functions between `start` and itself on the shortest way back, searched inside its
/// component alone; none when it is on no cycle. `parent` and `seen` are the search's arrays,
/// reused from one start to the next (`seen[v] == start + 1` marks a node met in this search).
fn way_back(start: usize, edges: &[Vec<usize>], comp: &[usize], parent: &mut [usize], seen: &mut [usize]) -> Option<Vec<usize>> {
    if edges[start].contains(&start) {
        return Some(Vec::new());
    }
    let mark = start + 1;
    let mut queue = std::collections::VecDeque::new();
    for &w in &edges[start] {
        if comp[w] == comp[start] && seen[w] != mark {
            seen[w] = mark;
            parent[w] = start;
            queue.push_back(w);
        }
    }
    while let Some(v) = queue.pop_front() {
        if edges[v].contains(&start) {
            let mut path = vec![v];
            let mut at = v;
            while parent[at] != start {
                at = parent[at];
                path.push(at);
            }
            path.reverse();
            return Some(path);
        }
        for &w in &edges[v] {
            if w != start && comp[w] == comp[start] && seen[w] != mark {
                seen[w] = mark;
                parent[w] = v;
                queue.push_back(w);
            }
        }
    }
    None
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

