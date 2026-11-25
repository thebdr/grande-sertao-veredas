//! A test and its functions as the engine runs them (`docs/test-schema.md`), every part with its
//! place in its file's text, so the text view can point at it and the flow view can rewrite it.
//!
//! The model does not depend on the syntax it was read from (`docs/questions.md` Q310): the
//! parser builds it, the check resolves its names, the engine runs it.

use std::fmt;

/// A byte range in a file's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// The range from this one's start to `other`'s end.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start, end: other.end.max(self.end) }
    }
}

/// A place in a text: line and column from 1, the column counted in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub line: usize,
    pub column: usize,
}

impl Place {
    /// The place of byte `at` in `text`.
    pub fn of(text: &str, at: usize) -> Place {
        let at = at.min(text.len());
        let before = &text[..at];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        Place { line, column: text[line_start..at].chars().count() + 1 }
    }
}

/// A value with the range it was read from.
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Spanned<T> {
        Spanned { value, span }
    }
}

/// A test file: one test, and the flow view's positions.
#[derive(Clone, Debug, PartialEq)]
pub struct TestFile {
    pub title: Spanned<String>,
    /// The `test` keyword's range: where a problem of the whole test points.
    pub head: Span,
    pub settings: Vec<Setting>,
    pub steps: Vec<Step>,
    pub layout: Option<Layout>,
}

/// A setting, in the order written.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    /// The key's range.
    pub key_span: Span,
    pub kind: SettingKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    Id(Spanned<String>),
    Area(Spanned<String>),
    Portal(Spanned<String>),
    Use(Spanned<String>),
    Modes(Vec<Spanned<Mode>>),
    Timeout(Spanned<i64>),
    /// The roles that must sign the run's report (Q312 (e)).
    Sign(Vec<Spanned<String>>),
    Let { name: Spanned<String>, value: Expr },
}

impl SettingKind {
    pub fn key(&self) -> &'static str {
        match self {
            SettingKind::Id(_) => "id",
            SettingKind::Area(_) => "area",
            SettingKind::Portal(_) => "portal",
            SettingKind::Use(_) => "use",
            SettingKind::Modes(_) => "modes",
            SettingKind::Timeout(_) => "timeout",
            SettingKind::Sign(_) => "sign",
            SettingKind::Let { .. } => "let",
        }
    }
}

/// The setting keys, in the order the reference lists them.
pub const SETTINGS: &[&str] = &["id", "area", "portal", "modes", "timeout", "use", "sign", "let"];

/// The roles a report is signed as.
pub const ROLES: [&str; 2] = ["tester", "witness"];

/// A run's mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mode {
    Live,
    Replay,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Mode::Live => "live",
            Mode::Replay => "replay",
        })
    }
}

/// The bound of a run when the test sets none: ten minutes.
pub const DEFAULT_TIMEOUT_MS: i64 = 10 * 60 * 1000;

impl TestFile {
    /// The test's `id`, when it has one.
    pub fn id(&self) -> Option<&str> {
        self.settings.iter().find_map(|s| match &s.kind {
            SettingKind::Id(v) => Some(v.value.as_str()),
            _ => None,
        })
    }

    pub fn area(&self) -> Option<&str> {
        self.settings.iter().find_map(|s| match &s.kind {
            SettingKind::Area(v) => Some(v.value.as_str()),
            _ => None,
        })
    }

    pub fn portal(&self) -> Option<&str> {
        self.settings.iter().find_map(|s| match &s.kind {
            SettingKind::Portal(v) => Some(v.value.as_str()),
            _ => None,
        })
    }

    /// The modes the test runs in: both when it says nothing.
    pub fn modes(&self) -> Vec<Mode> {
        self.settings
            .iter()
            .find_map(|s| match &s.kind {
                SettingKind::Modes(m) => Some(m.iter().map(|m| m.value).collect()),
                _ => None,
            })
            .unwrap_or_else(|| vec![Mode::Live, Mode::Replay])
    }

