//! The driver: rustc, run as the compiler of a program's crates (as `RUSTC`, or as `RUSTC_WRAPPER`, whose first
//! argument, the compiler it wraps, it drops). Every crate is compiled with `-Zalways-encode-mir`, so the standard
//! library's functions keep their bodies for the program that calls them, and `-Zmir-opt-level=0`, so every function
//! keeps the shape of its source (`docs/orcl/front-end.md`). The panic strategy is left as the caller gives it: an
//! ORCL type has no drop glue, so MIR holds no cleanup block to keep out, and a crate built so links with the rest of
//! the build as it would.
//!
//! The program's own crate, the one whose name is the program's, is lowered once the compiler has analysed it, when
//! the compiler builds it as a library, and the program file is written once the compiler has finished it, with no
//! error. Its name is the one the command line's `--crate-name` gives, as cargo always passes it, or, with none, the
//! one the compiler gives the crate itself, from its file or its own `crate_name` attribute. Built as anything else,
//! a bin of the program's name (cargo's bin target of a package beside its library), its own tests (`--test`) or a
//! procedural macro, it is compiled and nothing is lowered or written.
//!
//! Every crate it compiles depends on the driver's three variables, `ORCL_FRONT_PROGRAM`, `ORCL_FRONT_OUT` and
//! `ORCL_FRONT_REVISION`: the compiler's dep-info file names each with the value it holds (`# env-dep:`), as
//! clippy-driver names `CLIPPY_ARGS`, and cargo, which reads that file back, rebuilds the crates when one of them
//! changes, so a new revision or output path is written with no source changed.
//!
//! Asked the compiler's verbose version (`-vV`), the driver answers with rustc's lines and one of its own after them,
//! `orcl-front: <its version> <the nightly> build <its executable's hash>` ([`version_line`]), whatever the environment or the options say. Cargo
//! asks the wrapper as it asks rustc and fingerprints every crate by that answer, so in a target directory shared by
//! plain and wrapped builds every switch between the two, either way, rebuilds every crate: a crate built plainly,
//! without the driver's flags, is never reused by a wrapped build, nor a stale program file kept. A function of the
//! program's crates without MIR, from a crate built without those flags by other means (an rlib built by rustc
//! alone, by hand or elsewhere), is refused naming that likely cause.
//!
//! What the driver keeps fresh is the file of one configuration of a program in one target directory. Cargo keeps a set of
//! compiled units for each configuration (a feature set, a profile) in a target directory and runs no compiler for a
//! unit it finds fresh, so a configuration built again after another into the same file may compile nothing and leave
//! the other's program there. Each configuration of a program is built into an output path of its own; a build
//! naming a program the build does not hold lowers nothing, writes no file and leaves what was there.
// DECISION PENDING: docs/questions.md Q238
use std::io::Write;
use std::process::ExitCode;

use orcl_interp::{Limits, Machine};
use orcl_interp::file::{self, Header};
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::{Compiler, Config};
use rustc_middle::ty::TyCtxt;
use rustc_public::rustc_internal;
use rustc_session::Session;
use rustc_span::Symbol;
use rustc_span::def_id::LOCAL_CRATE;
use rustc_structures::CrateType;

use crate::NIGHTLY;
use crate::error::Error;

/// The options the driver takes for itself and strips before rustc sees them, each `--name value` or
/// `--name=value`, or else from the environment variable beside it.
const PROGRAM: (&str, &str) = ("--orcl-program", "ORCL_FRONT_PROGRAM");
const OUT: (&str, &str) = ("--orcl-out", "ORCL_FRONT_OUT");
const REVISION: (&str, &str) = ("--orcl-revision", "ORCL_FRONT_REVISION");

/// rustc's options asking its version, and asking it verbose: each alone, or the two in one (`-vV`, as cargo asks).
const VERSION: [&str; 4] = ["-V", "--version", "-vV", "-Vv"];
const VERBOSE: [&str; 4] = ["-v", "--verbose", "-vV", "-Vv"];

fn any_of(args: &[String], options: &[&str]) -> bool {
    args.iter().any(|a| options.contains(&a.as_str()))
}

/// The line the driver adds to the compiler's verbose version, after rustc's own: the front end, its version and the
/// nightly it is pinned to and its own build, `orcl-front: 2.0.0-alpha.1 nightly-2026-11-21 build <hash>`.
pub fn version_line() -> String {
    format!("orcl-front: {} {NIGHTLY} build {}", env!("CARGO_PKG_VERSION"), build_identity())
}

