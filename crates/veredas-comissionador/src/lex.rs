//! The tokens of a test file or a function file (`docs/test-schema.md`, "The file, line by line").
//!
//! The lexer never fails: a character the language has no use for, an unclosed string or a
//! number run into letters becomes a [`Tok::Bad`] token carrying its reason, and the parser
//! reports it where it meets it. A line's end is a token ([`Tok::Newline`]); runs of them are
//! one, and a line whose first symbol is `->` continues the line before it, so the newline
//! before it is dropped. Every token carries its byte range in the whole file.

use crate::model::Span;

/// The language's own words: none of them names a step, a variable, a table or a function
/// (`docs/test-schema.md`, "Names"). After a `.` any name is a column's or a message's. `max`
/// is not among them: it is a built-in call, and `retry <step> max <n>` reads it by its place.
pub const KEYWORDS: &[&str] = &[
    "and", "as", "await", "branch", "call", "check", "else", "error", "fail", "false", "for", "function", "if", "in",
    "layout", "not", "on", "or", "pass", "read", "reads", "report", "retry", "script", "send", "step", "stop",
    "test", "timeout", "true", "wait", "where", "write",
];

/// Whether a word is one of the language's own.
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// A symbol of the language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sym {
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Dot,
    DotDot,
    Colon,
    ColonEq,
    Semi,
    Arrow,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
}

impl Sym {
    pub fn text(self) -> &'static str {
        match self {
            Sym::LParen => "(",
            Sym::RParen => ")",
            Sym::LBrace => "{",
            Sym::RBrace => "}",
            Sym::Comma => ",",
            Sym::Dot => ".",
            Sym::DotDot => "..",
            Sym::Colon => ":",
            Sym::ColonEq => ":=",
            Sym::Semi => ";",
            Sym::Arrow => "->",
            Sym::Eq => "=",
            Sym::PlusEq => "+=",
            Sym::MinusEq => "-=",
            Sym::StarEq => "*=",
            Sym::SlashEq => "/=",
            Sym::EqEq => "==",
            Sym::NotEq => "!=",
            Sym::Lt => "<",
            Sym::Le => "<=",
            Sym::Gt => ">",
            Sym::Ge => ">=",
            Sym::Plus => "+",
            Sym::Minus => "-",
            Sym::Star => "*",
            Sym::Slash => "/",
            Sym::Percent => "%",
        }
    }
}

