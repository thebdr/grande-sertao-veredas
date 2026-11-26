//! The interpreter (`docs/test-schema.md`, "Steps", "Time"; `docs/comissionador-plan.md`, "How a
//! run meets the plant"): a checked test bound to a plant and run step by step on a clock that is
//! the store's axis, live against the plant or replayed over a stored window, each step ending in
//! an outcome that routes the run, the run ending passed, failed or aborted.
//!
//! Live and replay are one engine: every step is judged on the rows' own stamps; the modes differ
//! in where the rows come from (arriving, or stored) and in what a write does (sent, or noted).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::check::Checked;
use crate::eval::{self, Env, EvalError, Row, Value, compare};
use crate::model::*;
use crate::plant::{Plant, PlantError, Table};

/// A live run reads a moment only once it is this far past: the session's pump flushes every
/// 50 ms, so the rows stamped at or before it have reached the store.
pub const SETTLE_MS: i64 = 250;
/// How often a live run looks at the store while it waits.
pub const POLL: Duration = Duration::from_millis(50);
/// The steps a run takes at most: a test that routes round without end is aborted, not hung.
pub const MAX_STEPS: u64 = 100_000;
/// The statements a script box or a step's call runs at most, a turn of a `for` counted as one.
pub const MAX_STATEMENTS: u64 = 10_000_000;
/// How many statements run between two looks at the run's bound.
const BOUND_EVERY: u64 = 1024;

/// How a run reads the plant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    /// Against the plant as it runs, writes sent.
    Live,
    /// Over the stored window `[from_ms, to_ms]` of the store's axis, writes not sent.
    Replay { from_ms: i64, to_ms: i64 },
}

impl RunMode {
    pub fn mode(&self) -> Mode {
        match self {
            RunMode::Live => Mode::Live,
            RunMode::Replay { .. } => Mode::Replay,
        }
    }
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunVerdict {
    Passed,
    Failed,
    Aborted,
}

impl std::fmt::Display for RunVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            RunVerdict::Passed => "passed",
            RunVerdict::Failed => "failed",
            RunVerdict::Aborted => "aborted",
        })
    }
}

/// One step as it ran.
#[derive(Clone, Debug, PartialEq)]
pub struct StepRecord {
    pub step: String,
    /// From 1: the how-many-th time the run reached this step.
    pub attempt: u32,
    /// Its outcome; none when the run stopped inside it (the tester's stop, a replay's window
    /// ending, the run's bound).
    pub outcome: Option<Outcome>,
    /// What happened, in words: each action's.
    pub detail: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// A criterion as it was judged: a `check`, or a `branch`'s decision.
#[derive(Clone, Debug, PartialEq)]
pub struct Criterion {
    pub step: String,
    pub attempt: u32,
    /// Its label, or its own text.
    pub text: String,
    /// The values it read, `name = value`.
    pub measured: Vec<String>,
    /// `pass`, `fail`, or the arm a branch took.
    pub result: String,
}

/// A value or a note a `report` recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Reported {
    pub step: String,
    pub attempt: u32,
    /// `name = value`, the note's text, or what could not be evaluated and why.
    pub text: String,
    pub note: bool,
}

/// A gated write or send, as made.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteRecord {
    pub step: String,
    pub attempt: u32,
    /// `table.column := value` or `channel.message(field: value, ...)`.
    pub text: String,
    /// Whether it went to the plant (a replay sends nothing).
    pub sent: bool,
    /// The plant's answer, or portal's refusal, in their words.
    pub answer: String,
    pub ok: bool,
}

/// What a run did.
#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub verdict: RunVerdict,
    /// The step that decided it, when one did.
    pub decided_at: Option<String>,
    /// Why, in words.
    pub reason: String,
    /// The window on the store's axis the run covered: its start and its clock at the end.
    pub start_ms: i64,
    pub end_ms: i64,
    pub steps: Vec<StepRecord>,
    pub criteria: Vec<Criterion>,
    pub reported: Vec<Reported>,
    pub writes: Vec<WriteRecord>,
    /// The tables it read.
    pub tables: BTreeSet<String>,
}

/// What a run tells its listener as it goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    StepStarted { step: String, attempt: u32, at_ms: i64 },
    StepEnded(StepRecord),
    Criterion(Criterion),
    Reported(Reported),
    Write(WriteRecord),
    Ended { verdict: RunVerdict, decided_at: Option<String>, reason: String },
}

/// Why a run did not start. Nothing was written and nothing ran.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the run is refused: {}", .0.join("; "))]
pub struct Refused(pub Vec<String>);

/// A run's settings beside its mode.
#[derive(Clone, Debug)]
pub struct RunOptions {
    pub mode: RunMode,
    /// Set by the tester to stop the run: it ends aborted at the next look.
    pub stop: Arc<AtomicBool>,
    /// The fingerprint of the gated writes the person confirmed ([`writes_fingerprint`]): a live
    /// run whose test writes arms portal's switch for itself when it is this test's (Q314 (b)).
    pub confirmed: Option<String>,
}

impl RunOptions {
    pub fn live() -> RunOptions {
        RunOptions { mode: RunMode::Live, stop: Arc::new(AtomicBool::new(false)), confirmed: None }
    }
    pub fn replay(from_ms: i64, to_ms: i64) -> RunOptions {
        RunOptions { mode: RunMode::Replay { from_ms, to_ms }, stop: Arc::new(AtomicBool::new(false)), confirmed: None }
    }
    /// The same, the person having confirmed the gated writes of this fingerprint.
    pub fn confirming(mut self, fingerprint: impl Into<String>) -> RunOptions {
        self.confirmed = Some(fingerprint.into());
        self
    }
}

/// A test's gated actions (its writes and sends), each as the test writes it, its blanks and line
/// ends one space, in the order of its steps: the list a person confirms before a run arms
/// portal's switch for it.
pub fn gated_writes(c: &Checked) -> Vec<String> {
    c.test.steps.iter().flat_map(|s| s.actions.iter()).filter(|a| a.is_gated()).map(|a| squash(c.text.get(a.span.start..a.span.end).unwrap_or(""))).collect()
}

/// The fingerprint a person confirms a test's gated writes by: the first 128 bits of SHA-256, in
/// 32 hex digits, over the list ([`gated_writes`]), the test's whole text and every function file
/// it uses, each ended by a zero byte, so an edit that could change what a write writes asks again
/// and no pair of tests made to share one can be found.
pub fn writes_fingerprint(c: &Checked) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    let mut eat = |text: &str| {
        h.update(text.as_bytes());
        h.update([0u8]);
    };
    for w in gated_writes(c) {
        eat(&w);
    }
    eat(&c.text);
    for f in &c.files {
        eat(&f.path);
        eat(&f.text);
    }
    h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

// ---- binding -----------------------------------------------------------------------------------

/// Every plant reference of an expression.
fn plant_refs(e: &Expr, out: &mut Vec<(String, String, Span)>) {
    e.walk(&mut |x: &Expr| {
        if let ExprKind::Plant { table, column } = &x.kind {
            out.push((table.value.clone(), column.value.clone(), x.span));
        }
    });
}

fn stmt_refs(body: &[Stmt], refs: &mut Vec<(String, String, Span)>, tables: &mut Vec<String>) {
    for s in body {
        match &s.kind {
            StmtKind::Assign { value, .. } => plant_refs(value, refs),
            StmtKind::If { arms, otherwise } => {
                for (c, b) in arms {
                    plant_refs(c, refs);
                    stmt_refs(b, refs, tables);
                }
                if let Some(b) = otherwise {
                    stmt_refs(b, refs, tables);
                }
            }
            StmtKind::For { from, to, body, .. } => {
                plant_refs(from, refs);
                plant_refs(to, refs);
                stmt_refs(body, refs, tables);
            }
            StmtKind::Fail { reason } => reason.iter().for_each(|r| plant_refs(r, refs)),
            StmtKind::Wait { condition, .. } => plant_refs(condition, refs),
            StmtKind::Delay { .. } => {}
            StmtKind::Await { table, filter, .. } => {
                tables.push(table.value.clone());
                filter.iter().for_each(|f| plant_refs(f, refs));
            }
            StmtKind::Call { args, .. } => args.iter().for_each(|a| plant_refs(&a.value, refs)),
        }
    }
}