    /// The run's bound in milliseconds: ten minutes when the test says nothing.
    /// The roles the test names to sign its runs' reports, in its order; none when it names none.
    pub fn sign_roles(&self) -> Vec<String> {
        self.settings
            .iter()
            .find_map(|s| match &s.kind {
                SettingKind::Sign(r) => Some(r.iter().map(|r| r.value.clone()).collect()),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn timeout_ms(&self) -> i64 {
        self.settings
            .iter()
            .find_map(|s| match &s.kind {
                SettingKind::Timeout(t) => Some(t.value),
                _ => None,
            })
            .unwrap_or(DEFAULT_TIMEOUT_MS)
    }

    /// The function files it uses, as written.
    pub fn uses(&self) -> Vec<&Spanned<String>> {
        self.settings
            .iter()
            .filter_map(|s| match &s.kind {
                SettingKind::Use(p) => Some(p),
                _ => None,
            })
            .collect()
    }

    /// The constants, in order.
    pub fn lets(&self) -> Vec<(&Spanned<String>, &Expr)> {
        self.settings
            .iter()
            .filter_map(|s| match &s.kind {
                SettingKind::Let { name, value } => Some((name, value)),
                _ => None,
            })
            .collect()
    }

    /// A step by name.
    pub fn step(&self, name: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.name.value == name)
    }
}

/// A step: a name, its actions in order, and the routes of its outcomes.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub name: Spanned<String>,
    /// From `step` to the end of its last line.
    pub span: Span,
    pub actions: Vec<Action>,
    pub on: Vec<OnRoute>,
}

impl Step {
    /// The block the flow view draws it as: its first action's, a step that ends in `stop fail`
    /// being the fail end.
    pub fn block(&self) -> Block {
        if let Some(Action { kind: ActionKind::Stop { verdict: Verdict::Fail, .. }, .. }) = self.actions.last() {
            return Block::Fail;
        }
        self.actions.first().map(Action::block).unwrap_or(Block::Script)
    }
}

/// The kinds of block the flow view draws (`docs/design/comissionador/README.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A write or a send: gated.
    Write,
    Wait,
    Read,
    Check,
    Branch,
    Script,
    Function,
    Report,
    /// `stop pass`.
    End,
    /// A step ending in `stop fail`.
    Fail,
}

/// An action of a step.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub kind: ActionKind,
    /// From its keyword to the end of its line (its last line, for a block).
    pub span: Span,
}

impl Action {
    pub fn block(&self) -> Block {
        match &self.kind {
            ActionKind::Write { .. } | ActionKind::Send { .. } => Block::Write,
            ActionKind::Wait { .. } | ActionKind::Delay { .. } | ActionKind::Await { .. } => Block::Wait,
            ActionKind::Read { .. } => Block::Read,
            ActionKind::Check { .. } => Block::Check,
            ActionKind::Call { .. } => Block::Function,
            ActionKind::Script { .. } => Block::Script,
            ActionKind::Branch { .. } => Block::Branch,
            ActionKind::Report { .. } => Block::Report,
            ActionKind::Stop { verdict: Verdict::Pass, .. } => Block::End,
            ActionKind::Stop { verdict: Verdict::Fail, .. } => Block::Fail,
        }
    }

