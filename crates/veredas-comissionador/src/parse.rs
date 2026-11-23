//! A test file and a function file read into the model (`docs/test-schema.md`).
//!
//! The header comes first: every line before the first line that opens with `test` or
//! `function` is TOML holding `format` and `version` alone, read by core's one header reader, so
//! a file of another format, an unknown version or a missing stamp is refused by the texts every
//! file of the suite shares, before the body is looked at. The body is then read token by token;
//! the reading stops at a file's first error of form, named with its place.

use crate::Problem;
use crate::lex::{self, StrPiece, Sym, Tok, Token, is_keyword};
use crate::model::*;

/// A test file's format.
pub const TEST_FORMAT: &str = "veredas-test";
/// A function file's format.
pub const FUNCTIONS_FORMAT: &str = "veredas-functions";
/// The one version of each this reader knows.
pub const VERSION: u32 = 1;
/// What the header reader calls each file in its texts.
pub const TEST_NOUN: &str = "a test file";
pub const FUNCTIONS_NOUN: &str = "a function file";

/// The actions, as a refusal lists them.
const ACTIONS: &str = "write, send, wait, await, read, check, call, script, branch, report, stop";

/// Read a test file.
pub fn parse_test(text: &str, file: &str) -> Result<TestFile, Problem> {
    let body = header(text, file, TEST_NOUN, TEST_FORMAT)?;
    let mut p = Parser::new(text, file, lex::lex(text, body));
    p.test_file()
}

/// Read a function file.
pub fn parse_functions(text: &str, file: &str) -> Result<FunctionFile, Problem> {
    let body = header(text, file, FUNCTIONS_NOUN, FUNCTIONS_FORMAT)?;
    let mut p = Parser::new(text, file, lex::lex(text, body));
    p.function_file()
}

/// Where the body begins: the first line that opens with the word `test` or `function`.
pub fn body_start(text: &str) -> usize {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        for word in ["test", "function"] {
            if let Some(rest) = t.strip_prefix(word)
                && !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            {
                return at;
            }
        }
        at += line.len();
    }
    text.len()
}

/// Check the header and return where the body begins.
fn header(text: &str, file: &str, noun: &str, format: &str) -> Result<usize, Problem> {
    let body = body_start(text);
    let head = &text[..body];
    veredas_core::schema::check(head, noun, format, VERSION).map_err(|e| Problem::at(file, text, 0, e.to_string()))?;
    // The header holds the stamp alone, as TOML itself reads it (its keys decoded, a table, a
    // dotted key, a quoted one with its escapes, a value of any width left as text): the first
    // top-level key that is neither `format` nor `version` is named, at its line.
    let table = toml::de::DeTable::parse(head).map_err(|e| Problem::at(file, text, 0, format!("not {noun}: {e}")))?;
    let other = table
        .get_ref()
        .iter()
        .filter(|(k, _)| k.get_ref() != "format" && k.get_ref() != "version")
        .min_by_key(|(k, _)| k.span().start);
    if let Some((key, _)) = other {
        let at = key.span().start.min(head.len());
        let line_start = head[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = &head[line_start..];
        let lead = line.len() - line.trim_start_matches(['\u{feff}', ' ', '\t']).len();
        // A control character in a key is shown by its escape, every other character as it is.
        let name: String = key.get_ref().chars().map(|c| if c.is_control() { c.escape_default().to_string() } else { c.to_string() }).collect();
        return Err(Problem::at(file, text, line_start + lead, format!("the header holds `format` and `version` alone, not `{name}`")));
    }
    Ok(body)
}

/// What a statement may be: a script box's or a function's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Script,
    Function,
}

struct Parser<'a> {
    src: &'a str,
    file: &'a str,
    toks: Vec<Token>,
    i: usize,
    /// Parentheses open: inside them a line's end is no end.
    paren: usize,
    /// How deep the expression being read nests (parentheses, calls, `not`, `-`).
    nest: usize,
    /// The operators of the outermost expression being read.
    ops: usize,
    /// Expressions open, one inside another (a string's `{ }` aside).
    exprs: usize,
    /// Reading a string's `{ }`: its end is the `}` that closes it, not the file's.
    inner: bool,
    /// How deep the blocks being read nest.
    blocks: usize,
}

/// How deep blocks and expressions (parentheses, calls, `not`, `-`) may nest, counted together,
/// and how many operators one expression may hold: past these the reading names the problem
/// rather than exhaust its stack.
pub const MAX_NEST: usize = 64;
pub const MAX_OPERATORS: usize = 1000;

type R<T> = Result<T, Problem>;