/// Every expression of a block of statements, its blocks' included.
fn stmt_exprs<'a>(body: &'a [Stmt], f: &mut dyn FnMut(&'a Expr)) {
    for s in body {
        match &s.kind {
            StmtKind::Assign { value, .. } => f(value),
            StmtKind::If { arms, otherwise } => {
                for (c, b) in arms {
                    f(c);
                    stmt_exprs(b, f);
                }
                if let Some(b) = otherwise {
                    stmt_exprs(b, f);
                }
            }
            StmtKind::For { from, to, body, .. } => {
                f(from);
                f(to);
                stmt_exprs(body, f);
            }
            StmtKind::Fail { reason } => reason.iter().for_each(&mut *f),
            StmtKind::Wait { condition, .. } => f(condition),
            StmtKind::Delay { .. } => {}
            StmtKind::Await { filter, .. } => filter.iter().for_each(&mut *f),
            StmtKind::Call { args, .. } => args.iter().for_each(|a| f(&a.value)),
        }
    }
}

/// Every expression of a step's action, a script box's statements included.
fn action_exprs<'a>(a: &'a Action, f: &mut dyn FnMut(&'a Expr)) {
    match &a.kind {
        ActionKind::Write { value, .. } => f(value),
        ActionKind::Send { fields, .. } => fields.iter().for_each(|x| f(&x.value)),
        ActionKind::Wait { condition, .. } | ActionKind::Check { condition, .. } => f(condition),
        ActionKind::Await { filter, .. } => filter.iter().for_each(&mut *f),
        ActionKind::Call { args, .. } => args.iter().for_each(|x| f(&x.value)),
        ActionKind::Branch { value, arms, .. } => {
            f(value);
            for arm in arms {
                for p in &arm.patterns {
                    match p {
                        Pattern::Compare { value, .. } | Pattern::Value(value) => f(value),
                        Pattern::Range { low, high } => {
                            f(low);
                            f(high);
                        }
                    }
                }
            }
        }
        ActionKind::Report { items } => items.iter().for_each(&mut *f),
        ActionKind::Stop { reason, .. } => reason.iter().for_each(&mut *f),
        ActionKind::Script { body } => stmt_exprs(body, f),
        ActionKind::Read { .. } | ActionKind::Delay { .. } => {}
    }
}

/// The rows a block binds: every `await`'s name with its table, and every bare name assigned to a
/// name (`r = o`), to be followed by [`follow_copies`].
fn stmt_rows(body: &[Stmt], rows: &mut HashMap<String, BTreeSet<String>>, copies: &mut Vec<(String, String)>) {
    for s in body {
        match &s.kind {
            StmtKind::Await { bind, table, .. } => {
                rows.entry(bind.value.clone()).or_default().insert(table.value.clone());
            }
            StmtKind::Assign { target, value, .. } => {
                let mut from = Vec::new();
                row_sources(value, &mut from);
                copies.extend(from.into_iter().map(|n| (target.value.clone(), n)));
            }
            StmtKind::If { arms, otherwise } => {
                arms.iter().for_each(|(_, b)| stmt_rows(b, rows, copies));
                if let Some(b) = otherwise {
                    stmt_rows(b, rows, copies);
                }
            }
            StmtKind::For { body, .. } => stmt_rows(body, rows, copies),
            _ => {}
        }
    }
}

/// The names an expression's value may be one of, unchanged: a bare name, and the values `min`
/// and `max` choose among.
fn row_sources(e: &Expr, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Name(n) => out.push(n.clone()),
        ExprKind::Call { name, args } if name.value == "min" || name.value == "max" => args.iter().for_each(|a| row_sources(a, out)),
        _ => {}
    }
}

/// Every table a name may hold a row of: an `await`'s, and through every copy, in whatever order
/// the copies stand (a step routed back, a loop), until nothing changes.
fn follow_copies(rows: &mut HashMap<String, BTreeSet<String>>, copies: &[(String, String)]) {
    loop {
        let mut grew = false;
        for (target, source) in copies {
            let Some(from) = rows.get(source).cloned() else { continue };
            let into = rows.entry(target.clone()).or_default();
            for t in from {
                grew |= into.insert(t);
            }
        }
        if !grew {
            return;
        }
    }
}

/// The fields every row has, whatever its table.
const ROW_FIELDS: [&str; 5] = ["time", "arrival", "source", "key", "seq"];

/// Every column read through a row an `await` binds (`a.x`), against the tables the name may hold
/// a row of: `(table, column, base)` for each of them when none has that column. A name rebound to
/// rows of tables of other columns is judged where it runs: which await reached a read is the
/// routes' to say.
fn row_columns(exprs: &[&Expr], rows: &HashMap<String, BTreeSet<String>>, tables: &HashMap<String, Table>, out: &mut Vec<(String, String, String)>) {
    for e in exprs {
        e.walk(&mut |x: &Expr| {
            if let ExprKind::Field { base, field } = &x.kind
                && let Some(ts) = rows.get(&base.value)
                && !ROW_FIELDS.contains(&field.value.as_str())
            {
                let known: Vec<(&String, &Table)> = ts.iter().filter_map(|t| tables.get(t).map(|table| (t, table))).collect();
                if !known.is_empty() && known.iter().all(|(_, table)| !table.columns.iter().any(|c| c.name == field.value)) {
                    for (t, _) in known {
                        out.push((t.clone(), field.value.clone(), base.value.clone()));
                    }
                }
            }
        });
    }
}

/// Check a test against the plant before its first step: everything the reference's binding
/// paragraph lists, every problem named.
/// A test bound to a plant: the session's tables by name, and the ones the test reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    pub tables: HashMap<String, Table>,
    pub read: BTreeSet<String>,
    /// Whether the run arms portal's switch for itself, and disarms it at its end.
    pub arm: bool,
}

