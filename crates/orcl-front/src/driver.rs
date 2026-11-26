//! The driver: rustc, run as the compiler of a program's crates (as `RUSTC`, or as `RUSTC_WRAPPER`, whose first
//! argument, the compiler it wraps, it drops). Every crate is compiled with `-Zalways-encode-mir`, so the standard
//! library's functions keep their bodies for the program that calls them, and `-Zmir-opt-level=0`, so every function
//! keeps the shape of its source (`docs/orcl/front-end.md`). The panic strategy is left as the caller gives it: an
//! ORCL type has no drop glue, so MIR holds no cleanup block to keep out, and a crate built so links with the rest of
//! the build as it would.
//!
//! The program's own crate, the one whose `--crate-name` is the program's, is lowered once the compiler has analysed
//! it, and the program file is written once the compiler has finished it, with no error.
use std::process::ExitCode;

use orcl_interp::{Limits, Machine};
use orcl_interp::file::{self, Header};

use crate::NIGHTLY;
use crate::error::Error;

/// The options the driver takes for itself and strips before rustc sees them, each `--name value` or
/// `--name=value`, or else from the environment variable beside it.
const PROGRAM: (&str, &str) = ("--orcl-program", "ORCL_FRONT_PROGRAM");
const OUT: (&str, &str) = ("--orcl-out", "ORCL_FRONT_OUT");
const REVISION: (&str, &str) = ("--orcl-revision", "ORCL_FRONT_REVISION");

/// What the driver lowers: the program's crate, where its file goes, and the revision its header names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub program: String,
    pub out: String,
    pub revision: String,
}

/// An invocation, read: the arguments rustc is given and, when one of the driver's options names a program, what to
/// lower.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub rustc: Vec<String>,
    pub job: Option<Job>,
}

fn take(args: &mut Vec<String>, name: &str) -> Result<Option<String>, Error> {
    let mut found = None;
    let mut i = 0;
    while i < args.len() {
        let value = match args.get(i) {
            Some(a) if a == name => {
                args.remove(i);
                if i < args.len() { Some(args.remove(i)) } else { return Err(Error::Usage(format!("{name} needs a value"))) }
            }
            Some(a) if a.strip_prefix(name).is_some_and(|rest| rest.starts_with('=')) => {
                let a = args.remove(i);
                a.split_once('=').map(|(_, v)| v.to_string())
            }
            _ => {
                i = i.saturating_add(1);
                continue;
            }
        };
        if found.is_some() {
            return Err(Error::Usage(format!("{name} given twice")));
        }
        found = value;
    }
    Ok(found)
}

fn crate_name(args: &[String]) -> Option<&str> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--crate-name" {
            return it.next().map(String::as_str);
        }
        if let Some(name) = a.strip_prefix("--crate-name=") {
            return Some(name);
        }
    }
    None
}

impl Invocation {
    /// Reads the process's arguments (the driver's own path first) and, for an option not given, `env`.
    pub fn read(mut args: Vec<String>, env: impl Fn(&str) -> Option<String>) -> Result<Invocation, Error> {
        // as `RUSTC_WRAPPER` the compiler it wraps comes first; the driver is the compiler
        if args.get(1).is_some_and(|a| {
            std::path::Path::new(a).file_stem().is_some_and(|s| s == "rustc")
        }) {
            args.remove(1);
        }
        let mut option = |(name, var): (&str, &str)| -> Result<Option<String>, Error> {
            Ok(take(&mut args, name)?.or_else(|| env(var)))
        };
        let program = option(PROGRAM)?;
        let out = option(OUT)?;
        let revision = option(REVISION)?;
        let asks_version = args.iter().any(|a| a == "-vV" || a == "-V" || a == "--version");
        if !asks_version {
            args.push("-Zalways-encode-mir".into());
            args.push("-Zmir-opt-level=0".into());
        }
        let job = match program {
            Some(program) if crate_name(&args) == Some(program.as_str()) => {
                let out = out.ok_or_else(|| Error::Usage(format!("{} names no file to write", OUT.0)))?;
                let revision = revision.ok_or_else(|| Error::Usage(format!("{} names no revision", REVISION.0)))?;
                Some(Job { program, out, revision })
            }
            _ => None,
        };
        Ok(Invocation { rustc: args, job })
    }
}

struct Plain;

impl rustc_driver::Callbacks for Plain {}

/// What the driver's own check of a lowered program lets the interpreter's loader lay out: no region, no stack, and
/// room for the program's largest value and result up to 2^24 slots (512 MiB at 32 bytes a slot), far past what an
/// ORCL program holds and short of what would hurt the machine the driver runs on.
const CHECK: Limits = Limits { stack: 0, memory: 1 << 24 };

/// The program file's bytes, lowered from the crate the compiler holds and checked by the interpreter's own loader.
fn program_file(tcx: rustc_middle::ty::TyCtxt<'_>, job: &Job) -> Result<Vec<u8>, Error> {
    let program = crate::lower::lower(tcx)?;
    Machine::load(program.clone(), &[], CHECK).map_err(|e| Error::Lowering(e.to_string()))?;
    let header = Header { language: orcl_std::LANGUAGE, nightly: NIGHTLY.into(), revision: job.revision.clone() };
    Ok(file::write(&header, &program))
}

fn compile_and_lower(rustc: &[String], job: &Job) -> Result<(), Error> {
    let result = rustc_public::run_with_tcx!(rustc, |tcx| match program_file(tcx, job) {
        Ok(bytes) => std::ops::ControlFlow::Continue(bytes),
        Err(e) => std::ops::ControlFlow::Break(e),
    });
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(rustc_public::CompilerError::Interrupted(e)) => return Err(e),
        Err(_) => return Err(Error::Compile),
    };
    std::fs::write(&job.out, bytes).map_err(|e| Error::Write { path: job.out.clone(), error: e.to_string() })
}

/// The driver's `main`.
pub fn main() -> ExitCode {
    let invocation = match Invocation::read(std::env::args().collect(), |v| std::env::var(v).ok()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: orcl-front: {e}");
            return ExitCode::from(2);
        }
    };
    match &invocation.job {
        Some(job) => match compile_and_lower(&invocation.rustc, job) {
            Ok(()) => ExitCode::SUCCESS,
            Err(Error::Compile) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("error: orcl-front: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            rustc_driver::catch_with_exit_code(|| {
                rustc_driver::compiler_entrypoint(&invocation.rustc, &mut Plain);
            })
        }
    }
}