    /// The action's keyword.
    pub fn keyword(&self) -> &'static str {
        match &self.kind {
            ActionKind::Write { .. } => "write",
            ActionKind::Send { .. } => "send",
            ActionKind::Wait { .. } | ActionKind::Delay { .. } => "wait",
            ActionKind::Await { .. } => "await",
            ActionKind::Read { .. } => "read",
            ActionKind::Check { .. } => "check",
            ActionKind::Call { .. } => "call",
            ActionKind::Script { .. } => "script",
            ActionKind::Branch { .. } => "branch",
            ActionKind::Report { .. } => "report",
            ActionKind::Stop { .. } => "stop",
        }
    }

    /// Whether it touches the plant: a write or a send, behind portal's gate.
    pub fn is_gated(&self) -> bool {
        matches!(self.kind, ActionKind::Write { .. } | ActionKind::Send { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActionKind {
    Write { target: PlantRef, value: Expr },
    Send { channel: Spanned<String>, message: Spanned<String>, fields: Vec<Arg> },
    Wait { condition: Expr, timeout: Option<Spanned<i64>> },
    Delay { duration: Spanned<i64> },
    Await { table: Spanned<String>, filter: Option<Expr>, timeout: Option<Spanned<i64>>, bind: Spanned<String> },
    Read { refs: Vec<PlantRef>, into: Vec<Spanned<String>> },
    Check { condition: Expr, label: Option<Spanned<String>> },
    Call { function: Spanned<String>, args: Vec<Arg>, outputs: Vec<Spanned<String>> },
    Script { body: Vec<Stmt> },
    Branch { value: Expr, arms: Vec<Arm>, otherwise: Option<Spanned<Route>> },
    Report { items: Vec<Expr> },
    Stop { verdict: Verdict, reason: Option<Expr> },
}

/// `<table>.<column>` written where only plant data can stand (a write's target, a read).
#[derive(Clone, Debug, PartialEq)]
pub struct PlantRef {
    pub table: Spanned<String>,
    pub column: Spanned<String>,
}

impl PlantRef {
    pub fn span(&self) -> Span {
        self.table.span.to(self.column.span)
    }
}

/// `<name>: <expression>`, an input of a call or a field of a send.
#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Spanned<String>,
    pub value: Expr,
}

/// How a run ends by `stop`, or the verdict a route ends it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}

/// An action's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outcome {
    Pass,
    Fail,
    Timeout,
    Error,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
            Outcome::Timeout => "timeout",
            Outcome::Error => "error",
        })
    }
}

/// Where the run goes next.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Step(Spanned<String>),
    Retry { step: Spanned<String>, max: Spanned<u32> },
    Stop(Verdict),
}

/// `on <outcome>, ... -> <route>`.
#[derive(Clone, Debug, PartialEq)]
pub struct OnRoute {
    pub outcomes: Vec<Spanned<Outcome>>,
    pub route: Spanned<Route>,
    pub span: Span,
}

/// A branch's arm: patterns and a route.
#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub patterns: Vec<Pattern>,
    pub route: Spanned<Route>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    /// `>= 0.95`, `== "FULL"`: a comparison and a value.
    Compare { op: CmpOp, value: Expr },
    /// `0.85 .. 0.95`, both ends included.
    Range { low: Expr, high: Expr },
    /// A value, matched by equality.
    Value(Expr),
}

/// The flow view's positions.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub span: Span,
    pub positions: Vec<Position>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub step: Spanned<String>,
    pub column: u32,
    pub row: u32,
}

/// A function file: one or more functions.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionFile {
    pub functions: Vec<Function>,
}

/// A function: written once in text, placed in a test as a block.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: Spanned<String>,
    /// The comment lines right above `function`, `#` and one space taken off each.
    pub description: String,
    pub inputs: Vec<Param>,
    pub outputs: Vec<Param>,
    pub reads: Vec<Spanned<String>>,
    pub timeout: Option<Spanned<i64>>,
    pub body: Vec<Stmt>,
    /// From `function` to its closing brace.
    pub span: Span,
}