pub fn bind(c: &Checked, plant: &dyn Plant, mode: &RunMode, confirmed: Option<&str>) -> Result<Bound, Refused> {
    let mut problems = Vec::new();
    let t = &c.test;
    if let Some(p) = t.portal()
        && p != plant.name()
    {
        problems.push(format!("the test is written for the portal session `{p}`, and this one is `{}`", plant.name()));
    }
    if !t.modes().contains(&mode.mode()) {
        problems.push(format!("the test runs {}, not {}", t.modes().iter().map(|m| m.to_string()).collect::<Vec<_>>().join(" and "), mode.mode()));
    }
    if let RunMode::Replay { from_ms, to_ms } = mode
        && to_ms < from_ms
    {
        problems.push("the window ends before it starts".to_string());
    }
    let tables: HashMap<String, Table> = match plant.tables() {
        Ok(ts) => ts.into_iter().map(|t| (t.name.clone(), t)).collect(),
        Err(e) => return Err(Refused(vec![e.to_string()])),
    };
    let mut refs: Vec<(String, String, Span)> = Vec::new();
    let mut await_tables: Vec<String> = Vec::new();
    let mut writes = 0;
    for step in &t.steps {
        for a in &step.actions {
            match &a.kind {
                ActionKind::Write { target, value } => {
                    writes += 1;
                    plant_refs(value, &mut refs);
                    if !plant.writable(&target.table.value, &target.column.value) {
                        problems.push(format!("`{}.{}` is not an OPC UA tag the session marks writable", target.table.value, target.column.value));
                    }
                }
                ActionKind::Send { channel, message, fields } => {
                    writes += 1;
                    fields.iter().for_each(|f| plant_refs(&f.value, &mut refs));
                    if !plant.sendable(&channel.value, &message.value) {
                        problems.push(format!("`{}.{}` is no channel and message of the session", channel.value, message.value));
                    }
                }
                ActionKind::Wait { condition, .. } | ActionKind::Check { condition, .. } => plant_refs(condition, &mut refs),
                ActionKind::Await { table, filter, .. } => {
                    await_tables.push(table.value.clone());
                    filter.iter().for_each(|f| plant_refs(f, &mut refs));
                }
                ActionKind::Read { refs: rs, .. } => {
                    for r in rs {
                        refs.push((r.table.value.clone(), r.column.value.clone(), r.span()));
                    }
                }
                ActionKind::Call { args, .. } => args.iter().for_each(|x| plant_refs(&x.value, &mut refs)),
                ActionKind::Branch { value, arms, .. } => {
                    plant_refs(value, &mut refs);
                    for arm in arms {
                        for p in &arm.patterns {
                            match p {
                                Pattern::Compare { value, .. } | Pattern::Value(value) => plant_refs(value, &mut refs),
                                Pattern::Range { low, high } => {
                                    plant_refs(low, &mut refs);
                                    plant_refs(high, &mut refs);
                                }
                            }
                        }
                    }
                }
                ActionKind::Report { items } => items.iter().for_each(|i| plant_refs(i, &mut refs)),
                ActionKind::Stop { reason, .. } => reason.iter().for_each(|r| plant_refs(r, &mut refs)),
                ActionKind::Script { .. } | ActionKind::Delay { .. } => {}
            }
        }
    }
    for f in c.files.iter().flat_map(|f| f.functions.functions.iter()) {
        stmt_refs(&f.body, &mut refs, &mut await_tables);
    }
    let mut seen = BTreeSet::new();
    let read: BTreeSet<String> =
        await_tables.iter().cloned().chain(refs.iter().map(|(t, _, _)| t.clone())).filter(|t| tables.contains_key(t)).collect();
    for name in await_tables {
        if !tables.contains_key(&name) && seen.insert(name.clone()) {
            problems.push(format!("`{name}` is no table of the session"));
        }
    }
    for (table, column, _) in refs {
        match tables.get(&table) {
            None => {
                if seen.insert(table.clone()) {
                    problems.push(format!("`{table}` is no table of the session"));
                }
            }
            Some(t) if !t.columns.iter().any(|c| c.name == column) => {
                if seen.insert(format!("{table}.{column}")) {
                    problems.push(format!("`{table}` has no column `{column}`"));
                }
            }
            Some(_) => {}
        }
    }
    // A column read through a row an `await` binds: the steps' rows across the test (a script box's
    // names are the test's), a function's within it.
    let mut lacking = Vec::new();
    let mut rows: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut copies: Vec<(String, String)> = Vec::new();
    for a in t.steps.iter().flat_map(|s| s.actions.iter()) {
        match &a.kind {
            ActionKind::Await { bind, table, .. } => {
                rows.entry(bind.value.clone()).or_default().insert(table.value.clone());
            }
            ActionKind::Script { body } => stmt_rows(body, &mut rows, &mut copies),
            _ => {}
        }
    }
    follow_copies(&mut rows, &copies);
    let mut exprs = Vec::new();
    for a in t.steps.iter().flat_map(|s| s.actions.iter()) {
        action_exprs(a, &mut |e| exprs.push(e));
    }
    row_columns(&exprs, &rows, &tables, &mut lacking);
    for f in c.files.iter().flat_map(|f| f.functions.functions.iter()) {
        let (mut rows, mut copies) = (HashMap::new(), Vec::new());
        stmt_rows(&f.body, &mut rows, &mut copies);
        follow_copies(&mut rows, &copies);
        let mut exprs = Vec::new();
        stmt_exprs(&f.body, &mut |e| exprs.push(e));
        row_columns(&exprs, &rows, &tables, &mut lacking);
    }
    for (table, column, base) in lacking {
        if seen.insert(format!("{table}.{column}")) {
            problems.push(format!("`{table}` has no column `{column}`, read as `{base}.{column}`"));
        }
    }
    // Live, a test that writes with the switch off: the run arms it for itself when the person
    // confirmed this test's gated writes and the embedder handed it the way (Q314 (b)).
    let mut arm = false;
    if mode == &RunMode::Live && writes > 0 && !plant.armed() {
        let fp = writes_fingerprint(c);
        let what = if writes == 1 { "its 1 gated write, confirmed".to_string() } else { format!("its {writes} gated writes, confirmed") };
        match confirmed {
            _ if !plant.can_arm() => problems.push("the test writes, and portal's write switch is off".to_string()),
            Some(given) if given == fp => arm = true,
            Some(given) => problems.push(format!("the test writes, and portal's write switch is off: `{given}` confirms no list of this test's; {what} as `{fp}`, arm it for the run")),
            None => problems.push(format!("the test writes, and portal's write switch is off: {what} as `{fp}`, arm it for the run")),
        }
    }
    if problems.is_empty() { Ok(Bound { tables, read, arm }) } else { Err(Refused(problems)) }
}

// ---- the run -------------------------------------------------------------------------------------

/// Why a run stops before a step decides it.
#[derive(Clone, Debug, PartialEq)]
enum Halt {
    Stopped,
    /// A replay's window ended, at this moment.
    WindowEnded(i64),
    /// The test's own bound passed.
    Bound,
    Store(String),
    Steps,
}

type R<T> = Result<T, Halt>;

type Scope = HashMap<String, Value>;

/// How an action ended.
enum Flow {
    Pass(String),
    Out(Outcome, String),
    /// A branch's arm, its route.
    Route(Route, String),
    Stop(Verdict, String),
}

/// How a statement block ended.
enum StmtFlow {
    Next,
    Out(Outcome, String),
}

/// What a wait met.
enum Waited {
    At(i64),
    Timeout,
    Fault(String),
}

/// Values for an expression at a moment: variables from a scope, plant data as stored at or
/// before the moment.
struct AtEnv<'a> {
    plant: &'a dyn Plant,
    tables: &'a HashMap<String, Table>,
    vars: &'a Scope,
    at: i64,
    /// Plant data a wait already holds, read before the store.
    cache: Option<&'a HashMap<(String, String), Option<Value>>>,
}

impl Env for AtEnv<'_> {
    fn var(&self, name: &str) -> Option<Value> {
        self.vars.get(name).cloned()
    }
    fn plant(&mut self, table: &str, column: &str) -> Result<Value, EvalError> {
        let missing = || EvalError::NoValue(format!("`{table}.{column}`"));
        if let Some(cache) = self.cache
            && let Some(v) = cache.get(&(table.to_string(), column.to_string()))
        {
            return v.clone().ok_or_else(missing);
        }
        let t = self.tables.get(table).ok_or_else(|| EvalError::Fault(format!("`{table}` is no table of the session")))?;
        match self.plant.latest(t, column, self.at) {
            Ok(Some((_, v))) => Ok(v),
            Ok(None) => Err(missing()),
            Err(e) => Err(EvalError::Fault(e.to_string())),
        }
    }
    fn now(&self) -> i64 {
        self.at
    }
}

fn has_plant(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |x: &Expr| found |= matches!(x.kind, ExprKind::Plant { .. }));
    found
}

/// The names and plant data an expression reads, each once, in order.
fn leaves(e: &Expr) -> Vec<(String, Expr)> {
    let mut out: Vec<(String, Expr)> = Vec::new();
    e.walk(&mut |x: &Expr| {
        let name = match &x.kind {
            ExprKind::Name(n) => n.clone(),
            ExprKind::Field { base, field } => format!("{}.{}", base.value, field.value),
            ExprKind::Plant { table, column } => format!("{}.{}", table.value, column.value),
            _ => return,
        };
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, x.clone()));
        }
    });
    out
}

/// A value as a gated write's text names it: a truth value as 1 or 0.
fn wire_text(v: &Value) -> String {
    match v {
        Value::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
        other => other.to_string(),
    }
}

/// A value of a port's type: a whole number taken for a number.
fn fits(v: Value, ty: Type) -> Option<Value> {
    Some(match (ty, v) {
        (Type::Int, v @ Value::Int(_)) => v,
        (Type::Float, Value::Int(i)) => Value::Float(i as f64),
        (Type::Float, v @ Value::Float(_)) => v,
        (Type::Bool, v @ Value::Bool(_)) => v,
        (Type::Text, v @ Value::Text(_)) => v,
        (Type::Time, v @ Value::Time(_)) => v,
        (Type::Duration, v @ Value::Duration(_)) => v,
        _ => return None,
    })
}

struct Runner<'a> {
    c: &'a Checked,
    plant: &'a dyn Plant,
    tables: HashMap<String, Table>,
    mode: RunMode,
    stop: Arc<AtomicBool>,
    clock: i64,
    horizon: i64,
    deadline: i64,
    vars: Scope,
    attempts: HashMap<String, u32>,
    retries: HashMap<(String, usize), u32>,
    statements: u64,
    /// The last row an `await` of each table took: its stamp and how many rows of that stamp, in
    /// their order, it passed and took, so the next `await` at that stamp takes the row after it
    /// (a view's rows carry no source or sequence to tell them apart).
    taken: HashMap<String, (i64, usize)>,
    result: RunResult,
    listener: &'a mut dyn FnMut(&Event),
}