/// The driver's own build: the FNV-1a hash of its executable's bytes, so a driver rebuilt with any change answers
/// `-vV` otherwise and cargo, which fingerprints that answer, rebuilds what an older driver lowered; `unknown` when
/// the executable cannot be read.
fn build_identity() -> String {
    let bytes = std::env::current_exe().and_then(std::fs::read);
    match bytes {
        Ok(bytes) => format!("{:016x}", fnv1a(&bytes)),
        Err(_) => "unknown".to_string(),
    }
}

/// FNV-1a, 64 bits.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// What the driver lowers: the program's crate, where its file goes, and the revision its header names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub program: String,
    pub out: String,
    pub revision: String,
}

/// Which crate of an invocation the driver lowers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lowers {
    /// None: no program named, a crate the command line names otherwise, or the compiler's version asked for.
    Nothing,
    /// The crate the command line's `--crate-name` names, the program's.
    Named(Job),
    /// The crate the compiler names, from its file or its `crate_name` attribute, when that name is the program's:
    /// the command line names no crate. Where its file goes and the revision are asked for once it is known to be.
    ByCompiler { program: String, out: Option<String>, revision: Option<String> },
}

/// An invocation, read: the arguments rustc is given and, when one of the driver's options names a program, the crate
/// to lower.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub rustc: Vec<String>,
    pub lowers: Lowers,
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

/// The program `program`'s job, its file and its revision each given, or the usage error naming the one missing.
fn job(program: String, out: Option<String>, revision: Option<String>) -> Result<Job, Error> {
    let out = out.ok_or_else(|| Error::Usage(format!("{} names no file to write", OUT.0)))?;
    let revision = revision.ok_or_else(|| Error::Usage(format!("{} names no revision", REVISION.0)))?;
    Ok(Job { program, out, revision })
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
        // as `RUSTC_WRAPPER` the compiler it wraps comes first, a `rustc` of no extension but `.exe`; the driver is
        // the compiler
        if args.get(1).is_some_and(|a| {
            let path = std::path::Path::new(a);
            path.file_stem().is_some_and(|s| s == "rustc") && path.extension().is_none_or(|e| e == "exe")
        }) {
            args.remove(1);
        }
        let mut option = |(name, var): (&str, &str)| -> Result<Option<String>, Error> {
            Ok(take(&mut args, name)?.or_else(|| env(var)))
        };
        let program = option(PROGRAM)?;
        let out = option(OUT)?;
        let revision = option(REVISION)?;
        let asks_version = any_of(&args, &VERSION);
        if !asks_version {
            args.push("-Zalways-encode-mir".into());
            args.push("-Zmir-opt-level=0".into());
        }
        // whether the program's crate is built as a library, and so lowered, the compiler says (`program_file`), and,
        // with no `--crate-name`, whether the crate is the program's (`lowered`)
        let lowers = match program {
            None => Lowers::Nothing,
            Some(program) => match crate_name(&args) {
                Some(name) if name == program => Lowers::Named(job(program, out, revision)?),
                Some(_) => Lowers::Nothing,
                None if asks_version => Lowers::Nothing,
                None => Lowers::ByCompiler { program, out, revision },
            },
        };
        Ok(Invocation { rustc: args, lowers })
    }
}

/// What the driver's own check of a lowered program lets the interpreter's loader lay out: no region, no stack, and
/// room for the program's largest value and result up to 2^24 slots (512 MiB at 32 bytes a slot), far past what an
/// ORCL program holds and short of what would hurt the machine the driver runs on.
const CHECK: Limits = Limits { stack: 0, memory: 1 << 24 };

/// Whether the compiler builds the crate as a library, as a program's crate is built, whatever its options or its
/// own `crate_type` attribute say: a bin, a test build (an executable too) or a procedural macro is not one.
fn builds_library(tcx: TyCtxt<'_>) -> bool {
    tcx.crate_types().iter().any(|t| !matches!(t, CrateType::Executable | CrateType::ProcMacro))
}

/// The program file's bytes, lowered from the crate the compiler holds and checked by the interpreter's own loader,
/// or nothing for a crate of the program's name the compiler builds as no library.
fn program_file(tcx: TyCtxt<'_>, job: &Job) -> Result<Option<Vec<u8>>, Error> {
    if !builds_library(tcx) {
        return Ok(None);
    }
    let program = crate::lower::lower(tcx)?;
    Machine::load(program.clone(), &[], CHECK).map_err(|e| Error::Lowering(e.to_string()))?;
    let header = Header { language: orcl_std::LANGUAGE, nightly: NIGHTLY.into(), revision: job.revision.clone() };
    Ok(Some(file::write(&header, &program)))
}