/// A piece of a string: text, or an interpolated expression's tokens (`{` and `}` left out).
#[derive(Clone, Debug, PartialEq)]
pub enum StrPiece {
    Text(String),
    Expr(Vec<Token>, Span),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// A name or one of the language's words.
    Name(String),
    /// A name in backquotes: always a name, never one of the language's words.
    Quoted(String),
    Int(i64),
    Float(f64),
    /// A duration in whole milliseconds.
    Duration(i64),
    Str(Vec<StrPiece>),
    Sym(Sym),
    Newline,
    Eof,
    /// Something the language cannot read, with the reason.
    Bad(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Token {
    /// The token as a problem's text names it.
    pub fn describe(&self, src: &str) -> String {
        match &self.tok {
            Tok::Newline => "a line's end".to_string(),
            Tok::Eof => "the file's end".to_string(),
            Tok::Str(_) => "a string".to_string(),
            Tok::Bad(_) => "something unreadable".to_string(),
            _ => format!("`{}`", &src[self.span.start..self.span.end]),
        }
    }
}

/// The tokens of `src[from..]`, places counted in the whole of `src`, ending in [`Tok::Eof`].
pub fn lex(src: &str, from: usize) -> Vec<Token> {
    let mut out = Lexer { src, pos: from, end: src.len() }.run();
    tidy(&mut out);
    out
}

/// A line's ends: runs of them one, none first, none right before `->`.
fn tidy(tokens: &mut Vec<Token>) {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for t in tokens.drain(..) {
        match t.tok {
            Tok::Newline if out.is_empty() || matches!(out.last().map(|l| &l.tok), Some(Tok::Newline)) => {}
            Tok::Sym(Sym::Arrow) => {
                if matches!(out.last().map(|l| &l.tok), Some(Tok::Newline)) {
                    out.pop();
                }
                out.push(t);
            }
            _ => out.push(t),
        }
    }
    *tokens = out;
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    end: usize,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..self.end].chars().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut it = self.src[self.pos..self.end].chars();
        it.next();
        it.next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn run(&mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            // Blanks and comments.
            while let Some(c) = self.peek() {
                if c == ' ' || c == '\t' || c == '\r' {
                    self.bump();
                } else if c == '#' {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                } else {
                    break;
                }
            }
            let start = self.pos;
            let Some(c) = self.bump() else {
                out.push(Token { tok: Tok::Eof, span: Span::new(start, start) });
                return out;
            };
            let tok = match c {
                '\n' => Tok::Newline,
                '"' => self.string(),
                '`' => self.quoted(),
                c if c.is_ascii_digit() => self.number(start),
                c if c.is_ascii_alphabetic() || c == '_' => {
                    while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
                        self.bump();
                    }
                    Tok::Name(self.src[start..self.pos].to_string())
                }
                _ => self.symbol(c),
            };
            out.push(Token { tok, span: Span::new(start, self.pos) });
        }
    }

    fn symbol(&mut self, c: char) -> Tok {
        let next = self.peek();
        let two = |l: &mut Lexer, s: Sym| {
            l.bump();
            Tok::Sym(s)
        };
        match (c, next) {
            ('.', Some('.')) => two(self, Sym::DotDot),
            (':', Some('=')) => two(self, Sym::ColonEq),
            ('-', Some('>')) => two(self, Sym::Arrow),
            ('=', Some('=')) => two(self, Sym::EqEq),
            ('!', Some('=')) => two(self, Sym::NotEq),
            ('<', Some('=')) => two(self, Sym::Le),
            ('>', Some('=')) => two(self, Sym::Ge),
            ('+', Some('=')) => two(self, Sym::PlusEq),
            ('-', Some('=')) => two(self, Sym::MinusEq),
            ('*', Some('=')) => two(self, Sym::StarEq),
            ('/', Some('=')) => two(self, Sym::SlashEq),
            ('(', _) => Tok::Sym(Sym::LParen),
            (')', _) => Tok::Sym(Sym::RParen),
            ('{', _) => Tok::Sym(Sym::LBrace),
            ('}', _) => Tok::Sym(Sym::RBrace),
            (',', _) => Tok::Sym(Sym::Comma),
            ('.', _) => Tok::Sym(Sym::Dot),
            (':', _) => Tok::Sym(Sym::Colon),
            (';', _) => Tok::Sym(Sym::Semi),
            ('=', _) => Tok::Sym(Sym::Eq),
            ('<', _) => Tok::Sym(Sym::Lt),
            ('>', _) => Tok::Sym(Sym::Gt),
            ('+', _) => Tok::Sym(Sym::Plus),
            ('-', _) => Tok::Sym(Sym::Minus),
            ('*', _) => Tok::Sym(Sym::Star),
            ('/', _) => Tok::Sym(Sym::Slash),
            ('%', _) => Tok::Sym(Sym::Percent),
            (c, _) => Tok::Bad(format!("`{c}` is no part of the language")),
        }
    }

    fn quoted(&mut self) -> Tok {
        let from = self.pos;
        while let Some(c) = self.peek() {
            if c == '`' {
                let name = self.src[from..self.pos].to_string();
                self.bump();
                if name.trim().is_empty() || name.trim() != name {
                    return Tok::Bad("a name in backquotes is the name alone, not blank, no space around it".to_string());
                }
                return Tok::Quoted(name);
            }
            if c == '\n' {
                break;
            }
            self.bump();
        }
        Tok::Bad("a backquote not closed on its line".to_string())
    }

    /// A whole number, a number with a fraction or an exponent, or a duration: a number and its
    /// unit with no space between.
    fn number(&mut self, start: usize) -> Tok {
        let digits = |l: &mut Lexer| {
            while matches!(l.peek(), Some(c) if c.is_ascii_digit() || c == '_') {
                l.bump();
            }
        };
        digits(self);
        let mut float = false;
        if self.peek() == Some('.') && matches!(self.peek2(), Some(c) if c.is_ascii_digit()) {
            float = true;
            self.bump();
            digits(self);
        }
        if matches!(self.peek(), Some('e' | 'E'))
            && (matches!(self.peek2(), Some(c) if c.is_ascii_digit())
                || (matches!(self.peek2(), Some('+' | '-')) && {
                    let rest = &self.src[self.pos..self.end];
                    rest.chars().nth(2).is_some_and(|c| c.is_ascii_digit())
                }))
        {
            float = true;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            digits(self);
        }
        let number_end = self.pos;
        let text: String = self.src[start..number_end].chars().filter(|c| *c != '_').collect();
        if self.src[start..number_end].ends_with('_') || self.src[start..number_end].contains("__") {
            return Tok::Bad(format!("`{}`: a `_` stands between two digits", &self.src[start..number_end]));
        }
        // A unit, or letters run into the number.
        if matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c == '_') {
            let unit_start = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
                self.bump();
            }
            let unit = &self.src[unit_start..self.pos];
            let per = match unit {
                "ms" => 1.0,
                "s" => 1_000.0,
                "min" => 60_000.0,
                "h" => 3_600_000.0,
                _ => {
                    return Tok::Bad(format!(
                        "`{}`: a number run into letters (a duration's units are ms, s, min and h)",
                        &self.src[start..self.pos]
                    ));
                }
            };
            let Ok(n) = text.parse::<f64>() else {
                return Tok::Bad(format!("`{}` is no number", &self.src[start..self.pos]));
            };
            let ms = n * per;
            if !ms.is_finite() || ms > i64::MAX as f64 / 2.0 {
                return Tok::Bad(format!("`{}` is past a duration's range", &self.src[start..self.pos]));
            }
            if ms.fract() != 0.0 {
                return Tok::Bad(format!("`{}` is not a whole number of milliseconds", &self.src[start..self.pos]));
            }
            return Tok::Duration(ms as i64);
        }
        if float {
            match text.parse::<f64>() {
                Ok(f) if f.is_finite() => Tok::Float(f),
                _ => Tok::Bad(format!("`{}` is past a number's range", &self.src[start..number_end])),
            }
        } else {
            match text.parse::<i64>() {
                Ok(i) => Tok::Int(i),
                Err(_) => Tok::Bad(format!("`{}` is past a whole number's range", &self.src[start..number_end])),
            }
        }
    }

    /// A string on one line: its escapes read, every `{` expression `}` lexed in place.
    fn string(&mut self) -> Tok {
        let mut pieces: Vec<StrPiece> = Vec::new();
        let mut text = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Tok::Bad("a string not closed on its line".to_string());
            };
            match c {
                '\n' => return Tok::Bad("a string not closed on its line".to_string()),
                '"' => {
                    self.bump();
                    if !text.is_empty() || pieces.is_empty() {
                        pieces.push(StrPiece::Text(text));
                    }
                    return Tok::Str(pieces);
                }
                '\\' => {
                    self.bump();
                    let e = self.bump();
                    match e {
                        Some('"') => text.push('"'),
                        Some('\\') => text.push('\\'),
                        Some('n') => text.push('\n'),
                        Some('t') => text.push('\t'),
                        Some('{') => text.push('{'),
                        Some('}') => text.push('}'),
                        Some('\n') | None => return Tok::Bad("a string not closed on its line".to_string()),
                        Some(other) => {
                            self.skip_string();
                            return Tok::Bad(format!("`\\{other}` is no escape (\\\", \\\\, \\n, \\t, \\{{, \\}})"));
                        }
                    }
                }
                '{' => {
                    self.bump();
                    let inner = self.pos;
                    while let Some(c) = self.peek() {
                        if c == '}' || c == '\n' || c == '"' {
                            break;
                        }
                        self.bump();
                    }
                    if self.peek() != Some('}') {
                        self.skip_string();
                        return Tok::Bad("a `{` in a string not closed by `}` (a `{` alone is written `\\{`)".to_string());
                    }
                    let inner_end = self.pos;
                    self.bump();
                    if !text.is_empty() {
                        pieces.push(StrPiece::Text(std::mem::take(&mut text)));
                    }
                    let mut tokens = Lexer { src: self.src, pos: inner, end: inner_end }.run();
                    tokens.retain(|t| t.tok != Tok::Newline);
                    pieces.push(StrPiece::Expr(tokens, Span::new(inner, inner_end)));
                }
                '}' => {
                    self.skip_string();
                    return Tok::Bad("a `}` alone in a string (written `\\}`)".to_string());
                }
                _ => {
                    self.bump();
                    text.push(c);
                }
            }
        }
    }

    /// Past the rest of a string that is already refused, so the line reads on.
    fn skip_string(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                return;
            }
            self.bump();
            if c == '"' {
                return;
            }
        }
    }
}