/// Run a checked test against a plant: bound first (nothing runs when it is refused), then step
/// by step until a step ends it or it runs past its last step.
pub fn run(c: &Checked, plant: &dyn Plant, opts: &RunOptions, listener: &mut dyn FnMut(&Event)) -> Result<RunResult, Refused> {
    let Bound { tables, read, arm } = bind(c, plant, &opts.mode, opts.confirmed.as_deref())?;
    let (start, horizon) = match opts.mode {
        RunMode::Live => (plant.now_ms(), i64::MAX),
        RunMode::Replay { from_ms, to_ms } => (from_ms, to_ms),
    };
    // The constants, in order.
    let mut vars = Scope::new();
    for (name, e) in c.test.lets() {
        let mut env = AtEnv { plant, tables: &tables, vars: &vars, at: start, cache: None };
        match eval::eval(e, &mut env) {
            Ok(v) => {
                vars.insert(name.value.clone(), v);
            }
            Err(err) => return Err(Refused(vec![format!("the constant `{}`: {err}", name.value)])),
        }
    }
    let mut r = Runner {
        c,
        plant,
        mode: opts.mode,
        stop: opts.stop.clone(),
        clock: start,
        horizon,
        deadline: start.saturating_add(c.test.timeout_ms()),
        vars,
        attempts: HashMap::new(),
        retries: HashMap::new(),
        statements: 0,
        taken: HashMap::new(),
        result: RunResult {
            verdict: RunVerdict::Aborted,
            decided_at: None,
            reason: String::new(),
            start_ms: start,
            end_ms: start,
            steps: Vec::new(),
            criteria: Vec::new(),
            reported: Vec::new(),
            writes: Vec::new(),
            tables: read,
        },
        listener,
        tables,
    };
    // Armed for the run before its first step, disarmed at its end whichever way it ends; a switch
    // the run found armed it leaves as it found it.
    struct Disarm<'p>(&'p dyn Plant);
    impl Drop for Disarm<'_> {
        fn drop(&mut self) {
            self.0.set_armed(false);
        }
    }
    let _disarm = arm.then(|| {
        plant.set_armed(true);
        Disarm(plant)
    });
    r.main();
    Ok(r.result)
}