impl Function {
    /// `Name(a: int, b: int) -> (c: int)`: the block's ports, as the reference writes them.
    pub fn signature(&self) -> String {
        let list = |ps: &[Param]| ps.iter().map(|p| format!("{}: {}", p.name.value, p.ty.value)).collect::<Vec<_>>().join(", ");
        let mut s = format!("{}({})", self.name.value, list(&self.inputs));
        if !self.outputs.is_empty() {
            s.push_str(&format!(" -> ({})", list(&self.outputs)));
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: Spanned<String>,
    pub ty: Spanned<Type>,
}

/// The types of a function's inputs and outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Text,
    Time,
    Duration,
}

impl Type {
    pub fn parse(word: &str) -> Option<Type> {
        Some(match word {
            "int" => Type::Int,
            "float" => Type::Float,
            "bool" => Type::Bool,
            "text" => Type::Text,
            "time" => Type::Time,
            "duration" => Type::Duration,
            _ => return None,
        })
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Type::Int => "int",
            Type::Float => "float",
            Type::Bool => "bool",
            Type::Text => "text",
            Type::Time => "time",
            Type::Duration => "duration",
        })
    }
}

/// A statement of a script box or a function.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Assign { target: Spanned<String>, op: AssignOp, value: Expr },
    If { arms: Vec<(Expr, Vec<Stmt>)>, otherwise: Option<Vec<Stmt>> },
    For { var: Spanned<String>, from: Expr, to: Expr, body: Vec<Stmt> },
    Fail { reason: Option<Expr> },
    Wait { condition: Expr, timeout: Option<Spanned<i64>> },
    Delay { duration: Spanned<i64> },
    Await { bind: Spanned<String>, table: Spanned<String>, filter: Option<Expr>, timeout: Option<Spanned<i64>> },
    Call { outputs: Vec<Spanned<String>>, function: Spanned<String>, args: Vec<Arg> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
}

/// An expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Whole milliseconds.
    Duration(i64),
    Str(Vec<StrPart>),
    /// A variable.
    Name(String),
    /// `<name>.<field>`: a row's column once the check has found the name a variable; anything
    /// else the check turns into [`ExprKind::Plant`].
    Field { base: Spanned<String>, field: Spanned<String> },
    /// Plant data: a column's latest value in a portal table.
    Plant { table: Spanned<String>, column: Spanned<String> },
    /// A built-in call.
    Call { name: Spanned<String>, args: Vec<Expr> },
    Unary { op: UnOp, expr: Box<Expr> },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Text(String),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Cmp(CmpOp),
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn text(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// The built-in calls and how many values each takes (`None`: one or more).
pub const BUILTINS: &[(&str, Option<(usize, usize)>)] = &[
    ("abs", Some((1, 1))),
    ("min", None),
    ("max", None),
    ("round", Some((1, 2))),
    ("seconds", Some((1, 1))),
    ("now", Some((0, 0))),
    ("int", Some((1, 1))),
    ("float", Some((1, 1))),
    ("text", Some((1, 1))),
];

pub fn is_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|(n, _)| *n == name)
}

impl Expr {
    /// Visit this expression and every one inside it, outermost first.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        f(self);
        match &self.kind {
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPart::Expr(e) = p {
                        e.walk(f);
                    }
                }
            }
            ExprKind::Call { args, .. } => args.iter().for_each(|a| a.walk(f)),
            ExprKind::Unary { expr, .. } => expr.walk(f),
            ExprKind::Binary { left, right, .. } => {
                left.walk(f);
                right.walk(f);
            }
            _ => {}
        }
    }

    /// The same, mutably.
    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Expr)) {
        f(self);
        match &mut self.kind {
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPart::Expr(e) = p {
                        e.walk_mut(f);
                    }
                }
            }
            ExprKind::Call { args, .. } => args.iter_mut().for_each(|a| a.walk_mut(f)),
            ExprKind::Unary { expr, .. } => expr.walk_mut(f),
            ExprKind::Binary { left, right, .. } => {
                left.walk_mut(f);
                right.walk_mut(f);
            }
            _ => {}
        }
    }

    /// Whether it is a string literal: a note, when a `report` item.
    pub fn is_string(&self) -> bool {
        matches!(self.kind, ExprKind::Str(_))
    }
}