impl<'a> Parser<'a> {
    fn new(src: &'a str, file: &'a str, toks: Vec<Token>) -> Parser<'a> {
        Parser { src, file, toks, i: 0, paren: 0, nest: 0, ops: 0, exprs: 0, blocks: 0, inner: false }
    }

    // ---- tokens --------------------------------------------------------------------------------

    fn index(&self) -> usize {
        let mut i = self.i;
        if self.paren > 0 {
            while i + 1 < self.toks.len() && self.toks[i].tok == Tok::Newline {
                i += 1;
            }
        }
        i.min(self.toks.len() - 1)
    }

    fn peek(&self) -> &Token {
        &self.toks[self.index()]
    }

    fn peek_at(&self, n: usize) -> &Token {
        let mut i = self.index();
        for _ in 0..n {
            i += 1;
            if self.paren > 0 {
                while i + 1 < self.toks.len() && self.toks[i].tok == Tok::Newline {
                    i += 1;
                }
            }
        }
        &self.toks[i.min(self.toks.len() - 1)]
    }

    fn next(&mut self) -> Token {
        let i = self.index();
        let t = self.toks[i].clone();
        if t.tok != Tok::Eof {
            self.i = i + 1;
        } else {
            self.i = i;
        }
        t
    }

    /// The end of the last token taken.
    fn last_end(&self) -> usize {
        if self.i == 0 { self.toks[0].span.start } else { self.toks[self.i - 1].span.end }
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(&self.peek().tok, Tok::Name(n) if n == word)
    }

    fn is_sym(&self, sym: Sym) -> bool {
        self.peek().tok == Tok::Sym(sym)
    }

    fn eat_sym(&mut self, sym: Sym) -> bool {
        if self.is_sym(sym) {
            self.next();
            true
        } else {
            false
        }
    }

    fn skip_newlines(&mut self) {
        while self.toks[self.i].tok == Tok::Newline {
            self.i += 1;
        }
    }

    fn problem(&self, at: usize, message: impl Into<String>) -> Problem {
        Problem::at(self.file, self.src, at, message.into())
    }

    /// "`x` where `what` was expected", or the reason of a token the lexer could not read.
    fn expected(&self, tok: &Token, what: &str) -> Problem {
        match &tok.tok {
            Tok::Bad(reason) => self.problem(tok.span.start, reason.clone()),
            Tok::Eof if self.inner => self.problem(tok.span.start, format!("`}}` where {what} was expected")),
            _ => self.problem(tok.span.start, format!("{} where {what} was expected", tok.describe(self.src))),
        }
    }

    fn expect_sym(&mut self, sym: Sym, what: &str) -> R<Token> {
        if self.is_sym(sym) {
            Ok(self.next())
        } else {
            Err(self.expected(&self.peek().clone(), what))
        }
    }

    fn expect_word(&mut self, word: &str) -> R<Token> {
        if self.is_word(word) {
            Ok(self.next())
        } else {
            Err(self.expected(&self.peek().clone(), &format!("`{word}`")))
        }
    }

    /// A name of the test's own: not one of the language's words.
    fn name(&mut self, what: &str) -> R<Spanned<String>> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Name(n) if is_keyword(n) => {
                Err(self.problem(t.span.start, format!("`{n}` is a word of the language, not a name ({what})")))
            }
            Tok::Name(n) => {
                self.next();
                Ok(Spanned::new(n.clone(), t.span))
            }
            _ => Err(self.expected(&t, what)),
        }
    }

    /// A table's name: a name, or any name in backquotes.
    fn table(&mut self, what: &str) -> R<Spanned<String>> {
        if let Tok::Quoted(q) = &self.peek().tok {
            let q = q.clone();
            let t = self.next();
            return Ok(Spanned::new(q, t.span));
        }
        self.name(what)
    }

    /// The name after a `.`: any name, the language's words included.
    fn after_dot(&mut self, what: &str) -> R<Spanned<String>> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Name(n) | Tok::Quoted(n) => {
                self.next();
                Ok(Spanned::new(n.clone(), t.span))
            }
            _ => Err(self.expected(&t, what)),
        }
    }

    fn duration(&mut self) -> R<Spanned<i64>> {
        let t = self.peek().clone();
        match t.tok {
            Tok::Duration(ms) => {
                self.next();
                Ok(Spanned::new(ms, t.span))
            }
            _ => Err(self.expected(&t, "a duration (`5s`, `250ms`)")),
        }
    }

    /// The end of a line: a line's end taken, or the file's end or a closing brace left in place.
    fn line_end(&mut self) -> R<()> {
        match self.peek().tok {
            Tok::Newline => {
                self.next();
                Ok(())
            }
            Tok::Eof | Tok::Sym(Sym::RBrace) => Ok(()),
            _ => Err(self.expected(&self.peek().clone(), "a line's end")),
        }
    }

    /// A plain string: no `{` expression `}` in it.
    fn plain_string(&mut self, what: &str) -> R<Spanned<String>> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Str(pieces) => {
                self.next();
                let mut s = String::new();
                for p in pieces {
                    match p {
                        StrPiece::Text(x) => s.push_str(x),
                        StrPiece::Expr(_, span) => {
                            return Err(self.problem(span.start - 1, format!("{what} is plain text: no `{{` expression `}}` in it")));
                        }
                    }
                }
                Ok(Spanned::new(s, t.span))
            }
            _ => Err(self.expected(&t, what)),
        }
    }

    // ---- the test file -------------------------------------------------------------------------

    fn test_file(&mut self) -> R<TestFile> {
        self.skip_newlines();
        let head = self.expect_word("test")?.span;
        let title = self.plain_string("the test's title (a string)")?;
        self.expect_sym(Sym::LBrace, "`{`")?;
        self.line_end()?;
        let mut settings = Vec::new();
        loop {
            self.skip_newlines();
            match &self.peek().tok {
                Tok::Name(w) if SETTINGS.contains(&w.as_str()) => settings.push(self.setting()?),
                _ => break,
            }
        }
        let mut steps: Vec<Step> = Vec::new();
        loop {
            self.skip_newlines();
            let t = self.peek().clone();
            match &t.tok {
                Tok::Name(w) if w == "step" => steps.push(self.step()?),
                Tok::Sym(Sym::RBrace) => break,
                Tok::Name(w) if SETTINGS.contains(&w.as_str()) => {
                    return Err(self.problem(t.span.start, format!("`{w}` is a setting, and settings come before the first step")));
                }
                Tok::Name(w) if steps.is_empty() && !is_keyword(w) => {
                    return Err(self.problem(t.span.start, format!("`{w}` is no setting ({})", SETTINGS.join(", "))));
                }
                _ => return Err(self.expected(&t, "`step` or the test's closing `}`")),
            }
        }
        self.expect_sym(Sym::RBrace, "`}`")?;
        self.skip_newlines();
        let layout = if self.is_word("layout") { Some(self.layout()?) } else { None };
        self.skip_newlines();
        let t = self.peek().clone();
        if t.tok != Tok::Eof {
            let what = if layout.is_some() { "the file's end" } else { "the file's end or the `layout` block" };
            return Err(self.expected(&t, what));
        }
        Ok(TestFile { title, head, settings, steps, layout })
    }

    fn setting(&mut self) -> R<Setting> {
        let key = self.next();
        let key_span = key.span;
        let Tok::Name(word) = &key.tok else { unreachable!("a setting starts with its key") };
        let kind = match word.as_str() {
            "id" | "area" | "portal" | "use" => {
                let value = self.rest_of_line(word, key.span.end)?;
                match word.as_str() {
                    "id" => SettingKind::Id(value),
                    "area" => SettingKind::Area(value),
                    "portal" => SettingKind::Portal(value),
                    _ => SettingKind::Use(value),
                }
            }
            "modes" => {
                let mut modes = Vec::new();
                loop {
                    let t = self.peek().clone();
                    let mode = match &t.tok {
                        Tok::Name(m) if m == "live" => Mode::Live,
                        Tok::Name(m) if m == "replay" => Mode::Replay,
                        Tok::Name(m) => return Err(self.problem(t.span.start, format!("`{m}` is no mode (live, replay)"))),
                        _ => return Err(self.expected(&t, "a mode (live, replay)")),
                    };
                    self.next();
                    modes.push(Spanned::new(mode, t.span));
                    if !self.eat_sym(Sym::Comma) {
                        break;
                    }
                }
                self.line_end()?;
                SettingKind::Modes(modes)
            }
            "timeout" => {
                let d = self.duration()?;
                self.line_end()?;
                SettingKind::Timeout(d)
            }
            _ => {
                let name = self.name("a constant's name")?;
                self.expect_sym(Sym::Eq, "`=`")?;
                let value = self.expr()?;
                self.line_end()?;
                SettingKind::Let { name, value }
            }
        };
        Ok(Setting { key_span, kind })
    }

    /// The rest of a setting's line: trimmed, its comment and a surrounding pair of quotes
    /// taken off; the tokens of the line passed over.
    fn rest_of_line(&mut self, key: &str, from: usize) -> R<Spanned<String>> {
        let mut line_end = self.src[from..].find('\n').map(|i| from + i).unwrap_or(self.src.len());
        // A CRLF line ends at its carriage return, where its line-end token begins.
        if line_end > from && self.src.as_bytes()[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        let raw = &self.src[from..line_end];
        let mut cut = raw.len();
        let mut quoted = false;
        for (i, c) in raw.char_indices() {
            if c == '"' {
                quoted = !quoted;
            } else if c == '#' && !quoted {
                cut = i;
                break;
            }
        }
        let kept = &raw[..cut];
        let lead = kept.len() - kept.trim_start().len();
        let mut value = kept.trim();
        let mut start = from + lead;
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value = &value[1..value.len() - 1];
            start += 1;
        }
        if value.is_empty() {
            return Err(self.problem(from, format!("`{key}` has no value")));
        }
        let span = Span::new(start, start + value.len());
        while self.toks[self.i].tok != Tok::Eof && self.toks[self.i].span.start < line_end {
            self.i += 1;
        }
        self.line_end()?;
        Ok(Spanned::new(value.to_string(), span))
    }

    fn step(&mut self) -> R<Step> {
        let start = self.next().span.start;
        let name = self.name("a step's name")?;
        let mut actions = vec![self.action()?];
        self.line_end()?;
        let mut on = Vec::new();
        loop {
            self.skip_newlines();
            let t = self.peek().clone();
            match &t.tok {
                Tok::Name(w) if w == "step" => break,
                Tok::Sym(Sym::RBrace) | Tok::Eof => break,
                Tok::Name(w) if w == "on" => {
                    on.push(self.on_route()?);
                    self.line_end()?;
                }
                Tok::Name(w) if SETTINGS.contains(&w.as_str()) => {
                    return Err(self.problem(t.span.start, format!("`{w}` is a setting, and settings come before the first step")));
                }
                _ if !on.is_empty() => {
                    return Err(self.problem(t.span.start, "the actions come before the step's `on` lines"));
                }
                _ => {
                    if let Some(last) = actions.last()
                        && matches!(last.kind, ActionKind::Branch { .. } | ActionKind::Stop { .. })
                    {
                        return Err(self.problem(
                            t.span.start,
                            format!("`{}` is a step's last action: nothing follows it in step `{}`", last.keyword(), name.value),
                        ));
                    }
                    actions.push(self.action()?);
                    self.line_end()?;
                }
            }
        }
        let end = actions.iter().map(|a| a.span.end).chain(on.iter().map(|o: &OnRoute| o.span.end)).max().unwrap_or(start);
        Ok(Step { name, span: Span::new(start, end), actions, on })
    }

    fn action(&mut self) -> R<Action> {
        let t = self.peek().clone();
        let start = t.span.start;
        let Tok::Name(word) = &t.tok else {
            return Err(self.expected(&t, &format!("an action ({ACTIONS})")));
        };
        let kind = match word.as_str() {
            "write" => {
                self.next();
                let target = self.plant_ref()?;
                self.expect_sym(Sym::ColonEq, "`:=`")?;
                let value = self.expr()?;
                ActionKind::Write { target, value }
            }
            "send" => {
                self.next();
                let channel = self.table("a channel's name")?;
                self.expect_sym(Sym::Dot, "`.` and the message's name")?;
                let message = self.after_dot("a message's name")?;
                let fields = self.args()?;
                ActionKind::Send { channel, message, fields }
            }
            "wait" => {
                self.next();
                match self.wait()? {
                    StmtKind::Delay { duration } => ActionKind::Delay { duration },
                    StmtKind::Wait { condition, timeout } => ActionKind::Wait { condition, timeout },
                    _ => unreachable!("a wait is a wait or a delay"),
                }
            }
            "await" => {
                self.next();
                let table = self.table("a table's name")?;
                let (filter, timeout) = self.await_tail()?;
                self.expect_sym(Sym::Arrow, "`->` and the name the row is bound to")?;
                let bind = self.name("the name the row is bound to")?;
                ActionKind::Await { table, filter, timeout, bind }
            }
            "read" => {
                self.next();
                let mut refs = vec![self.plant_ref()?];
                while self.eat_sym(Sym::Comma) {
                    refs.push(self.plant_ref()?);
                }
                let arrow = self.expect_sym(Sym::Arrow, "`->` and the names the values go into")?;
                let into = self.names("a name the value goes into")?;
                if into.len() != refs.len() {
                    return Err(self.problem(
                        arrow.span.start,
                        format!("`read` reads {} value{} into {} name{}", refs.len(), plural(refs.len()), into.len(), plural(into.len())),
                    ));
                }
                ActionKind::Read { refs, into }
            }
            "check" => {
                self.next();
                let condition = self.expr()?;
                let label = if self.is_word("as") {
                    self.next();
                    Some(self.plain_string("a check's label (a string)")?)
                } else {
                    None
                };
                ActionKind::Check { condition, label }
            }
            "call" => {
                self.next();
                let function = self.name("a function's name")?;
                let args = self.args()?;
                let outputs = if self.eat_sym(Sym::Arrow) { self.names("a name an output goes into")? } else { Vec::new() };
                ActionKind::Call { function, args, outputs }
            }
            "script" => {
                self.next();
                let body = self.block(Context::Script)?;
                ActionKind::Script { body }
            }
            "branch" => {
                self.next();
                self.branch()?
            }
            "report" => {
                self.next();
                let mut items = vec![self.expr()?];
                while self.eat_sym(Sym::Comma) {
                    items.push(self.expr()?);
                }
                ActionKind::Report { items }
            }
            "stop" => {
                self.next();
                let verdict = self.verdict()?;
                let reason = if matches!(self.peek().tok, Tok::Str(_)) { Some(self.expr()?) } else { None };
                ActionKind::Stop { verdict, reason }
            }
            w if SETTINGS.contains(&w) => {
                return Err(self.problem(start, format!("`{w}` is a setting, and settings come before the first step")));
            }
            _ => return Err(self.problem(start, format!("`{word}` is no action ({ACTIONS})"))),
        };
        Ok(Action { kind, span: Span::new(start, self.last_end()) })
    }

    fn verdict(&mut self) -> R<Verdict> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Name(w) if w == "pass" => {
                self.next();
                Ok(Verdict::Pass)
            }
            Tok::Name(w) if w == "fail" => {
                self.next();
                Ok(Verdict::Fail)
            }
            _ => Err(self.expected(&t, "`pass` or `fail`")),
        }
    }

    /// After `wait`: a duration alone, or a condition and its optional timeout.
    fn wait(&mut self) -> R<StmtKind> {
        let condition = self.expr()?;
        if let ExprKind::Duration(ms) = condition.kind
            && !self.is_word("timeout")
        {
            return Ok(StmtKind::Delay { duration: Spanned::new(ms, condition.span) });
        }
        let timeout = if self.is_word("timeout") {
            self.next();
            Some(self.duration()?)
        } else {
            None
        };
        Ok(StmtKind::Wait { condition, timeout })
    }

    /// After an await's table: `where <condition>` and `timeout <duration>`, each optional.
    fn await_tail(&mut self) -> R<(Option<Expr>, Option<Spanned<i64>>)> {
        let filter = if self.is_word("where") {
            self.next();
            Some(self.expr()?)
        } else {
            None
        };
        let timeout = if self.is_word("timeout") {
            self.next();
            Some(self.duration()?)
        } else {
            None
        };
        Ok((filter, timeout))
    }

    fn plant_ref(&mut self) -> R<PlantRef> {
        let table = self.table("a table's name")?;
        self.expect_sym(Sym::Dot, "`.` and a column's name")?;
        let column = self.after_dot("a column's name")?;
        Ok(PlantRef { table, column })
    }

    fn names(&mut self, what: &str) -> R<Vec<Spanned<String>>> {
        let mut out = vec![self.name(what)?];
        while self.eat_sym(Sym::Comma) {
            out.push(self.name(what)?);
        }
        Ok(out)
    }

    /// `(<name>: <expression>, ...)`.
    fn args(&mut self) -> R<Vec<Arg>> {
        self.expect_sym(Sym::LParen, "`(`")?;
        self.paren += 1;
        let mut out = Vec::new();
        if !self.is_sym(Sym::RParen) {
            loop {
                let name = self.table("a name and `:`")?;
                self.expect_sym(Sym::Colon, "`:`")?;
                let value = self.expr()?;
                out.push(Arg { name, value });
                if !self.eat_sym(Sym::Comma) {
                    break;
                }
            }
        }
        self.expect_sym(Sym::RParen, "`)`")?;
        self.paren -= 1;
        Ok(out)
    }

    fn branch(&mut self) -> R<ActionKind> {
        let value = self.expr()?;
        self.expect_sym(Sym::LBrace, "`{` and the branch's arms")?;
        self.line_end()?;
        let mut arms = Vec::new();
        let mut otherwise: Option<Spanned<Route>> = None;
        loop {
            self.skip_newlines();
            let t = self.peek().clone();
            if t.tok == Tok::Sym(Sym::RBrace) {
                self.next();
                break;
            }
            if otherwise.is_some() {
                return Err(self.problem(t.span.start, "`else` is a branch's last arm"));
            }
            if self.is_word("else") {
                self.next();
                self.expect_sym(Sym::Arrow, "`->`")?;
                let route = self.route()?;
                otherwise = Some(route);
            } else {
                let mut patterns = vec![self.pattern()?];
                while self.eat_sym(Sym::Comma) {
                    patterns.push(self.pattern()?);
                }
                self.expect_sym(Sym::Arrow, "`->`")?;
                let route = self.route()?;
                arms.push(Arm { patterns, span: Span::new(t.span.start, route.span.end), route });
            }
            self.line_end()?;
        }
        if arms.is_empty() && otherwise.is_none() {
            return Err(self.problem(self.last_end() - 1, "a branch has at least one arm"));
        }
        Ok(ActionKind::Branch { value, arms, otherwise })
    }

    fn pattern(&mut self) -> R<Pattern> {
        let op = match self.peek().tok {
            Tok::Sym(Sym::EqEq) => Some(CmpOp::Eq),
            Tok::Sym(Sym::NotEq) => Some(CmpOp::Ne),
            Tok::Sym(Sym::Lt) => Some(CmpOp::Lt),
            Tok::Sym(Sym::Le) => Some(CmpOp::Le),
            Tok::Sym(Sym::Gt) => Some(CmpOp::Gt),
            Tok::Sym(Sym::Ge) => Some(CmpOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            return Ok(Pattern::Compare { op, value: self.sum()? });
        }
        let low = self.sum()?;
        if self.eat_sym(Sym::DotDot) {
            let high = self.sum()?;
            return Ok(Pattern::Range { low, high });
        }
        Ok(Pattern::Value(low))
    }

    fn route(&mut self) -> R<Spanned<Route>> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Name(w) if w == "retry" => {
                self.next();
                let step = self.name("the step to retry")?;
                self.expect_word("max")?;
                let n = self.peek().clone();
                let max = match n.tok {
                    Tok::Int(v) if (0..=u32::MAX as i64).contains(&v) => {
                        self.next();
                        Spanned::new(v as u32, n.span)
                    }
                    _ => return Err(self.expected(&n, "a whole number of retries")),
                };
                Ok(Spanned::new(Route::Retry { step, max }, Span::new(t.span.start, self.last_end())))
            }
            Tok::Name(w) if w == "stop" => {
                self.next();
                let v = self.verdict()?;
                Ok(Spanned::new(Route::Stop(v), Span::new(t.span.start, self.last_end())))
            }
            _ => {
                let step = self.name("a step's name, `retry` or `stop`")?;
                let span = step.span;
                Ok(Spanned::new(Route::Step(step), span))
            }
        }
    }

    fn on_route(&mut self) -> R<OnRoute> {
        let start = self.next().span.start;
        let mut outcomes = Vec::new();
        loop {
            let t = self.peek().clone();
            let o = match &t.tok {
                Tok::Name(w) if w == "pass" => Outcome::Pass,
                Tok::Name(w) if w == "fail" => Outcome::Fail,
                Tok::Name(w) if w == "timeout" => Outcome::Timeout,
                Tok::Name(w) if w == "error" => Outcome::Error,
                Tok::Name(w) => return Err(self.problem(t.span.start, format!("`{w}` is no outcome (pass, fail, timeout, error)"))),
                _ => return Err(self.expected(&t, "an outcome (pass, fail, timeout, error)")),
            };
            self.next();
            outcomes.push(Spanned::new(o, t.span));
            if !self.eat_sym(Sym::Comma) {
                break;
            }
        }
        self.expect_sym(Sym::Arrow, "`->`")?;
        let route = self.route()?;
        Ok(OnRoute { outcomes, span: Span::new(start, route.span.end), route })
    }

    fn layout(&mut self) -> R<Layout> {
        let start = self.next().span.start;
        self.expect_sym(Sym::LBrace, "`{`")?;
        let mut positions = Vec::new();
        loop {
            self.skip_newlines();
            if self.is_sym(Sym::RBrace) {
                let end = self.next().span.end;
                return Ok(Layout { span: Span::new(start, end), positions });
            }
            let step = self.name("a step's name or the layout's closing `}`")?;
            let column = self.grid()?;
            self.expect_sym(Sym::Comma, "`,`")?;
            let row = self.grid()?;
            positions.push(Position { step, column, row });
        }
    }

    fn grid(&mut self) -> R<u32> {
        let t = self.peek().clone();
        match t.tok {
            Tok::Int(v) if (0..=u32::MAX as i64).contains(&v) => {
                self.next();
                Ok(v as u32)
            }
            _ => Err(self.expected(&t, "a whole number from 0")),
        }
    }

    // ---- the function file ---------------------------------------------------------------------

    fn function_file(&mut self) -> R<FunctionFile> {
        let mut functions = Vec::new();
        loop {
            self.skip_newlines();
            if self.peek().tok == Tok::Eof {
                break;
            }
            functions.push(self.function()?);
        }
        if functions.is_empty() {
            return Err(self.expected(&self.peek().clone(), "`function`"));
        }
        Ok(FunctionFile { functions })
    }

    fn function(&mut self) -> R<Function> {
        let head = self.expect_word("function")?;
        let description = description_above(self.src, head.span.start);
        let name = self.name("a function's name")?;
        if is_builtin(&name.value) {
            return Err(self.problem(name.span.start, format!("`{}` is a built-in call, not a function's name", name.value)));
        }
        let inputs = self.params()?;
        let outputs = if self.eat_sym(Sym::Arrow) { self.params()? } else { Vec::new() };
        self.expect_sym(Sym::LBrace, "`{`")?;
        self.line_end()?;
        let mut reads: Option<Vec<Spanned<String>>> = None;
        let mut timeout: Option<Spanned<i64>> = None;
        loop {
            self.skip_newlines();
            let t = self.peek().clone();
            if self.is_word("reads") {
                self.next();
                if reads.is_some() {
                    return Err(self.problem(t.span.start, "`reads` is given twice"));
                }
                let mut tables = vec![self.table("a table's name")?];
                while self.eat_sym(Sym::Comma) {
                    tables.push(self.table("a table's name")?);
                }
                reads = Some(tables);
                self.line_end()?;
            } else if self.is_word("timeout") {
                self.next();
                if timeout.is_some() {
                    return Err(self.problem(t.span.start, "`timeout` is given twice"));
                }
                timeout = Some(self.duration()?);
                self.line_end()?;
            } else {
                break;
            }
        }
        let body = self.statements(Context::Function)?;
        let end = self.expect_sym(Sym::RBrace, "`}`")?.span.end;
        Ok(Function {
            name,
            description,
            inputs,
            outputs,
            reads: reads.unwrap_or_default(),
            timeout,
            body,
            span: Span::new(head.span.start, end),
        })
    }

    fn params(&mut self) -> R<Vec<Param>> {
        self.expect_sym(Sym::LParen, "`(`")?;
        self.paren += 1;
        let mut out = Vec::new();
        if !self.is_sym(Sym::RParen) {
            loop {
                let name = self.name("a port's name")?;
                self.expect_sym(Sym::Colon, "`:` and a type")?;
                let t = self.peek().clone();
                let ty = match &t.tok {
                    Tok::Name(w) => match Type::parse(w) {
                        Some(ty) => ty,
                        None => {
                            return Err(self.problem(t.span.start, format!("`{w}` is no type (int, float, bool, text, time, duration)")));
                        }
                    },
                    _ => return Err(self.expected(&t, "a type (int, float, bool, text, time, duration)")),
                };
                self.next();
                out.push(Param { name, ty: Spanned::new(ty, t.span) });
                if !self.eat_sym(Sym::Comma) {
                    break;
                }
            }
        }
        self.expect_sym(Sym::RParen, "`)`")?;
        self.paren -= 1;
        Ok(out)
    }

    // ---- statements ----------------------------------------------------------------------------

    fn block(&mut self, ctx: Context) -> R<Vec<Stmt>> {
        let open = self.expect_sym(Sym::LBrace, "`{`")?;
        self.blocks += 1;
        if self.blocks + self.nest > MAX_NEST {
            return Err(self.problem(open.span.start, format!("blocks and expressions nested deeper than {MAX_NEST}, counted together")));
        }
        let body = self.statements(ctx)?;
        self.expect_sym(Sym::RBrace, "`}`")?;
        self.blocks -= 1;
        Ok(body)
    }

    /// One level deeper into an expression, refused past the limit.
    fn deeper(&mut self, at: usize) -> R<()> {
        self.nest += 1;
        if self.blocks + self.nest > MAX_NEST {
            return Err(self.problem(at, format!("blocks and expressions nested deeper than {MAX_NEST}, counted together")));
        }
        Ok(())
    }

    /// One operator more in the outermost expression, refused past the limit.
    fn operator(&mut self) -> R<()> {
        self.ops += 1;
        if self.ops > MAX_OPERATORS {
            return Err(self.problem(self.peek().span.start, format!("an expression of more than {MAX_OPERATORS} operators")));
        }
        Ok(())
    }

    /// Statements up to a closing brace, which is left in place.
    fn statements(&mut self, ctx: Context) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            while matches!(self.toks[self.i].tok, Tok::Newline | Tok::Sym(Sym::Semi)) {
                self.i += 1;
            }
            match self.peek().tok {
                Tok::Sym(Sym::RBrace) => return Ok(out),
                Tok::Eof => return Err(self.expected(&self.peek().clone(), "`}`")),
                _ => {}
            }
            out.push(self.statement(ctx)?);
            match self.peek().tok {
                Tok::Newline | Tok::Sym(Sym::Semi) | Tok::Sym(Sym::RBrace) => {}
                _ => return Err(self.expected(&self.peek().clone(), "a line's end")),
            }
        }
    }

    fn statement(&mut self, ctx: Context) -> R<Stmt> {
        let t = self.peek().clone();
        let start = t.span.start;
        let kind = match &t.tok {
            Tok::Name(w) if w == "if" => self.if_stmt(ctx)?,
            Tok::Name(w) if w == "for" => {
                self.next();
                let var = self.name("the loop's name")?;
                self.expect_word("in")?;
                let from = self.sum()?;
                self.expect_sym(Sym::DotDot, "`..`")?;
                let to = self.sum()?;
                let body = self.block(ctx)?;
                StmtKind::For { var, from, to, body }
            }
            Tok::Name(w) if w == "fail" => {
                self.next();
                let reason = match self.peek().tok {
                    Tok::Newline | Tok::Sym(Sym::Semi) | Tok::Sym(Sym::RBrace) | Tok::Eof => None,
                    _ => Some(self.expr()?),
                };
                StmtKind::Fail { reason }
            }
            Tok::Name(w) if w == "wait" => {
                self.next();
                self.wait()?
            }
            Tok::Name(w) if w == "await" => {
                return Err(self.problem(start, "an `await` is assigned: `<name> = await <table> ...`"));
            }
            Tok::Name(w) if (w == "write" || w == "send") && ctx == Context::Function => {
                return Err(self.problem(start, format!("`{w}` is not a statement: a function only reads")));
            }
            Tok::Name(w) if ["write", "send", "read", "check", "call", "script", "branch", "report", "stop"].contains(&w.as_str()) => {
                return Err(self.problem(start, format!("`{w}` is a step's action, not a statement")));
            }
            Tok::Name(w) if !is_keyword(w) => self.assignment()?,
            _ => return Err(self.expected(&t, "a statement")),
        };
        Ok(Stmt { kind, span: Span::new(start, self.last_end()) })
    }

    fn if_stmt(&mut self, ctx: Context) -> R<StmtKind> {
        self.next();
        let mut arms = Vec::new();
        let cond = self.expr()?;
        arms.push((cond, self.block(ctx)?));
        let mut otherwise = None;
        loop {
            // `else` may open the next line.
            let mut j = self.i;
            while self.toks[j].tok == Tok::Newline {
                j += 1;
            }
            if !matches!(&self.toks[j].tok, Tok::Name(w) if w == "else") {
                break;
            }
            self.i = j + 1;
            if self.is_word("if") {
                self.next();
                let cond = self.expr()?;
                arms.push((cond, self.block(ctx)?));
            } else {
                otherwise = Some(self.block(ctx)?);
                break;
            }
        }
        Ok(StmtKind::If { arms, otherwise })
    }

    /// A statement that opens with a name: an assignment, an await, a call.
    fn assignment(&mut self) -> R<StmtKind> {
        let first = self.name("a name")?;
        if self.is_sym(Sym::Comma) {
            let mut outputs = vec![first];
            while self.eat_sym(Sym::Comma) {
                outputs.push(self.name("a name an output goes into")?);
            }
            self.expect_sym(Sym::Eq, "`=` and a function's call")?;
            let function = self.name("a function's name")?;
            let args = self.args()?;
            return Ok(StmtKind::Call { outputs, function, args });
        }
        let op = match self.peek().tok {
            Tok::Sym(Sym::Eq) => AssignOp::Set,
            Tok::Sym(Sym::PlusEq) => AssignOp::Add,
            Tok::Sym(Sym::MinusEq) => AssignOp::Sub,
            Tok::Sym(Sym::StarEq) => AssignOp::Mul,
            Tok::Sym(Sym::SlashEq) => AssignOp::Div,
            _ => return Err(self.expected(&self.peek().clone(), "`=`, `+=`, `-=`, `*=` or `/=`")),
        };
        self.next();
        if op == AssignOp::Set {
            if self.is_word("await") {
                self.next();
                let table = self.table("a table's name")?;
                let (filter, timeout) = self.await_tail()?;
                return Ok(StmtKind::Await { bind: first, table, filter, timeout });
            }
            if let Tok::Name(f) = &self.peek().tok
                && !is_keyword(f)
                && !is_builtin(f)
                && self.peek_at(1).tok == Tok::Sym(Sym::LParen)
                && self.named_inputs()
            {
                let function = self.name("a function's name")?;
                let args = self.args()?;
                return Ok(StmtKind::Call { outputs: vec![first], function, args });
            }
        }
        let value = self.expr()?;
        Ok(StmtKind::Assign { target: first, op, value })
    }

    /// After a name and its `(`: whether what follows is `)` or a name and `:` (a function's
    /// named inputs, or none), line ends inside the parentheses passed over.
    fn named_inputs(&self) -> bool {
        let after = |mut j: usize| {
            j += 1;
            while j + 1 < self.toks.len() && self.toks[j].tok == Tok::Newline {
                j += 1;
            }
            j
        };
        let open = after(self.index());
        let first = after(open);
        match &self.toks[first].tok {
            Tok::Sym(Sym::RParen) => true,
            Tok::Name(_) | Tok::Quoted(_) => self.toks[after(first)].tok == Tok::Sym(Sym::Colon),
            _ => false,
        }
    }

    // ---- expressions ---------------------------------------------------------------------------

    fn expr(&mut self) -> R<Expr> {
        self.open();
        let mut left = self.and_expr()?;
        while self.is_word("or") {
            self.operator()?;
            self.next();
            let right = self.and_expr()?;
            left = binary(BinOp::Or, left, right);
        }
        self.exprs -= 1;
        Ok(left)
    }

    /// An expression opens: the outermost one counts its operators afresh.
    fn open(&mut self) {
        if self.exprs == 0 {
            self.ops = 0;
        }
        self.exprs += 1;
    }

    /// An expression read at the additive level as an outermost one (a pattern, a range's end).
    fn sum(&mut self) -> R<Expr> {
        self.open();
        let e = self.additive()?;
        self.exprs -= 1;
        Ok(e)
    }

    fn and_expr(&mut self) -> R<Expr> {
        let mut left = self.not_expr()?;
        while self.is_word("and") {
            self.operator()?;
            self.next();
            let right = self.not_expr()?;
            left = binary(BinOp::And, left, right);
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> R<Expr> {
        if self.is_word("not") {
            let start = self.next().span.start;
            self.deeper(start)?;
            let inner = self.not_expr()?;
            self.nest -= 1;
            let span = Span::new(start, inner.span.end);
            return Ok(Expr { kind: ExprKind::Unary { op: UnOp::Not, expr: Box::new(inner) }, span });
        }
        self.comparison()
    }

    fn cmp_op(&self) -> Option<CmpOp> {
        match self.peek().tok {
            Tok::Sym(Sym::EqEq) => Some(CmpOp::Eq),
            Tok::Sym(Sym::NotEq) => Some(CmpOp::Ne),
            Tok::Sym(Sym::Lt) => Some(CmpOp::Lt),
            Tok::Sym(Sym::Le) => Some(CmpOp::Le),
            Tok::Sym(Sym::Gt) => Some(CmpOp::Gt),
            Tok::Sym(Sym::Ge) => Some(CmpOp::Ge),
            _ => None,
        }
    }

    fn comparison(&mut self) -> R<Expr> {
        let left = self.additive()?;
        let Some(op) = self.cmp_op() else { return Ok(left) };
        self.operator()?;
        self.next();
        let right = self.additive()?;
        if self.cmp_op().is_some() {
            return Err(self.problem(self.peek().span.start, "two comparisons in a row: join them with `and`"));
        }
        Ok(binary(BinOp::Cmp(op), left, right))
    }

    fn additive(&mut self) -> R<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek().tok {
                Tok::Sym(Sym::Plus) => BinOp::Add,
                Tok::Sym(Sym::Minus) => BinOp::Sub,
                _ => return Ok(left),
            };
            self.operator()?;
            self.next();
            let right = self.multiplicative()?;
            left = binary(op, left, right);
        }
    }

    fn multiplicative(&mut self) -> R<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek().tok {
                Tok::Sym(Sym::Star) => BinOp::Mul,
                Tok::Sym(Sym::Slash) => BinOp::Div,
                Tok::Sym(Sym::Percent) => BinOp::Rem,
                _ => return Ok(left),
            };
            self.operator()?;
            self.next();
            let right = self.unary()?;
            left = binary(op, left, right);
        }
    }

    fn unary(&mut self) -> R<Expr> {
        if self.is_sym(Sym::Minus) {
            let start = self.next().span.start;
            self.deeper(start)?;
            let inner = self.unary()?;
            self.nest -= 1;
            let span = Span::new(start, inner.span.end);
            return Ok(Expr { kind: ExprKind::Unary { op: UnOp::Neg, expr: Box::new(inner) }, span });
        }
        self.primary()
    }

    fn primary(&mut self) -> R<Expr> {
        let t = self.peek().clone();
        let kind = match &t.tok {
            Tok::Int(v) => ExprKind::Int(*v),
            Tok::Float(v) => ExprKind::Float(*v),
            Tok::Duration(v) => ExprKind::Duration(*v),
            Tok::Str(pieces) => {
                let mut parts = Vec::new();
                for p in pieces {
                    match p {
                        StrPiece::Text(s) => parts.push(StrPart::Text(s.clone())),
                        StrPiece::Expr(tokens, span) => {
                            let e = self.interpolated(tokens, *span)?;
                            parts.push(StrPart::Expr(e));
                        }
                    }
                }
                ExprKind::Str(parts)
            }
            Tok::Name(w) if w == "true" => ExprKind::Bool(true),
            Tok::Name(w) if w == "false" => ExprKind::Bool(false),
            Tok::Name(w) if is_keyword(w) => {
                return Err(self.problem(t.span.start, format!("`{w}` where a value was expected")));
            }
            Tok::Name(w) => {
                self.next();
                let name = Spanned::new(w.clone(), t.span);
                if self.is_sym(Sym::LParen) {
                    let open = self.next().span.start;
                    self.deeper(open)?;
                    self.paren += 1;
                    let mut args = Vec::new();
                    if !self.is_sym(Sym::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_sym(Sym::Comma) {
                                break;
                            }
                        }
                    }
                    let end = self.expect_sym(Sym::RParen, "`)`")?.span.end;
                    self.paren -= 1;
                    self.nest -= 1;
                    return Ok(Expr { kind: ExprKind::Call { name, args }, span: Span::new(t.span.start, end) });
                }
                if self.eat_sym(Sym::Dot) {
                    let field = self.after_dot("a column's name")?;
                    self.no_second_dot(t.span.start)?;
                    let span = Span::new(t.span.start, field.span.end);
                    return Ok(Expr { kind: ExprKind::Field { base: name, field }, span });
                }
                return Ok(Expr { kind: ExprKind::Name(w.clone()), span: t.span });
            }
            Tok::Quoted(q) => {
                self.next();
                let table = Spanned::new(q.clone(), t.span);
                self.expect_sym(Sym::Dot, "`.` and a column's name (a table in backquotes is read by its column)")?;
                let column = self.after_dot("a column's name")?;
                self.no_second_dot(t.span.start)?;
                let span = Span::new(t.span.start, column.span.end);
                return Ok(Expr { kind: ExprKind::Plant { table, column }, span });
            }
            Tok::Sym(Sym::LParen) => {
                self.next();
                self.deeper(t.span.start)?;
                self.paren += 1;
                let inner = self.expr()?;
                let end = self.expect_sym(Sym::RParen, "`)`")?.span.end;
                self.paren -= 1;
                self.nest -= 1;
                return Ok(Expr { kind: inner.kind, span: Span::new(t.span.start, end) });
            }
            _ => return Err(self.expected(&t, "a value")),
        };
        self.next();
        Ok(Expr { kind, span: t.span })
    }

    fn no_second_dot(&self, start: usize) -> R<()> {
        if self.is_sym(Sym::Dot) {
            let end = self.toks[self.index() + 1].span.end;
            return Err(self.problem(start, format!("`{}`: a name reads one column", &self.src[start..end])));
        }
        Ok(())
    }

    /// The expression of a string's `{ }`.
    fn interpolated(&mut self, tokens: &[Token], span: Span) -> R<Expr> {
        let mut toks = tokens.to_vec();
        toks.push(Token { tok: Tok::Eof, span: Span::new(span.end, span.end) });
        let mut p = Parser::new(self.src, self.file, toks);
        // The limits count on from the expression the string sits in.
        p.nest = self.nest;
        p.ops = self.ops;
        p.exprs = self.exprs;
        p.blocks = self.blocks;
        p.inner = true;
        if p.peek().tok == Tok::Eof {
            return Err(self.problem(span.start - 1, "a `{` in a string holds an expression (a `{` alone is written `\\{`)"));
        }
        let e = p.expr()?;
        let t = p.peek().clone();
        if t.tok != Tok::Eof {
            return Err(p.expected(&t, "the `}` closing the expression"));
        }
        self.ops = p.ops;
        Ok(e)
    }
}

fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    let span = left.span.to(right.span);
    Expr { kind: ExprKind::Binary { op, left: Box::new(left), right: Box::new(right) }, span }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The comment lines right above the line where `at` sits, `#` and one space taken off each.
fn description_above(src: &str, at: usize) -> String {
    let line_start = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut lines: Vec<&str> = Vec::new();
    let mut end = line_start;
    while end > 0 {
        let prev_start = src[..end - 1].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = src[prev_start..end - 1].trim_end_matches('\r').trim();
        match line.strip_prefix('#') {
            Some(c) => lines.push(c.strip_prefix(' ').unwrap_or(c)),
            None => break,
        }
        end = prev_start;
    }
    lines.reverse();
    lines.join("\n")
}