impl Runner<'_> {
    /// A span of the test's text, its blanks and line ends each one space, as a run quotes it.
    fn text(&self, span: Span) -> String {
        squash(self.c.text.get(span.start..span.end).unwrap_or(""))
    }

    fn emit(&mut self, e: Event) {
        (self.listener)(&e);
    }

    fn finish(&mut self, verdict: RunVerdict, at: Option<String>, reason: String) {
        self.result.verdict = verdict;
        self.result.decided_at = at.clone();
        self.result.reason = reason.clone();
        self.result.end_ms = self.clock;
        self.emit(Event::Ended { verdict, decided_at: at, reason });
    }

    fn halted(&mut self, h: Halt, at: Option<String>) {
        let reason = match &h {
            Halt::Stopped => "stopped by the tester".to_string(),
            Halt::WindowEnded(t) => format!("the window ended at {}, before the step decided", eval::time_text(*t)),
            Halt::Bound => format!("the run's bound of {} passed", eval::duration_text(self.c.test.timeout_ms())),
            Halt::Store(e) => format!("the store did not answer: {e}"),
            Halt::Steps => format!("more than {MAX_STEPS} steps without an end"),
        };
        if let Halt::WindowEnded(t) = h {
            self.clock = t;
        }
        self.finish(RunVerdict::Aborted, at, reason);
    }

    fn main(&mut self) {
        let steps = &self.c.test.steps;
        if steps.is_empty() {
            self.finish(RunVerdict::Passed, None, "the test has no step".to_string());
            return;
        }
        let mut idx = 0usize;
        let mut taken: u64 = 0;
        loop {
            taken += 1;
            let name = steps[idx].name.value.clone();
            if taken > MAX_STEPS {
                return self.halted(Halt::Steps, Some(name.clone()));
            }
            if let Err(h) = self.check_stop() {
                return self.halted(h, Some(name.clone()));
            }
            if self.past_bound() {
                return self.halted(Halt::Bound, Some(name.clone()));
            }
            let (outcome, route, why) = match self.step(idx) {
                Ok(x) => x,
                Err(h) => return self.halted(h, Some(name.clone())),
            };
            // Where the run goes: the step's own route (a branch's arm, a stop), else its `on`
            // line for the outcome, else the default.
            let mut outcome = outcome;
            let mut why = why;
            let mut route = route.or_else(|| self.on_route(idx, outcome));
            loop {
                match route.take() {
                    Some(Route::Stop(Verdict::Pass)) => {
                        let reason = if why.is_empty() { format!("`{name}` passed the run") } else { why.clone() };
                        return self.finish(RunVerdict::Passed, Some(name.clone()), reason);
                    }
                    Some(Route::Stop(Verdict::Fail)) => {
                        let reason = if why.is_empty() { format!("`{name}` failed the run") } else { why.clone() };
                        return self.finish(RunVerdict::Failed, Some(name.clone()), reason);
                    }
                    Some(Route::Step(s)) => {
                        idx = self.index(&s.value);
                        break;
                    }
                    Some(Route::Retry { step, max }) => {
                        let key = (name.clone(), step.span.start);
                        let n = self.retries.entry(key).or_insert(0);
                        *n += 1;
                        if *n <= max.value {
                            idx = self.index(&step.value);
                            break;
                        }
                        // Exhausted: the step's outcome is a failure, routed once more by its
                        // `on fail` line unless that line is the retry spent.
                        why = format!("the retry of `{}` is spent after {}", step.value, max.value);
                        let from_on = outcome == Outcome::Fail;
                        outcome = Outcome::Fail;
                        route = if from_on { None } else { self.on_route(idx, Outcome::Fail) };
                        if route.is_none() {
                            return self.finish(RunVerdict::Failed, Some(name.clone()), format!("`{name}`: {why}"));
                        }
                    }
                    None => match outcome {
                        Outcome::Pass => {
                            idx += 1;
                            if idx >= steps.len() {
                                return self.finish(RunVerdict::Passed, Some(name.clone()), "the run passed its last step".to_string());
                            }
                            break;
                        }
                        Outcome::Fail => return self.finish(RunVerdict::Failed, Some(name.clone()), format!("`{name}` failed: {why}")),
                        Outcome::Timeout => return self.finish(RunVerdict::Aborted, Some(name.clone()), format!("`{name}` timed out: {why}")),
                        Outcome::Error => return self.finish(RunVerdict::Aborted, Some(name.clone()), format!("`{name}`: {why}")),
                    },
                }
            }
        }
    }

    fn index(&self, step: &str) -> usize {
        self.c.test.steps.iter().position(|s| s.name.value == step).unwrap_or(0)
    }

    fn on_route(&self, idx: usize, outcome: Outcome) -> Option<Route> {
        self.c.test.steps[idx].on.iter().find(|o| o.outcomes.iter().any(|x| x.value == outcome)).map(|o| o.route.value.clone())
    }

    /// Whether the run's bound has passed, on the run's clock in both modes: the plant's time, which
    /// a replay of the run's own window reproduces, not the machine's work.
    fn past_bound(&self) -> bool {
        self.clock > self.deadline
    }

    fn check_stop(&self) -> R<()> {
        if self.stop.load(Ordering::SeqCst) { Err(Halt::Stopped) } else { Ok(()) }
    }

    /// Live, wait until `t` is far enough past for its rows to be stored; replay, `t` must lie in
    /// the window.
    fn settle(&self, t: i64) -> R<()> {
        match self.mode {
            RunMode::Replay { .. } => {
                self.check_stop()?;
                if t > self.horizon {
                    return Err(Halt::WindowEnded(self.horizon));
                }
                Ok(())
            }
            RunMode::Live => loop {
                self.check_stop()?;
                let lag = t + SETTLE_MS - self.plant.now_ms();
                if lag <= 0 {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(lag.min(POLL.as_millis() as i64) as u64));
            },
        }
    }

    /// Evaluate at the clock, plant data settled first.
    fn value(&self, e: &Expr, scope: &Scope) -> R<Result<Value, EvalError>> {
        if has_plant(e) {
            self.settle(self.clock)?;
        }
        let mut env = AtEnv { plant: self.plant, tables: &self.tables, vars: scope, at: self.clock, cache: None };
        Ok(eval::eval(e, &mut env))
    }

    fn measured(&self, e: &Expr, scope: &Scope) -> R<Vec<String>> {
        let mut out = Vec::new();
        for (name, leaf) in leaves(e) {
            out.push(match self.value(&leaf, scope)? {
                Ok(v) => format!("{name} = {v}"),
                Err(err) => format!("{name}: {err}"),
            });
        }
        Ok(out)
    }

    /// How far the rows may be read now, up to `until`. Live, after the first look, it waits a
    /// poll first.
    fn reach(&self, until: i64, first: bool) -> R<i64> {
        match self.mode {
            RunMode::Replay { .. } => {
                self.check_stop()?;
                Ok(until.min(self.horizon))
            }
            RunMode::Live => {
                if !first {
                    std::thread::sleep(POLL);
                }
                self.check_stop()?;
                Ok(until.min(self.plant.now_ms() - SETTLE_MS))
            }
        }
    }

    /// The rows of `tables` stamped in `(after, upto]`, in stamp order.
    fn rows(&self, tables: &[String], after: i64, upto: i64) -> R<Vec<Row>> {
        let mut all = Vec::new();
        for (i, name) in tables.iter().enumerate() {
            let Some(t) = self.tables.get(name) else { continue };
            let rows = self.plant.rows(t, after, upto).map_err(|e| Halt::Store(e.to_string()))?;
            all.extend(rows.into_iter().map(|r| (i, r)));
        }
        // Rows sharing a stamp in one order whatever the store's: by every field, then their cells.
        all.sort_by(|(ia, a), (ib, b)| {
            a.time_ms
                .cmp(&b.time_ms)
                .then(ia.cmp(ib))
                .then(a.seq.cmp(&b.seq))
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.key.cmp(&b.key))
                .then(a.arrival_ms.cmp(&b.arrival_ms))
                .then_with(|| format!("{:?}", a.columns).cmp(&format!("{:?}", b.columns)))
        });
        Ok(all.into_iter().map(|(_, r)| r).collect())
    }

    /// The end of a wait that started at the clock: its own timeout or the bound around it.
    fn until(&self, timeout: Option<i64>, bound: i64) -> (i64, bool) {
        match timeout {
            Some(t) if self.clock.saturating_add(t) <= bound => (self.clock + t, true),
            _ => (bound, false),
        }
    }

    /// Wait until `cond` holds over the plant's values, from the clock to `until`.
    fn wait_level(&mut self, cond: &Expr, until: i64, scope: &Scope) -> R<Waited> {
        let t0 = self.clock;
        let mut refs: Vec<(String, String)> = Vec::new();
        cond.walk(&mut |x: &Expr| {
            if let ExprKind::Plant { table, column } = &x.kind {
                let k = (table.value.clone(), column.value.clone());
                if !refs.contains(&k) {
                    refs.push(k);
                }
            }
        });
        self.settle(t0.min(until))?;
        let mut cache: HashMap<(String, String), Option<Value>> = HashMap::new();
        for (t, c) in &refs {
            let v = match self.tables.get(t) {
                Some(table) => self.plant.latest(table, c, t0).map_err(|e| Halt::Store(e.to_string()))?.map(|(_, v)| v),
                None => None,
            };
            cache.insert((t.clone(), c.clone()), v);
        }
        let holds = |plant: &dyn Plant, tables: &HashMap<String, Table>, cache: &HashMap<(String, String), Option<Value>>, at: i64| {
            let mut env = AtEnv { plant, tables, vars: scope, at, cache: Some(cache) };
            match eval::eval(cond, &mut env) {
                Ok(Value::Bool(b)) => Ok(b),
                Ok(other) => Err(format!("a wait takes a truth value, not {}", other.kind())),
                Err(EvalError::NoValue(_)) => Ok(false),
                Err(EvalError::Fault(f)) => Err(f),
            }
        };
        match holds(self.plant, &self.tables, &cache, t0) {
            Ok(true) => return Ok(Waited::At(t0)),
            Ok(false) => {}
            Err(f) => return Ok(Waited::Fault(f)),
        }
        let names: Vec<String> = {
            let mut v: Vec<String> = refs.iter().map(|(t, _)| t.clone()).collect();
            v.dedup();
            v
        };
        let mut cursor = t0;
        let mut first = true;
        loop {
            let upto = self.reach(until, first)?;
            first = false;
            if upto > cursor {
                let rows = self.rows(&names, cursor, upto)?;
                // The tables with more than one row at the stamp being applied.
                let mut tied: BTreeSet<String> = BTreeSet::new();
                for (i, row) in rows.iter().enumerate() {
                    // The tester's stop looked at on every row, a replay's whole window walked at once.
                    self.check_stop()?;
                    for ((t, c), slot) in cache.iter_mut() {
                        if *t == row.table
                            && let Some(Some(v)) = row.field(c)
                        {
                            *slot = Some(v);
                        }
                    }
                    if i > 0 && rows[i - 1].time_ms == row.time_ms && rows[i - 1].table == row.table {
                        tied.insert(row.table.clone());
                    }
                    // Judged at a stamp once every row of that stamp is applied: never half of one.
                    if rows.get(i + 1).is_some_and(|next| next.time_ms == row.time_ms) {
                        continue;
                    }
                    // A table's rows sharing the stamp: its values as a read at that stamp finds
                    // them, whichever the store takes for the latest of a tie.
                    for t in std::mem::take(&mut tied) {
                        let Some(table) = self.tables.get(&t) else { continue };
                        for ((tt, c), slot) in cache.iter_mut() {
                            if *tt == t {
                                *slot = self.plant.latest(table, c, row.time_ms).map_err(|e| Halt::Store(e.to_string()))?.map(|(_, v)| v);
                            }
                        }
                    }
                    match holds(self.plant, &self.tables, &cache, row.time_ms) {
                        Ok(true) => return Ok(Waited::At(row.time_ms)),
                        Ok(false) => {}
                        Err(f) => return Ok(Waited::Fault(f)),
                    }
                }
                cursor = upto;
            }
            if cursor >= until {
                return Ok(Waited::Timeout);
            }
            if let RunMode::Replay { .. } = self.mode
                && cursor >= self.horizon
            {
                return Err(Halt::WindowEnded(self.horizon));
            }
        }
    }

    /// Wait for the first row of `table` stamped after the clock for which `filter` holds, the
    /// row bound to `bind` in `scope`; at the stamp of the row the last `await` of `table` took,
    /// when the clock stands there, the rows after that one too, so a loop of awaits takes every
    /// row however many share a stamp.
    fn wait_event(&mut self, table: &str, bind: &str, filter: Option<&Expr>, until: i64, scope: &mut Scope) -> R<Waited> {
        let before = scope.get(bind).cloned();
        let skip = self.taken.get(table).filter(|t| t.0 == self.clock).map(|t| t.1);
        let mut cursor = if skip.is_some() { self.clock - 1 } else { self.clock };
        let mut first = true;
        // The stamp being walked, and how many of its rows have been met.
        let (mut stamp, mut met) = (None::<i64>, 0usize);
        loop {
            let upto = self.reach(until, first)?;
            first = false;
            if upto > cursor {
                for row in self.rows(&[table.to_string()], cursor, upto)? {
                    self.check_stop()?;
                    let at = row.time_ms;
                    if stamp != Some(at) {
                        (stamp, met) = (Some(at), 0);
                    }
                    met += 1;
                    // The rows of the last taken one's stamp, up to it, are passed over.
                    if skip.is_some_and(|k| at == self.clock && met <= k) {
                        continue;
                    }
                    scope.insert(bind.to_string(), Value::Row(Arc::new(row)));
                    let ok = match filter {
                        None => Ok(true),
                        Some(f) => {
                            let mut env = AtEnv { plant: self.plant, tables: &self.tables, vars: scope, at, cache: None };
                            match eval::eval(f, &mut env) {
                                Ok(Value::Bool(b)) => Ok(b),
                                Ok(other) => Err(format!("`where` takes a truth value, not {}", other.kind())),
                                Err(EvalError::NoValue(_)) => Ok(false),
                                Err(EvalError::Fault(x)) => Err(x),
                            }
                        }
                    };
                    match ok {
                        Ok(true) => {
                            self.taken.insert(table.to_string(), (at, met));
                            return Ok(Waited::At(at));
                        }
                        Ok(false) => {}
                        Err(f) => {
                            restore(scope, bind, before);
                            return Ok(Waited::Fault(f));
                        }
                    }
                }
                cursor = upto;
            }
            if cursor >= until {
                restore(scope, bind, before);
                return Ok(Waited::Timeout);
            }
            if let RunMode::Replay { .. } = self.mode
                && cursor >= self.horizon
            {
                restore(scope, bind, before);
                return Err(Halt::WindowEnded(self.horizon));
            }
        }
    }

    /// Wait a duration, from the clock. A replay's window ends first when it ends before both the
    /// duration and the bound: the clock never passes it.
    fn delay(&mut self, d: i64, until: i64) -> R<bool> {
        self.check_stop()?;
        let target = self.clock.saturating_add(d.max(0));
        if let RunMode::Replay { .. } = self.mode
            && target.min(until) > self.horizon
        {
            return Err(Halt::WindowEnded(self.horizon));
        }
        // Live, the machine's clock reaches the end first, the duration's or the bound's.
        let end = target.min(until);
        match self.mode {
            RunMode::Replay { .. } => {}
            RunMode::Live => loop {
                self.check_stop()?;
                let left = end - self.plant.now_ms();
                if left <= 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(left.min(POLL.as_millis() as i64) as u64));
            },
        }
        // Never back: a bound already behind the clock leaves it where it stands.
        self.clock = self.clock.max(end);
        Ok(target <= until)
    }
}

