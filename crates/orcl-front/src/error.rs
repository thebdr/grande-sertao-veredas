//! Why the front end wrote no program file: what it refused, by name and source location, or why it could not run.
use core::fmt;

/// A source location, as rustc names it: the file as the compiler was given it, a line and a column, each from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// What kind of thing a program holds that the interpreter does not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A type outside ORCL's: a raw pointer, `String`, a slice, a closure, a trait object, a `char`, a 128-bit
    /// integer, a union, a type from a crate other than the program's (core's `Option` and `Duration` excepted),
    /// or `usize` and `isize` where the program declares them (a field, a parameter, a result, a named variable),
    /// wherever they stand in the type it declares (a tuple's field, a reference's pointee, an enum's payload, an
    /// array's element).
    Type,
    /// A type with drop glue: no ORCL type has any (O6).
    DropGlue,
    /// A statement the interpreter does not have.
    Statement,
    /// A value the interpreter does not have: an operation, a cast, a constant, a place's projection.
    Value,
    /// A reference to a constant (a promoted `&FAST` or `&1`, a `const R: &T`) in a function whose result holds a
    /// reference, or one of whose parameters reaches a `&mut` to a type that holds one (the scan's `&mut Self` when
    /// the state holds one): the interpreter keeps no constant in memory of its own, so the function holds the
    /// constant in its frame, which a reference it returned or wrote out would outlive (O3: no reference in a return
    /// value or a field).
    ConstRef,
    /// A terminator the interpreter does not have.
    Terminator,
    /// A call into core outside the closed list (`docs/questions.md` Q227).
    CoreCall,
    /// A call the interpreter cannot make: into a crate that is neither the program's nor core, into a foreign
    /// function, through a pointer or a trait object.
    Call,
    /// A function of the program's crates whose MIR the compiler did not keep: a crate built without the front end's
    /// flags (`-Zalways-encode-mir`), an rlib of `orcl-std` built by rustc alone, by hand or elsewhere (under cargo
    /// the driver's answer to `-vV` keeps a plain build's crates apart from a wrapped build's).
    NoBody,
    /// A crate that is no program: not one non-generic `orcl_std::Program` implementation, or no `Default` of its
    /// state.
    Program,
}

impl Refused {
    /// The words a message names it by.
    pub const fn words(self) -> &'static str {
        match self {
            Refused::Type => "a type outside ORCL's",
            Refused::DropGlue => "a type with drop glue",
            Refused::Statement => "a statement the interpreter does not have",
            Refused::Value => "a value the interpreter does not have",
            Refused::ConstRef => "a reference to a constant a returned reference may hold",
            Refused::Terminator => "a terminator the interpreter does not have",
            Refused::CoreCall => "a call into core outside the list",
            Refused::Call => "a call the interpreter cannot make",
            Refused::NoBody => "a crate built without the front end's flags, by rustc alone or outside the front end",
            Refused::Program => "no program",
        }
    }
}

/// What the program holds that the interpreter does not run: its kind, its name, and where the program holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub what: Refused,
    pub name: String,
    /// `None` for what has no place of its own in the source, a crate with no program.
    pub at: Option<Location>,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.at {
            Some(at) => write!(f, "{at}: {}: `{}`", self.what.words(), self.name),
            None => write!(f, "{}: `{}`", self.what.words(), self.name),
        }
    }
}

impl core::error::Error for Refusal {}

/// Why the driver wrote no program file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The program holds what the interpreter does not run.
    Refused(Refusal),
    /// The driver's own options: one missing or given twice.
    Usage(String),
    /// What the front end made, the interpreter refused to load: the front end's own fault, named by the loader.
    Lowering(String),
    /// The program file could not be written.
    Write { path: String, error: String },
    /// The compiler stopped with errors of its own, which it printed.
    Compile,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Refused(r) => write!(f, "{r}"),
            Error::Usage(what) => write!(f, "usage: {what}"),
            Error::Lowering(what) => write!(f, "the lowered program does not load: {what}"),
            Error::Write { path, error } => write!(f, "{path}: {error}"),
            Error::Compile => write!(f, "the compiler stopped with errors"),
        }
    }
}

impl core::error::Error for Error {}

impl From<Refusal> for Error {
    fn from(r: Refusal) -> Error {
        Error::Refused(r)
    }
}