/// A program file's bytes and the path they are written to.
type ProgramFile = (Vec<u8>, String);

/// The program file's bytes and where they go, for the crate the compiler holds when it is the program's crate and
/// built as a library, or nothing for any other crate.
fn lowered(tcx: TyCtxt<'_>, lowers: &Lowers) -> Result<Option<ProgramFile>, Error> {
    let job = match lowers {
        Lowers::Nothing => return Ok(None),
        Lowers::Named(job) => job.clone(),
        // the name the compiler gives the crate, from its file or its `crate_name` attribute
        Lowers::ByCompiler { program, out, revision } => {
            if tcx.crate_name(LOCAL_CRATE).as_str() != program {
                return Ok(None);
            }
            job(program.clone(), out.clone(), revision.clone())?
        }
    };
    Ok(program_file(tcx, &job)?.map(|bytes| (bytes, job.out)))
}

/// The driver's variables, each with the value the environment gives it (none when it is unset or not Unicode), as
/// the environment the crate depends on: the compiler writes them into its dep-info file, which cargo reads back.
fn track(sess: &Session) {
    let mut depends = sess.env_depinfo.borrow_mut();
    for (_, var) in [PROGRAM, OUT, REVISION] {
        depends.insert((Symbol::intern(var), std::env::var(var).ok().map(|v| Symbol::intern(&v))));
    }
}

/// What the driver asks of the compiler for every crate: the environment the crate depends on and, once the crate is
/// analysed, its program file, lowered through `rustc_public` when the crate is the program's.
struct Front<'a> {
    lowers: &'a Lowers,
    /// What the lowering made: the file and where it goes, nothing for a crate not lowered, or why no file is written.
    lowered: Option<Result<Option<ProgramFile>, Error>>,
}

impl Callbacks for Front<'_> {
    fn config(&mut self, config: &mut Config) {
        config.track_state = Some(Box::new(track));
    }

    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        if matches!(self.lowers, Lowers::Nothing) {
            return Compilation::Continue;
        }
        let lowers = self.lowers;
        // `rustc_public` refuses only to run inside itself, which the driver never asks of it
        let lowered =
            rustc_internal::run(tcx, || lowered(tcx, lowers)).unwrap_or_else(|e| Err(Error::Lowering(e.to_string())));
        let go_on = lowered.is_ok();
        self.lowered = Some(lowered);
        if go_on { Compilation::Continue } else { Compilation::Stop }
    }
}

fn compile_and_lower(rustc: &[String], lowers: &Lowers) -> Result<(), Error> {
    let mut front = Front { lowers, lowered: None };
    let compiled = rustc_driver::catch_fatal_errors(|| rustc_driver::compiler_entrypoint(rustc, &mut front));
    let (bytes, out) = match (compiled, front.lowered) {
        (Ok(()), Some(Ok(Some(file)))) => file,
        (Ok(()), Some(Ok(None))) => return Ok(()),
        (Ok(()), Some(Err(e))) => return Err(e),
        // the command line naming no crate, the compiler ended before its analysis (a `--print`, `--emit=dep-info`
        // alone): no crate built and none named, as with rustc alone
        (Ok(()), None) if matches!(lowers, Lowers::ByCompiler { .. }) => return Ok(()),
        _ => return Err(Error::Compile),
    };
    std::fs::write(&out, bytes).map_err(|e| Error::Write { path: out, error: e.to_string() })
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
    match &invocation.lowers {
        Lowers::Nothing => rustc_driver::catch_with_exit_code(|| {
            rustc_driver::compiler_entrypoint(&invocation.rustc, &mut Front { lowers: &Lowers::Nothing, lowered: None });
            // whatever the environment or the options say: cargo asks the wrapper as it asks rustc, and the driver
            // gives every crate its flags, a program named or none
            if any_of(&invocation.rustc, &VERSION) && any_of(&invocation.rustc, &VERBOSE) {
                // a stdout closed under rustc's own lines is rustc's to answer
                let _ = writeln!(std::io::stdout(), "{}", version_line());
            }
        }),
        lowers => match compile_and_lower(&invocation.rustc, lowers) {
            Ok(()) => ExitCode::SUCCESS,
            Err(Error::Compile) => ExitCode::FAILURE,
            // an option missing, found once the compiler named the crate the program's: the exit code of the usage
            // errors found before rustc runs
            Err(e @ Error::Usage(_)) => {
                eprintln!("error: orcl-front: {e}");
                ExitCode::from(2)
            }
            Err(e) => {
                eprintln!("error: orcl-front: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