fn restore(scope: &mut Scope, name: &str, before: Option<Value>) {
    match before {
        Some(v) => {
            scope.insert(name.to_string(), v);
        }
        None => {
            scope.remove(name);
        }
    }
}

// ---- steps and actions ------------------------------------------------------------------------------

impl Runner<'_> {
    /// Run one step: its outcome, the route it chose itself (a branch's arm, a stop), and why.
    fn step(&mut self, idx: usize) -> R<(Outcome, Option<Route>, String)> {
        let step = &self.c.test.steps[idx];
        let name = step.name.value.clone();
        let attempt = {
            let n = self.attempts.entry(name.clone()).or_insert(0);
            *n += 1;
            *n
        };
        let start = self.clock;
        self.emit(Event::StepStarted { step: name.clone(), attempt, at_ms: start });
        let mut details: Vec<String> = Vec::new();
        let mut outcome = Outcome::Pass;
        let mut route = None;
        let mut why = String::new();
        let actions = step.actions.clone();
        for a in &actions {
            // The bound judged between actions too: an action after it is not run.
            let flow = match if self.past_bound() { Err(Halt::Bound) } else { self.action(a, &name, attempt) } {
                Ok(f) => f,
                Err(h) => {
                    // Cut short: what it did so far is kept, with no outcome.
                    details.retain(|d| !d.is_empty());
                    let record = StepRecord { step: name, attempt, outcome: None, detail: details.join(" · "), start_ms: start, end_ms: self.clock };
                    self.result.steps.push(record.clone());
                    self.emit(Event::StepEnded(record));
                    return Err(h);
                }
            };
            match flow {
                Flow::Pass(d) => details.push(d),
                Flow::Out(o, d) => {
                    outcome = o;
                    why = d.clone();
                    details.push(d);
                    break;
                }
                Flow::Route(r, d) => {
                    route = Some(r);
                    details.push(d);
                    break;
                }
                Flow::Stop(v, d) => {
                    route = Some(Route::Stop(v));
                    why = d.clone();
                    details.push(d);
                    break;
                }
            }
        }
        if outcome == Outcome::Fail && why.is_empty() {
            why = details.last().cloned().unwrap_or_default();
        }
        details.retain(|d| !d.is_empty());
        let record = StepRecord { step: name, attempt, outcome: Some(outcome), detail: details.join(" · "), start_ms: start, end_ms: self.clock };
        self.result.steps.push(record.clone());
        self.emit(Event::StepEnded(record));
        Ok((outcome, route, why))
    }

    fn action(&mut self, a: &Action, step: &str, attempt: u32) -> R<Flow> {
        let vars = self.vars.clone();
        Ok(match &a.kind {
            ActionKind::Write { target, value } => {
                let what = format!("{}.{}", target.table.value, target.column.value);
                let v = match self.value(value, &vars)? {
                    Ok(v) => v,
                    Err(e) => return Ok(Flow::Out(Outcome::Error, format!("{what} := {}: {e}", self.text(value.span)))),
                };
                let text = format!("{what} := {}", wire_text(&v));
                self.gated(step, attempt, text, |p| p.write(&target.table.value, &target.column.value, &v))?
            }
            ActionKind::Send { channel, message, fields } => {
                let mut out = Vec::new();
                for f in fields {
                    match self.value(&f.value, &vars)? {
                        Ok(v) => out.push((f.name.value.clone(), wire_text(&v))),
                        Err(e) => return Ok(Flow::Out(Outcome::Error, format!("{}: {e}", f.name.value))),
                    }
                }
                let text = format!(
                    "{}.{}({})",
                    channel.value,
                    message.value,
                    out.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")
                );
                self.gated(step, attempt, text, |p| p.send(&channel.value, &message.value, out.clone()).map(|_| "sent".to_string()))?
            }
            ActionKind::Wait { condition, timeout } => {
                let (until, own) = self.until(timeout.as_ref().map(|t| t.value), self.deadline);
                let t0 = self.clock;
                let text = self.text(condition.span);
                match self.wait_level(condition, until, &vars)? {
                    Waited::At(t) => {
                        self.clock = t;
                        Flow::Pass(format!("{text} held after {}", eval::duration_text(t - t0)))
                    }
                    Waited::Timeout if own => {
                        self.clock = until;
                        Flow::Out(Outcome::Timeout, format!("{text} did not hold within {}", eval::duration_text(until - t0)))
                    }
                    Waited::Timeout => {
                        // The run's bound passed inside the wait: the clock stands at it, or past it
                        // when it already was.
                        self.clock = self.clock.max(until);
                        return Err(Halt::Bound);
                    }
                    Waited::Fault(f) => Flow::Out(Outcome::Error, format!("{text}: {f}")),
                }
            }
            ActionKind::Delay { duration } => {
                if self.delay(duration.value, self.deadline)? {
                    Flow::Pass(format!("waited {}", eval::duration_text(duration.value)))
                } else {
                    return Err(Halt::Bound);
                }
            }
            ActionKind::Await { table, filter, timeout, bind } => {
                let (until, own) = self.until(timeout.as_ref().map(|t| t.value), self.deadline);
                let t0 = self.clock;
                let mut scope = vars;
                match self.wait_event(&table.value, &bind.value, filter.as_ref(), until, &mut scope)? {
                    Waited::At(t) => {
                        self.clock = t;
                        if let Some(v) = scope.get(&bind.value) {
                            self.vars.insert(bind.value.clone(), v.clone());
                        }
                        Flow::Pass(format!("a row of `{}` after {}", table.value, eval::duration_text(t - t0)))
                    }
                    Waited::Timeout if own => {
                        self.clock = until;
                        Flow::Out(Outcome::Timeout, format!("no row of `{}` within {}", table.value, eval::duration_text(until - t0)))
                    }
                    Waited::Timeout => {
                        self.clock = self.clock.max(until);
                        return Err(Halt::Bound);
                    }
                    Waited::Fault(f) => Flow::Out(Outcome::Error, f),
                }
            }
            ActionKind::Read { refs, into } => {
                self.settle(self.clock)?;
                let mut got = Vec::new();
                for (r, n) in refs.iter().zip(into) {
                    let t = &self.tables[&r.table.value];
                    match self.plant.latest(t, &r.column.value, self.clock).map_err(|e| Halt::Store(e.to_string()))? {
                        Some((_, v)) => {
                            got.push(format!("{} = {v}", n.value));
                            self.vars.insert(n.value.clone(), v);
                        }
                        None => {
                            return Ok(Flow::Out(Outcome::Error, format!("`{}.{}` has no value yet", r.table.value, r.column.value)));
                        }
                    }
                }
                Flow::Pass(got.join(", "))
            }
            ActionKind::Check { condition, label } => {
                let text = label.as_ref().map(|l| l.value.clone()).unwrap_or_else(|| self.text(condition.span));
                let measured = self.measured(condition, &vars)?;
                let (flow, result) = match self.value(condition, &vars)? {
                    Ok(Value::Bool(true)) => (Flow::Pass(format!("{text}: pass")), "pass".to_string()),
                    Ok(Value::Bool(false)) => (Flow::Out(Outcome::Fail, format!("{text}: fail ({})", measured.join(", "))), "fail".to_string()),
                    Ok(other) => (Flow::Out(Outcome::Error, format!("{text}: a check takes a truth value, not {}", other.kind())), "error".to_string()),
                    Err(e) => (Flow::Out(Outcome::Error, format!("{text}: {e}")), "error".to_string()),
                };
                self.criterion(Criterion { step: step.to_string(), attempt, text, measured, result });
                flow
            }
            ActionKind::Call { function, args, outputs } => {
                let Some((f, file)) = self.function(&function.value) else {
                    return Ok(Flow::Out(Outcome::Error, format!("`{}` is no function", function.value)));
                };
                self.statements = 0;
                let (o, why, values) = self.call(&f, file, args, &vars, self.deadline)?;
                if o != Outcome::Pass {
                    return Ok(Flow::Out(o, format!("{}: {why}", function.value)));
                }
                let mut shown = Vec::new();
                for (n, v) in outputs.iter().zip(values) {
                    shown.push(format!("{} {v}", n.value));
                    self.vars.insert(n.value.clone(), v);
                }
                Flow::Pass(shown.join(" · "))
            }
            ActionKind::Script { body } => {
                let mut scope = vars.clone();
                self.statements = 0;
                let flow = self.exec(body, &mut scope, &Ctx { until: self.deadline, file: None })?;
                // A script's names are the test's; its detail is what it set, in name order.
                let mut set: Vec<String> =
                    scope.iter().filter(|(k, v)| vars.get(*k) != Some(*v)).map(|(k, v)| format!("{k} = {v}")).collect();
                set.sort();
                self.vars = scope;
                match flow {
                    StmtFlow::Next => Flow::Pass(set.join(", ")),
                    StmtFlow::Out(o, why) => Flow::Out(o, why),
                }
            }
            ActionKind::Branch { value, arms, otherwise } => self.branch(value, arms, otherwise.as_ref(), step, attempt, &vars)?,
            ActionKind::Report { items } => {
                for item in items {
                    let note = item.is_string();
                    let text = match self.value(item, &vars)? {
                        Ok(v) if note => v.to_string(),
                        Ok(v) => format!("{} = {v}", self.text(item.span)),
                        Err(e) => format!("{}: {e}", self.text(item.span)),
                    };
                    let rep = Reported { step: step.to_string(), attempt, text, note };
                    self.result.reported.push(rep.clone());
                    self.emit(Event::Reported(rep));
                }
                Flow::Pass(format!("{} reported", items.len()))
            }
            ActionKind::Stop { verdict, reason } => {
                let why = match reason {
                    Some(r) => match self.value(r, &vars)? {
                        Ok(v) => v.to_string(),
                        Err(_) => self.text(r.span),
                    },
                    None => String::new(),
                };
                Flow::Stop(*verdict, why)
            }
        })
    }

    /// A write or a send: sent live and its answer kept; noted, not sent, in a replay.
    fn gated(&mut self, step: &str, attempt: u32, text: String, send: impl Fn(&dyn Plant) -> Result<String, PlantError>) -> R<Flow> {
        self.check_stop()?;
        let (flow, record) = match self.mode {
            // DECISION PENDING (Q318): a replay's write leaves the clock where it stands, where the
            // live one moved it to the plant's answer.
            RunMode::Replay { .. } => (
                Flow::Pass(format!("{text}: not sent: replay")),
                WriteRecord { step: step.to_string(), attempt, text, sent: false, answer: "not sent: replay".to_string(), ok: true },
            ),
            RunMode::Live => {
                self.check_stop()?;
                let answer = send(self.plant);
                // Whatever the plant does after the write is after the write.
                self.clock = self.clock.max(self.plant.now_ms());
                match answer {
                    Ok(a) => (Flow::Pass(format!("{text}: {a}")), WriteRecord { step: step.to_string(), attempt, text, sent: true, answer: a, ok: true }),
                    Err(e) => {
                        // Sent when it left portal for the plant: the plant's own refusal, and a write
                        // whose answer never came; not when portal refused it first.
                        let sent = matches!(e, PlantError::Answered(_) | PlantError::Unconfirmed(_));
                        (Flow::Out(Outcome::Error, format!("{text}: {e}")), WriteRecord { step: step.to_string(), attempt, text, sent, answer: e.to_string(), ok: false })
                    }
                }
            }
        };
        self.result.writes.push(record.clone());
        self.emit(Event::Write(record));
        Ok(flow)
    }

    fn criterion(&mut self, c: Criterion) {
        self.result.criteria.push(c.clone());
        self.emit(Event::Criterion(c));
    }

    fn branch(&mut self, value: &Expr, arms: &[Arm], otherwise: Option<&Spanned<Route>>, step: &str, attempt: u32, vars: &Scope) -> R<Flow> {
        let mut text = vec![self.text(value.span)];
        text.extend(arms.iter().map(|a| self.text(a.span)));
        if let Some(o) = otherwise {
            text.push(format!("else -> {}", self.text(o.span)));
        }
        let text = format!("{}: {}", text[0], text[1..].join("; "));
        let measured = self.measured(value, vars)?;
        let v = match self.value(value, vars)? {
            Ok(v) => v,
            Err(e) => {
                self.criterion(Criterion { step: step.to_string(), attempt, text, measured, result: "error".to_string() });
                return Ok(Flow::Out(Outcome::Error, format!("{}: {e}", self.text(value.span))));
            }
        };
        for arm in arms {
            for p in &arm.patterns {
                let hit = match p {
                    Pattern::Compare { op, value: pv } => self.value(pv, vars)?.and_then(|pv| compare(*op, &v, &pv)),
                    Pattern::Value(pv) => self.value(pv, vars)?.and_then(|pv| compare(CmpOp::Eq, &v, &pv)),
                    Pattern::Range { low, high } => {
                        let lo = self.value(low, vars)?;
                        let hi = self.value(high, vars)?;
                        lo.and_then(|lo| compare(CmpOp::Ge, &v, &lo)).and_then(|a| Ok(a && compare(CmpOp::Le, &v, &hi?)?))
                    }
                };
                match hit {
                    Ok(true) => {
                        let took = self.text(arm.span);
                        self.criterion(Criterion { step: step.to_string(), attempt, text, measured, result: took.clone() });
                        return Ok(Flow::Route(arm.route.value.clone(), format!("{v} matched {took}")));
                    }
                    Ok(false) => {}
                    Err(e) => {
                        self.criterion(Criterion { step: step.to_string(), attempt, text, measured, result: "error".to_string() });
                        return Ok(Flow::Out(Outcome::Error, format!("{}: {e}", self.text(arm.span))));
                    }
                }
            }
        }
        match otherwise {
            Some(o) => {
                let took = format!("else -> {}", self.text(o.span));
                self.criterion(Criterion { step: step.to_string(), attempt, text, measured, result: took.clone() });
                Ok(Flow::Route(o.value.clone(), format!("{v} matched {took}")))
            }
            None => {
                self.criterion(Criterion { step: step.to_string(), attempt, text, measured, result: "no arm".to_string() });
                Ok(Flow::Out(Outcome::Fail, format!("{v} matched no arm")))
            }
        }
    }

    /// A function by name, and the file it is in.
    fn function(&self, name: &str) -> Option<(Function, usize)> {
        self.c.files.iter().enumerate().find_map(|(i, f)| f.functions.functions.iter().find(|x| x.name.value == name).map(|x| (x.clone(), i)))
    }
}

// ---- statements and calls ------------------------------------------------------------------------

/// Where a block of statements runs: the bound of the call it is in, and its file.
struct Ctx {
    until: i64,
    file: Option<usize>,
}

impl Runner<'_> {
    /// One statement, or one turn of a `for`, counted: past the budget, the box or the call ends in
    /// an error; the tester's stop looked at every time, the run's bound every `BOUND_EVERY`.
    fn tick(&mut self) -> R<Option<StmtFlow>> {
        self.statements += 1;
        if self.statements > MAX_STATEMENTS {
            return Ok(Some(StmtFlow::Out(Outcome::Error, format!("more than {MAX_STATEMENTS} statements"))));
        }
        self.check_stop()?;
        if self.statements.is_multiple_of(BOUND_EVERY) && self.past_bound() {
            return Err(Halt::Bound);
        }
        Ok(None)
    }

    fn exec(&mut self, body: &[Stmt], scope: &mut Scope, ctx: &Ctx) -> R<StmtFlow> {
        for s in body {
            if let Some(out) = self.tick()? {
                return Ok(out);
            }
            let flow = self.stmt(s, scope, ctx)?;
            if !matches!(flow, StmtFlow::Next) {
                return Ok(flow);
            }
        }
        Ok(StmtFlow::Next)
    }

    fn stmt(&mut self, s: &Stmt, scope: &mut Scope, ctx: &Ctx) -> R<StmtFlow> {
        let err = |e: EvalError| StmtFlow::Out(Outcome::Error, e.to_string());
        Ok(match &s.kind {
            StmtKind::Assign { target, op, value } => {
                let v = match self.value(value, scope)? {
                    Ok(v) => v,
                    Err(e) => return Ok(err(e)),
                };
                let v = match op {
                    AssignOp::Set => v,
                    _ => {
                        let Some(old) = scope.get(&target.value).cloned() else {
                            return Ok(err(EvalError::NoValue(format!("`{}`", target.value))));
                        };
                        let bin = match op {
                            AssignOp::Add => BinOp::Add,
                            AssignOp::Sub => BinOp::Sub,
                            AssignOp::Mul => BinOp::Mul,
                            _ => BinOp::Div,
                        };
                        match eval::arith(bin, &old, &v) {
                            Ok(v) => v,
                            Err(e) => return Ok(err(e)),
                        }
                    }
                };
                scope.insert(target.value.clone(), v);
                StmtFlow::Next
            }
            StmtKind::If { arms, otherwise } => {
                for (cond, b) in arms {
                    match self.value(cond, scope)? {
                        Ok(Value::Bool(true)) => return self.exec(b, scope, ctx),
                        Ok(Value::Bool(false)) => {}
                        Ok(other) => return Ok(StmtFlow::Out(Outcome::Error, format!("`if` takes a truth value, not {}", other.kind()))),
                        Err(e) => return Ok(err(e)),
                    }
                }
                match otherwise {
                    Some(b) => self.exec(b, scope, ctx)?,
                    None => StmtFlow::Next,
                }
            }
            StmtKind::For { var, from, to, body } => {
                let (a, b) = match (self.value(from, scope)?, self.value(to, scope)?) {
                    (Ok(Value::Int(a)), Ok(Value::Int(b))) => (a, b),
                    (Err(e), _) | (_, Err(e)) => return Ok(err(e)),
                    _ => return Ok(StmtFlow::Out(Outcome::Error, "a `for` runs over whole numbers".to_string())),
                };
                let mut i = a;
                while i <= b {
                    if let Some(out) = self.tick()? {
                        return Ok(out);
                    }
                    scope.insert(var.value.clone(), Value::Int(i));
                    let f = self.exec(body, scope, ctx)?;
                    if !matches!(f, StmtFlow::Next) {
                        return Ok(f);
                    }
                    // The last turn at the largest whole number ends the loop, past nothing.
                    let Some(next) = i.checked_add(1) else { break };
                    i = next;
                }
                StmtFlow::Next
            }
            StmtKind::Fail { reason } => {
                let why = match reason {
                    Some(r) => match self.value(r, scope)? {
                        Ok(v) => v.to_string(),
                        Err(e) => e.to_string(),
                    },
                    None => "failed".to_string(),
                };
                StmtFlow::Out(Outcome::Fail, why)
            }
            StmtKind::Wait { condition, timeout } => {
                let (until, own) = self.until(timeout.as_ref().map(|t| t.value), ctx.until);
                match self.wait_level(condition, until, scope)? {
                    Waited::At(t) => {
                        self.clock = t;
                        StmtFlow::Next
                    }
                    Waited::Timeout => {
                        self.clock = self.clock.max(until);
                        if !own && until >= self.deadline {
                            return Err(Halt::Bound);
                        }
                        StmtFlow::Out(Outcome::Timeout, format!("{} did not hold in time", self.text_in(ctx, condition.span)))
                    }
                    Waited::Fault(f) => StmtFlow::Out(Outcome::Error, f),
                }
            }
            StmtKind::Delay { duration } => {
                if self.delay(duration.value, ctx.until)? {
                    StmtFlow::Next
                } else if ctx.until >= self.deadline {
                    return Err(Halt::Bound);
                } else {
                    StmtFlow::Out(Outcome::Timeout, format!("waiting {} passed the call's bound", eval::duration_text(duration.value)))
                }
            }
            StmtKind::Await { bind, table, filter, timeout } => {
                let (until, own) = self.until(timeout.as_ref().map(|t| t.value), ctx.until);
                match self.wait_event(&table.value, &bind.value, filter.as_ref(), until, scope)? {
                    Waited::At(t) => {
                        self.clock = t;
                        StmtFlow::Next
                    }
                    Waited::Timeout => {
                        self.clock = self.clock.max(until);
                        if !own && until >= self.deadline {
                            return Err(Halt::Bound);
                        }
                        StmtFlow::Out(Outcome::Timeout, format!("no row of `{}` in time", table.value))
                    }
                    Waited::Fault(f) => StmtFlow::Out(Outcome::Error, f),
                }
            }
            StmtKind::Call { outputs, function, args } => {
                let Some(file) = ctx.file else {
                    return Ok(StmtFlow::Out(Outcome::Error, "a script box calls no function".to_string()));
                };
                let Some(f) = self.c.files[file].functions.functions.iter().find(|x| x.name.value == function.value).cloned() else {
                    return Ok(StmtFlow::Out(Outcome::Error, format!("`{}` is no function of this file", function.value)));
                };
                let (o, why, values) = self.call(&f, file, args, scope, ctx.until)?;
                if o != Outcome::Pass {
                    return Ok(StmtFlow::Out(o, format!("{}: {why}", function.value)));
                }
                for (n, v) in outputs.iter().zip(values) {
                    scope.insert(n.value.clone(), v);
                }
                StmtFlow::Next
            }
        })
    }

    /// The text of an expression in the file it was read from: the test's, or a function file's.
    fn text_in(&self, ctx: &Ctx, span: Span) -> String {
        let text = match ctx.file {
            Some(f) => &self.c.files[f].text,
            None => &self.c.text,
        };
        squash(text.get(span.start..span.end).unwrap_or(""))
    }

    /// Call a function: its inputs from the caller's scope, its body under its own bound and its
    /// caller's (`within`: the run's for a step, the calling function's for a call inside one),
    /// its outputs.
    fn call(&mut self, f: &Function, file: usize, args: &[Arg], caller: &Scope, within: i64) -> R<(Outcome, String, Vec<Value>)> {
        let mut local = Scope::new();
        for input in &f.inputs {
            let Some(arg) = args.iter().find(|a| a.name.value == input.name.value) else {
                return Ok((Outcome::Error, format!("input `{}` not given", input.name.value), Vec::new()));
            };
            let v = match self.value(&arg.value, caller)? {
                Ok(v) => v,
                Err(e) => return Ok((Outcome::Error, format!("input `{}`: {e}", input.name.value), Vec::new())),
            };
            let kind = v.kind();
            match fits(v, input.ty.value) {
                Some(v) => {
                    local.insert(input.name.value.clone(), v);
                }
                None => return Ok((Outcome::Error, format!("input `{}` takes {}, not {kind}", input.name.value, input.ty.value), Vec::new())),
            }
        }
        let until = match &f.timeout {
            Some(t) => self.clock.saturating_add(t.value).min(within),
            None => within,
        };
        let flow = self.exec(&f.body, &mut local, &Ctx { until, file: Some(file) })?;
        if let StmtFlow::Out(o, why) = flow {
            return Ok((o, why, Vec::new()));
        }
        let mut out = Vec::new();
        for o in &f.outputs {
            let Some(v) = local.get(&o.name.value).cloned() else {
                return Ok((Outcome::Error, format!("output `{}` was never assigned", o.name.value), Vec::new()));
            };
            let kind = v.kind();
            match fits(v, o.ty.value) {
                Some(v) => out.push(v),
                None => return Ok((Outcome::Error, format!("output `{}` is {}, not {}", o.name.value, kind, o.ty.value), Vec::new())),
            }
        }
        Ok((Outcome::Pass, String::new(), out))
    }
}


/// Text on one line: every run of blanks and line ends one space.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
