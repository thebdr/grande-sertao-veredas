//! The front end proven against Rust (C-156). Each conformance program under `programs/` (one file, or a folder of
//! its name holding its files) is compiled natively into this test and by the driver as a crate of its own
//! (`orcl-std` first, as the driver compiles it, then the program with `--extern`), into a folder of its own under
//! `CARGO_TARGET_TMPDIR`. The lowered program is read back from its
//! file, its first state and its safe outputs computed by calling its `Default` and its `safe_outputs` on a machine
//! (as a host does until the file carries them, `docs/questions.md` Q231), and run on the engine of a scan beside
//! its native build, each on its own virtual clock: every scan's report and every write compared, over thousands of
//! scans with varied inputs, and its instruction count recorded (the conveyor's and the probe's asserted as the
//! verification records them). Each refusal is met by a program that triggers it, its name and its location asserted
//! exactly, and no file written.
//!
//! Run by the pinned nightly alone: `cargo +nightly-2026-11-21 test -p orcl-front --features driver --test lowering`.

mod cap;

#[global_allocator]
static CAP: cap::Capped = cap::Capped;

#[allow(dead_code)]
#[path = "programs/arith.rs"]
mod arith;
#[allow(dead_code)]
#[path = "programs/cgen.rs"]
mod cgen;
#[allow(dead_code)]
#[path = "programs/conveyor.rs"]
mod conveyor;
#[allow(dead_code)]
#[path = "programs/deref2.rs"]
mod deref2;
#[allow(dead_code)]
#[path = "programs/edges.rs"]
mod edges;
#[allow(dead_code)]
#[path = "programs/faults.rs"]
mod faults;
#[allow(dead_code)]
#[path = "programs/lines.rs"]
mod lines;
#[allow(dead_code)]
#[path = "programs/more.rs"]
mod more;
#[allow(dead_code)]
#[path = "programs/niches.rs"]
mod niches;
#[allow(dead_code)]
#[path = "programs/order.rs"]
mod order;
#[allow(dead_code)]
#[path = "programs/own_trait.rs"]
mod own_trait;
#[allow(dead_code)]
#[path = "programs/p_names.rs"]
mod p_names;
#[allow(dead_code)]
#[path = "programs/p_repr.rs"]
mod p_repr;
#[allow(dead_code)]
#[path = "programs/p_traits.rs"]
mod p_traits;
#[allow(dead_code)]
#[path = "programs/p_wrap.rs"]
mod p_wrap;
#[allow(dead_code)]
#[path = "programs/padded.rs"]
mod padded;
#[allow(dead_code)]
#[path = "programs/probe.rs"]
mod probe;
#[allow(dead_code)]
#[path = "programs/refs.rs"]
mod refs;
#[allow(dead_code)]
#[path = "programs/shapes.rs"]
mod shapes;
#[allow(dead_code)]
#[path = "programs/sound.rs"]
mod sound;
#[allow(dead_code)]
#[path = "programs/twofile/twofile.rs"]
mod twofile;
#[allow(dead_code)]
#[path = "programs/zst_const.rs"]
mod zst_const;
#[allow(dead_code)]
#[path = "programs/zst_ref.rs"]
mod zst_ref;
#[allow(dead_code)]
#[path = "programs/usize_temps.rs"]
mod usize_temps;
#[allow(dead_code)]
#[path = "programs/own_macro.rs"]
mod own_macro;
#[allow(dead_code)]
#[path = "programs/grid.rs"]
mod grid;

use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::Duration;

use orcl_interp::file::{self, Header};
use orcl_interp::{
    Arg, BinOp, Check, Const, ErrorKind, FunctionId, Int, Limits, Machine, Operand, Program as Code, ProgramError,
    RegionId, Scalar, Terminator, Type, TypeId, Value,
};
use orcl_runtime::{Image, Interpreted, Io, Regions, RunState, Task, TaskConfig, TaskProgram, VirtualClock};
use orcl_std::Program;

/// The pinned nightly, as the crate's `rust-toolchain` names it (`tests/shell.rs` holds it to `orcl_front::NIGHTLY`;
/// this test links nothing of the library, which on the nightly links the compiler).
fn nightly() -> &'static str {
    static NIGHTLY: OnceLock<String> = OnceLock::new();
    NIGHTLY.get_or_init(|| fs::read_to_string(here().join("rust-toolchain")).unwrap().trim().to_string())
}

/// The revision every lowered program's header names.
const REVISION: &str = "conformance";

/// What a lowered program's machine may lay out, in slots: its stack and its memory in all (`docs/questions.md`
/// Q230), as the target's settings will give them.
const LIMITS: Limits = Limits { stack: 4096, memory: 1 << 20 };

// The driver, run as the compiler.

fn here() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The pinned nightly's libraries, where the driver finds the `rustc_driver` it links.
fn sysroot_lib() -> &'static str {
    static LIB: OnceLock<String> = OnceLock::new();
    LIB.get_or_init(|| {
        let out = Command::new("rustc").arg(format!("+{}", nightly())).args(["--print", "sysroot"]).output().unwrap();
        assert!(out.status.success(), "rustc +{} --print sysroot: {}", nightly(), String::from_utf8_lossy(&out.stderr));
        format!("{}/lib", String::from_utf8(out.stdout).unwrap().trim())
    })
}

fn driver(args: &[String]) -> Output {
    driver_with(args, &[])
}

/// The search path the driver's process is given: the nightly's libraries first.
fn library_path() -> String {
    match std::env::var("LD_LIBRARY_PATH") {
        Ok(p) if !p.is_empty() => format!("{}:{p}", sysroot_lib()),
        _ => sysroot_lib().to_string(),
    }
}

fn driver_with(args: &[String], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orcl-front-driver"));
    command.env("LD_LIBRARY_PATH", library_path()).args(args);
    for (k, v) in env {
        command.env(k, v);
    }
    command.output().unwrap()
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// What the driver made of a program's crate: its file read back, or what it said on refusing it.
struct Lowered {
    file: PathBuf,
    result: Result<(Header, Code), String>,
}

/// Lowers the crate `source` as the program `name`, in a folder of its own.
fn lower(name: &str, source: &Path) -> Lowered {
    lower_in(name, name, source)
}

/// The folder `name` under `CARGO_TARGET_TMPDIR`, emptied: one test's, since tests run at once.
fn fresh(name: &str) -> PathBuf {
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lowering").join(name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    work
}

/// `orcl-std` compiled by the driver into `work`, the driver's environment holding `env`.
fn compile_std(work: &Path, env: &[(&str, &str)]) {
    let w = work.display().to_string();
    let std_src = here().join("../orcl-std/src/lib.rs").display().to_string();
    let args = strings(&["--crate-name", "orcl_std", "--crate-type", "rlib", "--edition", "2024", &std_src, "--out-dir", &w]);
    let out = driver_with(&args, env);
    assert!(out.status.success(), "orcl-std: {}", String::from_utf8_lossy(&out.stderr));
}

/// Lowers the crate `source` as the program `name`, in the folder `folder`: one test's, since tests run at once.
fn lower_in(folder: &str, name: &str, source: &Path) -> Lowered {
    let work = fresh(folder);
    compile_std(&work, &[]);
    let w = work.display().to_string();
    let file = work.join(format!("{name}.orclb"));
    let mut args = strings(&["--crate-name", name, "--crate-type", "rlib", "--edition", "2024"]);
    args.push(source.display().to_string());
    args.extend(strings(&["--extern", &format!("orcl_std={w}/liborcl_std.rlib"), "-L", &w, "--out-dir", &w]));
    args.extend(strings(&["--orcl-program", name, &format!("--orcl-out={}", file.display()), "--orcl-revision", REVISION]));
    let out = driver(&args);
    let result = if out.status.success() {
        Ok(file::read(&fs::read(&file).unwrap()).unwrap())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    };
    Lowered { file, result }
}

/// The conformance program `name`'s crate root: `programs/<name>.rs`, or `programs/<name>/<name>.rs` for a program of
/// more than one file.
fn program(name: &str) -> PathBuf {
    let file = here().join("tests/programs").join(format!("{name}.rs"));
    if file.exists() { file } else { here().join("tests/programs").join(name).join(format!("{name}.rs")) }
}

// Values as the interpreter's slots: a native program's own types, laid out as the front end lowers them (fields in
// their declared order, an enum its discriminant then its variant's fields then unwritten slots, `Option` among them).

trait Slots: Sized {
    const SLOTS: usize;
    fn put(&self, out: &mut Vec<Scalar>);
    fn take(slots: &mut &[Scalar]) -> Option<Self>;
}

fn next(slots: &mut &[Scalar]) -> Option<Scalar> {
    let (first, rest) = slots.split_first()?;
    *slots = rest;
    Some(*first)
}

/// Skips `n` slots an enum's variant leaves unwritten.
fn unwritten(slots: &mut &[Scalar], n: usize) -> Option<()> {
    for _ in 0..n {
        if next(slots)? != Scalar::Uninit {
            return None;
        }
    }
    Some(())
}

macro_rules! int_slots {
    ($($t:ty => $i:ident),*) => {$(
        impl Slots for $t {
            const SLOTS: usize = 1;
            fn put(&self, out: &mut Vec<Scalar>) {
                out.push(Scalar::Int(Int::$i, i128::from(*self)));
            }
            fn take(slots: &mut &[Scalar]) -> Option<Self> {
                match next(slots)? {
                    Scalar::Int(Int::$i, v) => <$t>::try_from(v).ok(),
                    _ => None,
                }
            }
        }
    )*};
}

int_slots!(u8 => U8, u16 => U16, u32 => U32, u64 => U64, i8 => I8, i16 => I16, i32 => I32, i64 => I64);

impl Slots for bool {
    const SLOTS: usize = 1;
    fn put(&self, out: &mut Vec<Scalar>) {
        out.push(Scalar::Bool(*self));
    }
    fn take(slots: &mut &[Scalar]) -> Option<Self> {
        match next(slots)? {
            Scalar::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl Slots for f32 {
    const SLOTS: usize = 1;
    fn put(&self, out: &mut Vec<Scalar>) {
        out.push(Scalar::F32(*self));
    }
    fn take(slots: &mut &[Scalar]) -> Option<Self> {
        match next(slots)? {
            Scalar::F32(v) => Some(v),
            _ => None,
        }
    }
}

impl Slots for f64 {
    const SLOTS: usize = 1;
    fn put(&self, out: &mut Vec<Scalar>) {
        out.push(Scalar::F64(*self));
    }
    fn take(slots: &mut &[Scalar]) -> Option<Self> {
        match next(slots)? {
            Scalar::F64(v) => Some(v),
            _ => None,
        }
    }
}

impl Slots for Duration {
    const SLOTS: usize = 2;
    fn put(&self, out: &mut Vec<Scalar>) {
        self.as_secs().put(out);
        self.subsec_nanos().put(out);
    }
    fn take(slots: &mut &[Scalar]) -> Option<Self> {
        let secs = u64::take(slots)?;
        let nanos = u32::take(slots)?;
        (nanos < 1_000_000_000).then(|| Duration::new(secs, nanos))
    }
}

/// `Option`'s discriminant is `isize`, as the compiler gives every enum of no `repr`.
fn discriminant(d: i128) -> Scalar {
    Scalar::Int(Int::Isize, d)
}

impl<T: Slots> Slots for Option<T> {
    const SLOTS: usize = 1 + T::SLOTS;
    fn put(&self, out: &mut Vec<Scalar>) {
        match self {
            None => {
                out.push(discriminant(0));
                out.extend(std::iter::repeat_n(Scalar::Uninit, T::SLOTS));
            }
            Some(v) => {
                out.push(discriminant(1));
                v.put(out);
            }
        }
    }
    fn take(slots: &mut &[Scalar]) -> Option<Self> {
        match next(slots)? {
            Scalar::Int(Int::Isize, 0) => unwritten(slots, T::SLOTS).map(|()| None),
            Scalar::Int(Int::Isize, 1) => T::take(slots).map(Some),
            _ => None,
        }
    }
}

impl Slots for shapes::Mode {
    const SLOTS: usize = 3;
    fn put(&self, out: &mut Vec<Scalar>) {
        let start = out.len();
        match self {
            shapes::Mode::Off => out.push(discriminant(0)),
            shapes::Mode::Hold(v) => {
                out.push(discriminant(1));
                v.put(out);
            }
            shapes::Mode::Run { speed, forward } => {
                out.push(discriminant(2));
                speed.put(out);
                forward.put(out);
            }
            shapes::Mode::Fault(code) => {
                out.push(discriminant(3));
                code.0.put(out);
            }
        }
        out.resize(start + Self::SLOTS, Scalar::Uninit);
    }
    fn take(slots: &mut &[Scalar]) -> Option<Self> {
        let (mode, used) = match next(slots)? {
            Scalar::Int(Int::Isize, 0) => (shapes::Mode::Off, 0),
            Scalar::Int(Int::Isize, 1) => (shapes::Mode::Hold(u8::take(slots)?), 1),
            Scalar::Int(Int::Isize, 2) => (shapes::Mode::Run { speed: i16::take(slots)?, forward: bool::take(slots)? }, 2),
            Scalar::Int(Int::Isize, 3) => (shapes::Mode::Fault(shapes::Code(u16::take(slots)?)), 1),
            _ => return None,
        };
        unwritten(slots, 2 - used)?;
        Some(mode)
    }
}

/// A structure's slots, its fields listed in their declared order, and its process image.
macro_rules! image {
    ($t:path { $($f:ident: $ft:ty),* $(,)? }) => {
        impl Slots for $t {
            const SLOTS: usize = 0 $(+ <$ft as Slots>::SLOTS)*;
            fn put(&self, out: &mut Vec<Scalar>) {
                $(self.$f.put(out);)*
            }
            fn take(slots: &mut &[Scalar]) -> Option<Self> {
                Some(Self { $($f: <$ft as Slots>::take(slots)?,)* })
            }
        }
        impl Image for $t {
            fn write(&self, slots: &mut Vec<Scalar>) {
                self.put(slots);
            }
            fn read(slots: &[Scalar]) -> Option<Self> {
                let mut rest = slots;
                let value = <$t as Slots>::take(&mut rest)?;
                rest.is_empty().then_some(value)
            }
        }
    };
}

image!(conveyor::Inputs { start_pb: bool, stop_pb: bool, ack_pb: bool, door_ch1: bool, door_ch2: bool, part_at_end: bool });
image!(conveyor::Outputs { motor_on: bool, door_alarm: bool, lamp_running: bool, lamp_fault: bool, parts: i32 });
image!(probe::Inputs {
    a: bool, b: bool, reset: bool, preset: i32, pt_ms: u64, bit: u32, value: bool, w8: u8, w16: u16, w32: u32, w64: u64,
});
image!(probe::Outputs {
    ton: bool, ton_q: bool, ton_et: Duration, tof: bool, tof_q: bool, tof_et: Duration, tp: bool, tp_q: bool,
    tp_et: Duration, rising: bool, rising_q: bool, falling: bool, falling_q: bool, counted: bool, cv: i32,
    counter_q: bool, get8: bool, get16: bool, get32: bool, get64: bool, with8: u8, with16: u16, with32: u32,
    with64: u64, set8: u8, set16: u16, set32: u32, set64: u64, widths: u32,
});
image!(arith::Inputs { a: i32, b: i32, c: u8, d: u16, e: i64, f: u64, k: i8, sh: u32, flag: bool, x: f32, y: f64 });
image!(arith::Outputs {
    sum: i32, diff: i32, product: i32, quotient: i32, remainder: i32, negated: i32, wide: i64, wide_rem: u64,
    bits: u16, not_c: u8, shifted_left: u8, shifted_right: i64, shl: u16, shl_ok: bool, shr: u64, shr_ok: bool,
    sat_add: u8, sat_sub: u16, sat_mul: i8, wrap_add: u8, wrap_sub: i64, wrap_mul: i8, min: i32, max: u64,
    less: bool, at_least: bool, equal: bool, not_flag: bool, to_u8: u8, to_i64: i64, float_to_int: i32,
    double_to_u8: u8, int_to_float: f32, uint_to_double: f64, widened: f64, narrowed: f32, flag_as_int: u8,
    fsum: f32, fproduct: f32, dquotient: f64, ddiff: f64, frem: f32, fneg: f32, fless: bool, dge: bool, total: u64,
});
image!(shapes::Inputs { sel: u8, val: u8, idx: u8, flag: bool, t_ms: u64 });
image!(shapes::Outputs {
    mode: shapes::Mode, mode_code: u16, forward: bool, first: u8, ends: u16, table: u16, sum: u32,
    last: Option<u8>, opt_some: bool, opt_none: bool, opt_val: u8, held: Duration, quotient: u8, remainder: u8,
    factorial: u32, count: u32, dur_eq: bool, dur_ne: bool, dur_lt: bool, dur_le: bool, dur_gt: bool,
    dur_ge: bool, dur_min: Duration, dur_max: Duration, level: u8, niche: u8, const_mode: u16, timeout: Duration,
    pair: u32, repeated: u8, levels: bool,
});
image!(edges::Inputs { a: i16, b: i16, n: u8, flag: bool });
image!(edges::Outputs {
    gt: bool, le: bool, ge: bool, lt: bool, ne: bool, eq: bool, signed_match: u8, dir_code: i8, dir_match: u8,
    light: u8, first_last: u16, picked: u8, off_ref: u16, only: u8, sat16: i16, off_ref2: u16, off_ref3: u16,
});
image!(sound::Inputs { a: i8, b: u64, sel: u8, on: bool, off: bool, t_ms: u64 });
image!(sound::Outputs {
    tuple_match: u8, found: u8, broke: u32, nested: u16, items: u32, xor: bool, bool_ne: bool, widened: u64,
    narrowed: i8, cleared: bool, early: u8, defaulted: u32, cloned: u16, guard: u8, labeled: u8, since_ms: bool,
});
image!(faults::Inputs { sel: u8, a: i32, b: i32, idx: u8, c: u8, sh: u32 });
image!(faults::Outputs { v: i32, w: u8 });
image!(zst_const::Inputs { a: u8 });
image!(zst_const::Outputs { v: u8 });
image!(zst_ref::Inputs { a: u8 });
image!(zst_ref::Outputs { v: u8 });
image!(cgen::Inputs { v: u16 });
image!(cgen::Outputs { short: u16, long: u16 });
image!(deref2::Inputs { a: u8 });
image!(deref2::Outputs { v: u8 });
image!(order::Inputs { sel: u8 });
image!(order::Outputs { p0: u8, p1: u16, p2: bool, a: u8, b: u32, c: u16 });
image!(twofile::Inputs { a: i32, b: i32 });
image!(twofile::Outputs { v: i32 });
image!(lines::Inputs { a: i32, b: i32 });
image!(lines::Outputs { v: i32 });
image!(padded::Inputs { sel: u8 });
image!(padded::Outputs { cmd: u16, pair: u16, recipe: u16 });
image!(niches::Inputs { sel: u8 });
image!(niches::Outputs { held: Duration, has: bool, dir: i8, step: Duration });
image!(p_repr::Inputs { sel: u8, a: i64, b: u64, c: i8, d: u16 });
image!(p_repr::Outputs {
    cmd: u8, cmd_match: u8, wide: u64, wide_match: u8, sign: i16, sign_match: u8, big_match: u8, neg_match: u8,
    sw8: u8, sw_i8: u8, nested: u16, nested_neg: i16, grid: u8, oo: u8, ob: u8, data: i16, data_dur: Duration,
    umax: u64, imin: i64, neg: i64, negs: i32, held: u8,
});
image!(p_traits::Inputs { n: u8, p: bool, q: bool, a8: i8, u64v: u64, i64v: i64, u32v: u32, i16v: i16 });
image!(p_traits::Outputs {
    sa: u32, sb: u32, la: u16, lb: u16, pair_u8: u8, pair_dur: Duration, cfg: u32, count: u8, skipped: u8, got: u8,
    blt: bool, bge: bool, bxor: bool, band: bool, c1: u64, c2: i8, c3: u32, c4: i16, c5: i64, c6: u16, c7: u8,
    s1: u64, s2: i16, s3: u8, s4: i64, s5: u32, f1: f32, f2: i32, f3: u64, f4: f64,
});
image!(p_names::Inputs { a: u16, sel: u8, t_ms: u64 });
image!(p_names::Outputs { d_ms: u32, d_tag: u8, opt: u16, core_some: bool, core_val: u16, held: Duration });
image!(p_wrap::Inputs { sel: u8 });
image!(p_wrap::Outputs { v: u8 });
image!(more::Inputs { a: i16, b: u8, x: f32, y: f64, sel: u8 });
image!(more::Outputs {
    mention: u8, unnamed: u8, fact: u32, bumped: u16, setpoint: Duration, setv: u16, skipped: u8, tagged: u8,
    const_tag: u16, to_u8: u8, to_i32: i32, big_f32: f32, narrowed: f32, below_zero: bool, is_zero: bool, sar: i16,
    forever: Duration, far: Duration, self_lt: bool, self_ne: bool, oob: u8, grid: u16, updated: u8,
});
image!(refs::Inputs { a: u8, b: u8, on: bool });
image!(refs::Outputs { counted: u8, swapped: u16, inner: u8, ret: u16, since: Duration, by_ref: u8, timers: u8, cfg: u16 });
image!(own_trait::Inputs { a: u8 });
image!(own_trait::Outputs { v: u8 });
image!(usize_temps::Inputs { a: u8, b: i8 });
image!(usize_temps::Outputs { oldest: u8, back: i8, low: u8, widest: u8 });
image!(own_macro::Inputs { a: i32, b: i32 });
image!(own_macro::Outputs { v: i32 });
image!(grid::Inputs { v: u16 });
image!(grid::Outputs { small: u16, large: u16 });

// Inputs drawn from a scan's index, the same for the native program and its lowered twin.

fn mix(k: u64, salt: u64) -> u64 {
    // splitmix64
    let mut z = k.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// The engine: a task of 10 ms, its inputs read from a feed that takes 0 to 12 ms of the task's virtual time per
// read, so scans overrun and skip (none past twice the period), and every write logged by its time.

struct Feed<'a, I, O> {
    clock: VirtualClock,
    reads: u64,
    make: &'a dyn Fn(u64) -> I,
    writes: Vec<(Duration, O)>,
}

impl<I, O: Copy> Io<I, O> for Feed<'_, I, O> {
    fn read_inputs(&mut self, _now: Duration) -> I {
        let k = self.reads;
        self.reads += 1;
        self.clock.advance(Duration::from_millis(k.wrapping_mul(7919) % 13));
        (self.make)(k)
    }

    fn write_outputs(&mut self, now: Duration, outputs: &O) {
        self.writes.push((now, *outputs));
    }
}

fn config() -> TaskConfig {
    TaskConfig { name: "conformance", period: Duration::from_millis(10) }
}

/// What a report says of a scan, but the program and its state.
type Seen<O> = (u64, Duration, Duration, bool, O, bool);

fn seen<T: TaskProgram>(task: &mut Task<T>, feed: &mut Feed<'_, T::Inputs, T::Outputs>, clock: &mut VirtualClock) -> Seen<T::Outputs>
where
    T::Outputs: Copy,
{
    let r = task.step(feed, clock);
    (r.scan.cycle, r.scan.now, r.exec, r.time_error, *r.outputs, r.state == RunState::Running)
}

/// The regions the scan entry takes, read from its parameters: `&mut P`, `&Scan`, `&I`, `&mut O`.
fn regions(code: &Code) -> Regions {
    let scan = &code.functions[0];
    let to = |l: usize| match code.types[scan.locals[l].ty.0 as usize] {
        Type::Ref { to, .. } => to,
        ref other => panic!("parameter {l} of the scan is {other:?}"),
    };
    Regions { state: to(1), scan: to(2), inputs: to(3), outputs: to(4) }
}

fn load(code: &Code, regions: &[TypeId]) -> Machine {
    Machine::load(code.clone(), regions, LIMITS).unwrap()
}

/// The first state: the program's `<P as Default>::default`, called on a machine.
fn first_state(code: &Code) -> Vec<Scalar> {
    load(code, &[]).call(FunctionId(1), &[]).unwrap().to_vec()
}

/// The safe outputs: the program's `<P as Program>::safe_outputs`, called on a machine holding the first state.
fn safe_outputs<O: Image>(code: &Code, state_ty: TypeId, state: &[Scalar]) -> O {
    let mut machine = load(code, &[state_ty]);
    machine.set_region(RegionId(0), state).unwrap();
    let slots = machine.call(FunctionId(2), &[Arg::Ref(RegionId(0))]).unwrap().to_vec();
    O::read(&slots).unwrap()
}

/// Runs `name` natively and lowered on the engine for `scans` scans of `inputs`, every report and write alike, and
/// scans that overran and deadlines skipped among them. The native program's outputs, scan by scan, are returned for
/// the test to say the run reached what it is to prove, with the instructions the lowered program took in all.
fn conform<P, I, O>(name: &str, native: P, state_name: &str, inputs: &dyn Fn(u64) -> I, scans: u64) -> (Vec<O>, u64)
where
    P: Program<Inputs = I, Outputs = O>,
    I: Image + Debug,
    O: Image + PartialEq + Debug,
{
    let lowered = lower(name, &program(name));
    let (header, code) = lowered.result.unwrap_or_else(|e| panic!("{name} was not lowered: {e}"));
    assert_eq!(header, Header { language: orcl_std::LANGUAGE, nightly: nightly().into(), revision: REVISION.into() });
    let entries: Vec<&str> = code.functions.iter().take(3).map(|f| f.name.as_str()).collect();
    assert_eq!(
        entries,
        [
            format!("<{state_name} as Program>::scan"),
            format!("<{state_name} as Default>::default"),
            format!("<{state_name} as Program>::safe_outputs"),
        ],
        "{name}: the entries, first and in their order"
    );
    assert!(code.files.iter().any(|f| Path::new(f) == program(name)), "{name}: its source in the source map");

    let regions = regions(&code);
    let state = first_state(&code);
    let safe: O = safe_outputs(&code, regions.state, &state);
    assert_eq!(safe, native.safe_outputs(), "{name}: the safe outputs");
    let twin = Interpreted::<I, O>::new(code, FunctionId(0), regions, &state, safe, LIMITS).unwrap();

    let (mut native_clock, mut twin_clock) = (VirtualClock::new(), VirtualClock::new());
    let mut native_feed = Feed { clock: native_clock.clone(), reads: 0, make: inputs, writes: Vec::new() };
    let mut twin_feed = Feed { clock: twin_clock.clone(), reads: 0, make: inputs, writes: Vec::new() };
    let mut native_task = Task::new(config(), native).unwrap();
    let mut twin_task = Task::new(config(), twin).unwrap();
    let (mut instructions, mut most) = (0u64, 0u64);
    for k in 0..scans {
        let a = seen(&mut native_task, &mut native_feed, &mut native_clock);
        let b = seen(&mut twin_task, &mut twin_feed, &mut twin_clock);
        assert_eq!(a, b, "{name}, scan {k}");
        assert!(a.5, "{name}, scan {k}: still running");
        let steps = twin_task.program().machine().steps();
        instructions += steps;
        most = most.max(steps);
    }
    assert_eq!(native_feed.writes, twin_feed.writes, "{name}: every write, by its time and value");
    assert_eq!(native_task.stats(), twin_task.stats(), "{name}: the cycle statistics");
    let stats = native_task.stats();
    assert!(stats.overruns > 0 && stats.skipped > 0, "{name}: the engine's run met overruns and skips: {stats:?}");
    assert!(instructions > 0);
    let record = format!("{name}: {scans} scans alike, {instructions} instructions in all, {most} at most in one scan\n");
    print!("{record}");
    fs::write(lowered.file.with_extension("instructions"), record).unwrap();
    (native_feed.writes.into_iter().map(|(_, o)| o).collect(), instructions)
}

/// Asserts that a run's outputs held `what` true on some scan and false on another.
fn both<O>(outputs: &[O], what: &str, f: impl Fn(&O) -> bool) {
    let trues = outputs.iter().filter(|o| f(o)).count();
    assert!(trues > 0 && trues < outputs.len(), "{what}: true on {trues} of {} scans", outputs.len());
}

#[test]
fn the_conveyor_runs_lowered_as_it_runs_natively() {
    let inputs = |k: u64| {
        let phase = k % 1_700;
        conveyor::Inputs {
            start_pb: mix(k / 4, 1) % 40 == 0,
            stop_pb: mix(k / 4, 2) % 90 == 0,
            ack_pb: mix(k / 4, 3) % 25 == 0,
            // the door opened long enough to raise the alarm once a cycle of 17 s, and flickering short of it
            door_ch1: !(1_100..1_200).contains(&phase) && mix(k / 6, 4) % 60 != 0,
            door_ch2: !(1_400..1_430).contains(&phase),
            part_at_end: mix(k / 3, 5) % 9 == 0,
        }
    };
    let (outputs, instructions) = conform("conveyor", conveyor::Conveyor::default(), "Conveyor", &inputs, 6_000);
    both(&outputs, "the motor", |o| o.motor_on);
    both(&outputs, "the door's alarm", |o| o.door_alarm);
    both(&outputs, "the running lamp", |o| o.lamp_running);
    both(&outputs, "the fault lamp", |o| o.lamp_fault);
    assert!(outputs.iter().map(|o| o.parts).max() >= Some(10), "parts counted");
    assert_eq!(instructions, INSTRUCTIONS_CONVEYOR, "the conveyor's instructions, as the verification records them");
}

// The instructions the verification records (C-156), every crate compiled at MIR opt-level 0: a change to the
// program, to `orcl-std`, to the lowering or to the nightly moves them, and the clause's figure moves with them.
const INSTRUCTIONS_CONVEYOR: u64 = 998_579;
const INSTRUCTIONS_PROBE: u64 = 2_539_645;

#[test]
fn the_probe_of_every_block_of_orcl_std_runs_lowered_as_it_runs_natively() {
    let inputs = |k: u64| probe::Inputs {
        a: mix(k / 5, 1) % 3 != 0,
        b: mix(k / 3, 2) % 2 == 0,
        reset: mix(k, 3) % 150 == 0,
        preset: (mix(k / 200, 4) % 40) as i32 - 5,
        pt_ms: 20 + mix(k / 100, 5) % 300,
        bit: (mix(k, 6) % 70) as u32,
        value: mix(k, 7) % 2 == 0,
        w8: mix(k, 8) as u8,
        w16: mix(k, 9) as u16,
        w32: mix(k, 10) as u32,
        w64: mix(k, 11),
    };
    let (outputs, instructions) = conform("probe", probe::Probe::default(), "Probe", &inputs, 6_000);
    both(&outputs, "the on-delay", |o| o.ton);
    both(&outputs, "the off-delay", |o| o.tof);
    both(&outputs, "the pulse", |o| o.tp);
    both(&outputs, "the on-delay's time below its preset", |o| o.ton_et > Duration::ZERO && !o.ton);
    both(&outputs, "the off-delay's time running", |o| o.tof_et > Duration::ZERO && o.tof);
    both(&outputs, "the pulse's time running", |o| o.tp_et > Duration::ZERO && o.tp);
    both(&outputs, "the rising edge", |o| o.rising);
    both(&outputs, "the falling edge", |o| o.falling);
    both(&outputs, "the counter's output", |o| o.counted);
    both(&outputs, "the counter reset", |o| o.cv == 0);
    both(&outputs, "a bit of a u8", |o| o.get8);
    both(&outputs, "a bit of a u64", |o| o.get64);
    both(&outputs, "a bit written into a u16", |o| o.set16 != 0);
    assert_eq!(instructions, INSTRUCTIONS_PROBE, "the probe's instructions, as the verification records them");
}

fn arith_inputs(k: u64) -> arith::Inputs {
    let b = (mix(k, 2) % 200) as i32 - 100;
    arith::Inputs {
        a: (mix(k, 1) % 20_001) as i32 - 10_000,
        b: if b >= 0 { b + 1 } else { b },
        c: mix(k, 3) as u8,
        d: mix(k, 4) as u16,
        e: (mix(k, 5) % 2_000_000_001) as i64 - 1_000_000_000,
        f: mix(k, 6),
        k: mix(k, 7) as i8,
        sh: (mix(k, 8) % 80) as u32,
        flag: mix(k, 9) % 2 == 0,
        x: ((mix(k, 10) % 2_000_001) as f32 - 1.0e6) / 7.0,
        y: ((mix(k, 11) % 2_000_000_000_001) as f64 - 1.0e12) / 3.0,
    }
}

#[test]
fn arithmetic_bits_comparisons_and_casts_run_lowered_as_they_run_natively() {
    let (outputs, _) = conform("arith", arith::Arith::default(), "Arith", &arith_inputs, 4_000);
    both(&outputs, "a < b", |o| o.less);
    both(&outputs, "a shift left within the width", |o| o.shl_ok);
    both(&outputs, "a shift right past the width", |o| o.shr_ok);
    both(&outputs, "a saturated addition", |o| o.sat_add == u8::MAX);
    both(&outputs, "a float saturating into a u8 at its top", |o| o.double_to_u8 == u8::MAX);
    both(&outputs, "a float saturating into a u8 at zero", |o| o.double_to_u8 == 0);
    both(&outputs, "a float below a half", |o| o.fless);
}

#[test]
fn enums_arrays_options_tuples_references_durations_and_constants_run_lowered_as_natively() {
    let inputs = |k: u64| shapes::Inputs {
        sel: (mix(k, 1) % 11) as u8,
        val: mix(k, 2) as u8,
        idx: mix(k, 3) as u8,
        flag: mix(k, 4) % 3 == 0,
        t_ms: mix(k, 5) % 2_000,
    };
    let (outputs, _) = conform("shapes", shapes::Shapes::default(), "Shapes", &inputs, 4_000);
    both(&outputs, "the mode off", |o| o.mode == shapes::Mode::Off);
    both(&outputs, "the mode held", |o| matches!(o.mode, shapes::Mode::Hold(_)));
    both(&outputs, "the mode running forward", |o| o.forward);
    both(&outputs, "the mode running back", |o| matches!(o.mode, shapes::Mode::Run { forward: false, .. }));
    both(&outputs, "a fault", |o| matches!(o.mode, shapes::Mode::Fault(_)));
    both(&outputs, "a value held", |o| o.opt_some);
    both(&outputs, "a time held", |o| o.held > Duration::ZERO);
    both(&outputs, "a duration below a second", |o| o.dur_lt);
    for niche in 1..=3 {
        both(&outputs, &format!("the niche's variant {niche}"), |o| o.niche == niche);
    }
    both(&outputs, "a level", |o| o.level == 200);
}

#[test]
fn comparisons_at_their_edges_signed_matches_and_constants_of_every_layout_run_lowered_as_natively() {
    // `>`, `<=` and `!=` with their operands equal as often as not; a match on negative values; a negative
    // discriminant cast and matched; a niche holding two variants; `[first, .., last]`; a generic function; promoted
    // references at an offset into a constant; a constant enum of one variant
    let inputs = |k: u64| edges::Inputs {
        a: (mix(k, 1) % 9) as i16 - 4,
        b: (mix(k, 2) % 9) as i16 - 4,
        n: mix(k, 3) as u8,
        flag: mix(k, 4) % 2 == 0,
    };
    let (outputs, _) = conform("edges", edges::Edges::default(), "Edges", &inputs, 2_000);
    both(&outputs, "a > b", |o| o.gt);
    both(&outputs, "a <= b", |o| o.le);
    both(&outputs, "a == b", |o| o.eq);
    for v in 1..=5 {
        both(&outputs, &format!("the signed match's arm {v}"), |o| o.signed_match == v);
    }
    for v in [10, 20, 30] {
        both(&outputs, &format!("the direction's arm {v}"), |o| o.dir_match == v);
    }
    for v in 1..=4 {
        both(&outputs, &format!("the light's variant {v}"), |o| o.light == v);
    }
    both(&outputs, "the generic's second argument", |o| o.picked == 7);
    assert!(outputs.iter().all(|o| o.off_ref == 600 && o.off_ref2 == 600 && o.off_ref3 == 3_333 && o.only == 42));
}

#[test]
fn ordinary_control_code_runs_lowered_as_natively() {
    // matches on tuples, with guards and nested; a loop's value; labeled breaks; early returns; an inherent method
    // and a derived `Clone`; an `Option` of an enum with data; a time held since a first scan
    let inputs = |k: u64| sound::Inputs {
        a: mix(k, 1) as i8,
        b: mix(k, 2),
        sel: mix(k, 3) as u8,
        on: mix(k, 4) % 2 == 0,
        off: mix(k, 5) % 3 == 0,
        t_ms: mix(k, 6) % 3_000,
    };
    let (outputs, _) = conform("sound", sound::Sound::default(), "Sound", &inputs, 3_000);
    for v in 1..=4 {
        both(&outputs, &format!("the guard's arm {v}"), |o| o.guard == v);
        both(&outputs, &format!("the tuple's arm {v}"), |o| o.tuple_match == v);
    }
    both(&outputs, "the nested match's guarded arm", |o| o.nested == 1);
    both(&outputs, "a time held past its preset", |o| o.since_ms);
}

/// Every constant a lowered program's statements and terminators hold.
fn constants(code: &Code) -> impl Iterator<Item = &Const> {
    let blocks = code.functions.iter().flat_map(|f| f.blocks.iter());
    blocks.flat_map(|b| {
        let mut operands: Vec<&Operand> = Vec::new();
        for s in &b.statements {
            match &s.value {
                Value::Use(o) | Value::Unary(_, o) | Value::Cast(o, _) => operands.push(o),
                Value::Binary(_, x, y) | Value::Checked(_, x, y) => operands.extend([x, y]),
                Value::Aggregate(_, _, parts) => operands.extend(parts),
                Value::Discriminant(_) | Value::Ref(..) => {}
            }
        }
        match &b.terminator {
            Terminator::SwitchInt { value: o, .. } | Terminator::Assert { cond: o, .. } => operands.push(o),
            Terminator::Call { args, .. } => operands.extend(args),
            Terminator::Goto(_) | Terminator::Return { .. } | Terminator::Unreachable { .. } => {}
        }
        operands.into_iter().filter_map(|o| match o {
            Operand::Const(c) => Some(c),
            _ => None,
        })
    })
}

#[test]
fn constants_of_no_byte_hold_a_one_variant_enum_s_discriminant_and_run_lowered_as_natively() {
    // `const ONLY: Only = Only::One`, an array, a structure and a tuple of such: rustc evaluates each to a constant of
    // no byte; the interpreter holds each such enum in one slot, its discriminant
    let (_, code) = lower("zst_const", &program("zst_const")).result.unwrap_or_else(|e| panic!("zst_const: {e}"));
    let only = code.types.iter().position(|t| matches!(t, Type::Enum { name, .. } if name == "Only")).unwrap();
    let constants: Vec<&Const> =
        constants(&code).filter(|c| !c.slots.is_empty() && c.slots.iter().all(|s| *s == discriminant(0))).collect();
    let mut sizes: Vec<usize> = constants.iter().map(|c| c.slots.len()).collect();
    sizes.sort_unstable();
    sizes.dedup();
    assert_eq!(sizes, [1, 2, 3], "an enum, a structure and a tuple of two, an array of three: {constants:?}");
    assert!(constants.iter().any(|c| c.ty.0 as usize == only), "the enum's own constant");
    let inputs = |k: u64| zst_const::Inputs { a: mix(k, 1) as u8 };
    let (outputs, _) = conform("zst_const", zst_const::ZstConst::default(), "ZstConst", &inputs, 200);
    both(&outputs, "an odd sum", |o| o.v % 2 == 1);
}

#[test]
fn a_promoted_reference_to_a_fieldless_enum_runs_lowered_as_natively() {
    let inputs = |k: u64| zst_ref::Inputs { a: mix(k, 1) as u8 };
    let (outputs, _) = conform("zst_ref", zst_ref::ZstRef::default(), "ZstRef", &inputs, 200);
    both(&outputs, "an odd sum", |o| o.v % 2 == 1);
}

#[test]
fn two_instances_of_a_const_generic_block_run_lowered_as_natively() {
    // `Avg<4>` and `Avg<8>`: two types, of four and eight elements, one generic code
    let inputs = |k: u64| cgen::Inputs { v: (mix(k, 1) % 1_000) as u16 };
    let (outputs, _) = conform("cgen", cgen::Cgen::default(), "Cgen", &inputs, 400);
    both(&outputs, "the two averages apart", |o| o.short != o.long);
}

#[test]
fn a_place_behind_two_references_runs_lowered_as_natively() {
    let inputs = |k: u64| deref2::Inputs { a: mix(k, 1) as u8 };
    let (outputs, _) = conform("deref2", deref2::Deref2::default(), "Deref2", &inputs, 200);
    both(&outputs, "a value of four times an even input", |o| o.v % 8 == 0);
}

#[test]
fn constants_whose_fields_rustc_reorders_run_lowered_as_natively() {
    // each field of a `(u8, u16, bool)` and of a `{ u8, u32, u16 }` read on its own, not summed
    let inputs = |k: u64| order::Inputs { sel: (mix(k, 1) % 8) as u8 };
    let (outputs, _) = conform("order", order::Order::default(), "Order", &inputs, 200);
    assert!(outputs.iter().all(|o| (o.p1, o.a, o.b, o.c) == (300, 9, 70_000, 513)));
    both(&outputs, "the tuple's bool", |o| o.p2);
}

#[test]
fn constant_enums_padded_past_an_array_or_a_tuple_run_lowered_as_natively() {
    // the slots an enum's smaller variant leaves unwritten count its largest variant's array or tuple whole
    let inputs = |k: u64| padded::Inputs { sel: mix(k, 1) as u8 };
    let (outputs, _) = conform("padded", padded::Padded::default(), "Padded", &inputs, 200);
    both(&outputs, "the idle command", |o| o.cmd == 1);
    both(&outputs, "the pair", |o| o.pair == 515);
    both(&outputs, "the recipe", |o| o.recipe == 100);
}

#[test]
fn constants_in_a_duration_s_niche_and_a_signed_enum_s_run_lowered_as_natively() {
    let inputs = |k: u64| niches::Inputs { sel: mix(k, 1) as u8 };
    let (outputs, _) = conform("niches", niches::Niches::default(), "Niches", &inputs, 200);
    both(&outputs, "a time held", |o| o.has && o.held == Duration::from_millis(1_500));
    both(&outputs, "back", |o| o.dir == -1);
    both(&outputs, "none", |o| o.dir == 9);
    both(&outputs, "the long step", |o| o.step == Duration::from_secs(3));
}

fn edge_i64(k: u64) -> i64 {
    [i64::MIN, -1, 0, i64::MAX, 5, -77][(mix(k, 21) % 6) as usize]
}

fn edge_u64(k: u64) -> u64 {
    [u64::MAX, 1 << 63, 0, 12_345, mix(k, 22)][(mix(k, 23) % 5) as usize]
}

#[test]
fn repr_enums_wide_constants_and_edge_switches_run_lowered_as_natively() {
    let inputs = |k: u64| p_repr::Inputs {
        sel: mix(k, 1) as u8,
        a: edge_i64(k),
        b: edge_u64(k),
        c: [-128i8, -1, 127, 0, 5][(mix(k, 3) % 5) as usize],
        d: [255u16, 128, 0, 256, 511, 7][(mix(k, 4) % 6) as usize],
    };
    let (outputs, _) = conform("p_repr", p_repr::PRepr::default(), "PRepr", &inputs, 600);
    for v in 1..=4 {
        both(&outputs, &format!("big {v}"), |o| o.big_match == v);
        both(&outputs, &format!("sw8 {v}"), |o| o.sw8 == v);
        both(&outputs, &format!("sw_i8 {v}"), |o| o.sw_i8 == v);
    }
    for v in 1..=5 {
        both(&outputs, &format!("neg {v}"), |o| o.neg_match == v);
    }
}

#[test]
fn traits_generics_casts_and_shifts_run_lowered_as_natively() {
    let inputs = |k: u64| p_traits::Inputs {
        n: mix(k, 1) as u8,
        p: mix(k, 2) % 2 == 0,
        q: mix(k, 3) % 3 == 0,
        a8: mix(k, 4) as i8,
        u64v: edge_u64(k),
        i64v: edge_i64(k),
        u32v: mix(k, 7) as u32,
        i16v: mix(k, 8) as i16,
    };
    let (outputs, _) = conform("p_traits", p_traits::PTraits::default(), "PTraits", &inputs, 600);
    both(&outputs, "p < q", |o| o.blt);
    both(&outputs, "let else", |o| o.got == 99);
}

#[test]
fn a_program_s_own_duration_and_option_run_lowered_as_natively() {
    // core's `Duration` and `Option` known by their definitions: a program's own of those names lowers as any
    // structure and enum of the program's
    let inputs = |k: u64| p_names::Inputs { a: mix(k, 1) as u16, sel: mix(k, 2) as u8, t_ms: mix(k, 3) % 3_000 };
    let (outputs, _) = conform("p_names", p_names::PNames::default(), "PNames", &inputs, 400);
    both(&outputs, "core's Option some", |o| o.core_some);
    both(&outputs, "the program's Option's pair", |o| o.opt == 209);
}

#[test]
fn constants_of_a_niche_that_wraps_past_its_tag_s_top_run_lowered_as_natively() {
    let inputs = |k: u64| p_wrap::Inputs { sel: mix(k, 1) as u8 };
    let (outputs, _) = conform("p_wrap", p_wrap::PWrap, "PWrap", &inputs, 200);
    for v in 1..=4 {
        both(&outputs, &format!("arm {v}"), |o| o.v == v);
    }
}

#[test]
fn more_runs_lowered_as_natively() {
    // floats at their edges (NaN, the infinities, -0.0, past f32's range), `Duration::MAX` and a constant past 2^32 s,
    // a `repr(u8)` enum with data, constant enums padded past a structure, nested options in a bool's niche
    let xs = [f32::NAN, 1.5, -0.0, f32::INFINITY, 3.0e38, -7.25, f32::NEG_INFINITY, 0.0];
    let ys = [1.0e10, -2.5, 0.0, 1.0e-300, -1.0e9];
    let inputs = |k: u64| more::Inputs {
        a: mix(k, 1) as i16,
        b: mix(k, 2) as u8,
        x: xs[(mix(k, 3) % 8) as usize],
        y: ys[(mix(k, 4) % 5) as usize],
        sel: mix(k, 5) as u8,
    };
    let (outputs, _) = conform("more", more::More::default(), "More", &inputs, 600);
    both(&outputs, "forever", |o| o.forever == Duration::MAX);
    both(&outputs, "far", |o| o.far == Duration::from_secs(5_000_000_000));
    both(&outputs, "a NaN unequal to itself", |o| o.self_ne);
    both(&outputs, "an infinity narrowed", |o| o.narrowed.is_infinite());
    both(&outputs, "a NaN cast to zero, an infinity saturated", |o| o.to_i32 == i32::MAX);
    both(&outputs, "the setpoint run", |o| o.setv == 51);
    both(&outputs, "the setpoint held", |o| o.setv == 5);
    for v in 1..=4 {
        both(&outputs, &format!("oob {v}"), |o| o.oob == v);
    }
    both(&outputs, "an arithmetic shift of a negative value", |o| o.sar < 0);
}

#[test]
fn refs_runs_lowered_as_natively() {
    // places behind references and in enums: a `ref mut` into an option, a block's method through an array
    // element's field, a constant structure of options matched
    let inputs = |k: u64| refs::Inputs { a: mix(k, 1) as u8, b: mix(k, 2) as u8, on: mix(k / 4, 3) % 3 != 0 };
    let (outputs, _) = conform("refs", refs::Refs::default(), "Refs", &inputs, 1_000);
    both(&outputs, "counted", |o| o.counted == 255);
    both(&outputs, "a time held", |o| o.since > Duration::ZERO);
    both(&outputs, "a station's timer", |o| o.timers > 0);
    both(&outputs, "the held configuration", |o| o.cfg == 300);
    both(&outputs, "the running configuration", |o| o.cfg == 410);
}

#[test]
fn trait_runs_lowered_as_natively() {
    // a trait of the program's own named `Program`, implemented beside orcl-std's: orcl-std's implementation alone
    // is the program
    let inputs = |k: u64| own_trait::Inputs { a: mix(k, 1) as u8 };
    let (outputs, _) = conform("own_trait", own_trait::OwnTrait::default(), "OwnTrait", &inputs, 200);
    both(&outputs, "odd", |o| o.v % 2 == 1);
}

#[test]
fn the_driver_takes_its_options_from_the_environment_as_a_wrapper_and_lowers_no_test_build() {
    // as `RUSTC_WRAPPER` (the compiler it wraps first, then rustc's arguments), the program named by the
    // environment: the file is written as with the options
    let work = fresh("wrapper");
    let w = work.display().to_string();
    let std_src = here().join("../orcl-std/src/lib.rs").display().to_string();
    let out = driver(&strings(&["/usr/bin/rustc", "--crate-name", "orcl_std", "--crate-type", "rlib", "--edition", "2024", &std_src, "--out-dir", &w]));
    assert!(out.status.success(), "orcl-std: {}", String::from_utf8_lossy(&out.stderr));
    let file = work.join("arith.orclb");
    let source = program("arith").display().to_string();
    let extern_std = format!("orcl_std={w}/liborcl_std.rlib");
    let mut args = strings(&["/usr/bin/rustc", "--crate-name", "arith", "--crate-type", "rlib", "--edition", "2024", &source]);
    args.extend(strings(&["--extern", &extern_std, "-L", &w, "--out-dir", &w]));
    let path = file.display().to_string();
    let env = [("ORCL_FRONT_PROGRAM", "arith"), ("ORCL_FRONT_OUT", path.as_str()), ("ORCL_FRONT_REVISION", "from-env")];
    let out = driver_with(&args, &env);
    assert!(out.status.success(), "arith: {}", String::from_utf8_lossy(&out.stderr));
    let (header, code) = file::read(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(header.revision, "from-env");
    let with_options = lower_in("wrapper-options", "arith", &program("arith")).result.unwrap().1;
    assert_eq!(code, with_options, "the same program as with the options");

    // the program's crate built as its own tests is compiled, and nothing is lowered or written
    fs::remove_file(&file).unwrap();
    let mut args = strings(&["--crate-name", "arith", "--test", "--edition", "2024", &source]);
    args.extend(strings(&["--extern", &extern_std, "-L", &w, "--out-dir", &w]));
    let out = driver_with(&args, &env);
    assert!(out.status.success(), "arith's tests: {}", String::from_utf8_lossy(&out.stderr));
    assert!(!file.exists(), "a test build is not lowered");

    // the program named, but no file to write: refused before rustc runs
    let args = strings(&["--crate-name", "arith", "--crate-type", "rlib", &source, "--orcl-program=arith", "--orcl-revision", "x"]);
    let out = driver(&args);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: orcl-front: usage: --orcl-out names no file to write"));
}

#[test]
fn the_environment_lowers_only_the_crate_it_names() {
    // as `RUSTC_WRAPPER` under cargo every crate of the build sees the environment: a crate not the program is
    // compiled, and nothing is lowered or written
    let work = fresh("env-other");
    let file = work.join("arith.orclb");
    let path = file.display().to_string();
    compile_std(&work, &[("ORCL_FRONT_PROGRAM", "arith"), ("ORCL_FRONT_OUT", path.as_str()), ("ORCL_FRONT_REVISION", "env")]);
    assert!(work.join("liborcl_std.rlib").exists(), "orcl-std compiled");
    assert!(!file.exists(), "no file written for a crate not the program");
}

#[test]
fn an_option_wins_over_the_environment() {
    let work = fresh("precedence");
    compile_std(&work, &[]);
    let w = work.display().to_string();
    let file = work.join("arith.orclb");
    let source = program("arith").display().to_string();
    let mut args = strings(&["--crate-name", "arith", "--crate-type", "rlib", "--edition", "2024", &source]);
    args.extend(strings(&["--extern", &format!("orcl_std={w}/liborcl_std.rlib"), "-L", &w, "--out-dir", &w]));
    args.extend(strings(&["--orcl-program", "arith", &format!("--orcl-out={}", file.display()), "--orcl-revision=option"]));
    let out = driver_with(&args, &[("ORCL_FRONT_REVISION", "env")]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (header, _) = file::read(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(header.revision, "option");
}

#[test]
fn the_driver_refuses_a_program_named_with_no_revision() {
    let source = program("lines").display().to_string();
    let mut args = strings(&["--crate-name", "lines", "--crate-type", "rlib", &source]);
    args.extend(strings(&["--orcl-program=lines", "--orcl-out=/nonexistent/x.orclb"]));
    let out = driver(&args);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: orcl-front: usage: --orcl-revision names no revision"));
}

#[test]
fn the_driver_refuses_an_option_given_twice() {
    let source = program("lines").display().to_string();
    let mut args = strings(&["--crate-name", "lines", "--crate-type", "rlib", &source]);
    args.extend(strings(&["--orcl-program=lines", "--orcl-revision", "x"]));
    args.extend(strings(&["--orcl-out=/nonexistent/a.orclb", "--orcl-out", "/nonexistent/b.orclb"]));
    let out = driver(&args);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: orcl-front: usage: --orcl-out given twice"));
}

#[test]
fn the_driver_refuses_an_option_given_last_with_no_value() {
    let source = program("lines").display().to_string();
    let mut args = strings(&["--crate-name", "lines", "--crate-type", "rlib", &source]);
    args.extend(strings(&["--orcl-out=/nonexistent/x.orclb", "--orcl-revision=x", "--orcl-program"]));
    let out = driver(&args);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: orcl-front: usage: --orcl-program needs a value"));
}

#[test]
fn the_driver_reads_a_crate_name_given_with_an_equals_sign() {
    let work = fresh("crate-name-equals");
    compile_std(&work, &[]);
    let w = work.display().to_string();
    let file = work.join("lines.orclb");
    let source = program("lines").display().to_string();
    let mut args = strings(&["--crate-name=lines", "--crate-type", "rlib", "--edition", "2024", &source]);
    args.extend(strings(&["--extern", &format!("orcl_std={w}/liborcl_std.rlib"), "-L", &w, "--out-dir", &w]));
    args.extend(strings(&["--orcl-program", "lines", &format!("--orcl-out={}", file.display()), "--orcl-revision", "eq"]));
    let out = driver(&args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(file.exists(), "the program named by `--crate-name=` lowered");
}

#[test]
fn the_driver_drops_a_wrapped_rustc_exe() {
    // as `RUSTC_WRAPPER` of a compiler named `rustc.exe`, its path dropped as `/usr/bin/rustc`'s is
    let work = fresh("wrapper-exe");
    let w = work.display().to_string();
    let std_src = here().join("../orcl-std/src/lib.rs").display().to_string();
    let mut args = strings(&["/opt/x/bin/rustc.exe", "--crate-name", "orcl_std", "--crate-type", "rlib", "--edition", "2024"]);
    args.extend(strings(&[&std_src, "--out-dir", &w]));
    let out = driver(&args);
    assert!(out.status.success(), "as the wrapper of rustc.exe: {}", String::from_utf8_lossy(&out.stderr));
    assert!(work.join("liborcl_std.rlib").exists(), "orcl-std compiled");
}

#[test]
fn an_output_path_holding_an_equals_sign_is_written_whole() {
    // `--orcl-out=<path>` cut at its first `=`, never its last
    let work = fresh("out-equals");
    compile_std(&work, &[]);
    let w = work.display().to_string();
    let dir = work.join("a=b");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lines.orclb");
    let source = program("lines").display().to_string();
    let mut args = strings(&["--crate-name", "lines", "--crate-type", "rlib", "--edition", "2024", &source]);
    args.extend(strings(&["--extern", &format!("orcl_std={w}/liborcl_std.rlib"), "-L", &w, "--out-dir", &w]));
    args.extend(strings(&["--orcl-program", "lines", &format!("--orcl-out={}", file.display()), "--orcl-revision", "eq"]));
    let out = driver(&args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(file.exists(), "the file at the path given");
}

#[test]
fn under_cargo_a_bin_of_the_program_s_name_is_compiled_and_the_library_alone_lowered() {
    // cargo, the driver its `RUSTC_WRAPPER` and the environment naming the program, builds a package whose library is
    // the program and whose bin, of the same crate name, uses it: the library is lowered and its file written, the bin
    // compiled and not lowered, and the build succeeds
    let work = fresh("package");
    let package = work.join("lines");
    fs::create_dir_all(package.join("src")).unwrap();
    fs::copy(program("lines"), package.join("src/lib.rs")).unwrap();
    fs::write(package.join("src/main.rs"), "fn main() {\n    let _ = lines::Lines::default();\n}\n").unwrap();
    let orcl_std = here().join("../orcl-std").display().to_string();
    let manifest = format!(
        "[package]\nname = \"lines\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\norcl-std = {{ path = {orcl_std:?} }}\n\n[workspace]\n"
    );
    fs::write(package.join("Cargo.toml"), manifest).unwrap();
    let file = work.join("lines.orclb");
    let out = Command::new(env!("CARGO"))
        .args(["build", "--offline", "--manifest-path"])
        .arg(package.join("Cargo.toml"))
        // a cargo home and a target of the package's own: no other build's lock, cache or configuration
        .env("CARGO_HOME", work.join("cargo-home"))
        .env("CARGO_TARGET_DIR", work.join("target"))
        .env("RUSTUP_TOOLCHAIN", nightly())
        .env("RUSTC_WRAPPER", env!("CARGO_BIN_EXE_orcl-front-driver"))
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env("LD_LIBRARY_PATH", library_path())
        .env("ORCL_FRONT_PROGRAM", "lines")
        .env("ORCL_FRONT_OUT", &file)
        .env("ORCL_FRONT_REVISION", "cargo")
        .output()
        .unwrap();
    assert!(out.status.success(), "the package's build: {}", String::from_utf8_lossy(&out.stderr));
    assert!(work.join("target/debug/lines").exists(), "the bin built");
    let (header, code) = file::read(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(header.revision, "cargo");
    let alone = lower_in("package-alone", "lines", &program("lines")).result.unwrap().1;
    assert_eq!((&code.types, &code.functions), (&alone.types, &alone.functions), "the program, as lowered alone");
}

/// The 1-based line and column where `snippet` first stands in `source`.
fn place_of(source: &Path, snippet: &str) -> (usize, usize) {
    let text = fs::read_to_string(source).unwrap();
    let at = text.find(snippet).unwrap_or_else(|| panic!("{snippet:?} not in {}", source.display()));
    let before = &text[..at];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

/// The program error a lowered program's scan stops at, called once on a machine holding its first state, a scan at
/// time zero, `inputs` and the outputs' default.
fn fault<I: Image + Debug, O: Image + Default>(code: &Code, inputs: &I) -> ProgramError {
    let regions = regions(code);
    let state = first_state(code);
    let mut machine = load(code, &[regions.state, regions.scan, regions.inputs, regions.outputs]);
    let mut slots = Vec::new();
    machine.set_region(RegionId(0), &state).unwrap();
    slots.extend([Scalar::Int(Int::U64, 0), Scalar::Int(Int::U32, 0), Scalar::Int(Int::U64, 0), Scalar::Int(Int::U32, 0)]);
    slots.push(Scalar::Int(Int::U64, 0));
    machine.set_region(RegionId(1), &slots).unwrap();
    slots.clear();
    inputs.write(&mut slots);
    machine.set_region(RegionId(2), &slots).unwrap();
    slots.clear();
    O::default().write(&mut slots);
    machine.set_region(RegionId(3), &slots).unwrap();
    let args = [0, 1, 2, 3].map(|r| Arg::Ref(RegionId(r)));
    match machine.call(FunctionId(0), &args) {
        Err(orcl_interp::CallError::Program(e)) => e,
        other => panic!("{inputs:?} ran: {other:?}"),
    }
}

#[test]
fn a_scan_s_program_error_names_its_place_in_the_source_map() {
    // a divisor of zero: the native program would panic; the lowered one fails at the division, by file, line and
    // column, the machine standing at the assertion
    let (_, code) = lower_in("arith-fault", "arith", &program("arith")).result.unwrap();
    let mut zero = arith_inputs(5);
    zero.b = 0;
    let error = fault::<_, arith::Outputs>(&code, &zero);
    assert_eq!(error.kind, ErrorKind::Check(Check::DivisionByZero));
    let (line, column) = place_of(&program("arith"), "i.a / i.b");
    assert_eq!(code.files[error.at.file as usize], program("arith").display().to_string());
    assert_eq!((error.at.line as usize, error.at.column as usize), (line, column), "the division's place");
}

#[test]
fn every_check_a_scan_fails_names_its_kind_and_place() {
    // each of MIR's run-time checks, failed on its own selector: the fault's kind, and the file, line and column of
    // the operation it checks
    let (_, code) = lower("faults", &program("faults")).result.unwrap();
    let base = faults::Inputs { sel: 0, a: 0, b: 0, idx: 0, c: 1, sh: 0 };
    let cases: [(faults::Inputs, Check, &str); 8] = [
        (faults::Inputs { sel: 0, a: 5, b: 0, ..base }, Check::RemainderByZero, "i.a % i.b"),
        (faults::Inputs { sel: 1, a: i32::MAX, b: 1, ..base }, Check::Overflow(BinOp::Add), "i.a + i.b"),
        (faults::Inputs { sel: 2, a: i32::MIN, ..base }, Check::NegOverflow, "-i.a"),
        (faults::Inputs { sel: 3, idx: 9, ..base }, Check::Bounds, "self.table[i.idx as usize]"),
        (faults::Inputs { sel: 4, a: i32::MAX, b: 2, ..base }, Check::Overflow(BinOp::Mul), "i.a * i.b"),
        (faults::Inputs { sel: 5, a: i32::MIN, b: 1, ..base }, Check::Overflow(BinOp::Sub), "i.a - i.b"),
        (faults::Inputs { sel: 6, sh: 9, ..base }, Check::Overflow(BinOp::Shl), "i.c << i.sh"),
        (faults::Inputs { sel: 7, a: i32::MIN, b: -1, ..base }, Check::Overflow(BinOp::Div), "i.a / i.b"),
    ];
    let file = program("faults").display().to_string();
    for (inputs, check, snippet) in cases {
        let e = fault::<_, faults::Outputs>(&code, &inputs);
        let (line, column) = place_of(&program("faults"), snippet);
        assert_eq!(
            (e.kind, code.files[e.at.file as usize].as_str(), e.at.line as usize, e.at.column as usize),
            (ErrorKind::Check(check), file.as_str(), line, column),
            "{snippet}"
        );
    }
}

#[test]
fn statements_and_calls_name_their_own_lines() {
    // each statement and each call of a scan at the line of its own source, not its function's
    let (_, code) = lower_in("lines-faults", "faults", &program("faults")).result.unwrap();
    let lines: Vec<u32> = code.functions[0].blocks.iter().flat_map(|b| b.statements.iter().map(|s| s.at.line)).collect();
    for snippet in ["o.v = i.a + i.b", "o.w = i.c << i.sh", "o.v = self.table[i.idx as usize]"] {
        let (line, _) = place_of(&program("faults"), snippet);
        assert!(lines.contains(&(line as u32)), "{snippet}: line {line} not among the statements' {lines:?}");
    }
    let (_, code) = lower_in("lines-edges", "edges", &program("edges")).result.unwrap();
    let calls: Vec<u32> = code.functions[0]
        .blocks
        .iter()
        .filter_map(|b| match &b.terminator {
            Terminator::Call { at, .. } => Some(at.line),
            _ => None,
        })
        .collect();
    for snippet in ["o.light = code(", "o.picked = pick(", "o.off_ref = read16("] {
        let (line, _) = place_of(&program("edges"), snippet);
        assert!(calls.contains(&(line as u32)), "{snippet}: line {line} not among the calls' {calls:?}");
    }
}

#[test]
fn switches_and_returns_name_their_own_lines() {
    // a `match`'s switch at the line of its scrutinee, a function's return past its first line
    let (_, code) = lower_in("switches-edges", "edges", &program("edges")).result.unwrap();
    let scan = &code.functions[0];
    let switches: Vec<u32> = scan
        .blocks
        .iter()
        .filter_map(|b| match &b.terminator {
            Terminator::SwitchInt { at, .. } => Some(at.line),
            _ => None,
        })
        .collect();
    let (line, _) = place_of(&program("edges"), "o.signed_match = match i.a {");
    assert!(switches.contains(&(line as u32)), "line {line} not among the switches' {switches:?}");
    let (head, _) = place_of(&program("edges"), "fn scan(");
    let mut returns = 0;
    for f in code.functions.iter().filter(|f| f.name.ends_with("::scan")) {
        for b in &f.blocks {
            if let Terminator::Return { at } = &b.terminator {
                assert!(at.line > head as u32, "{}: return at {at:?}, the function at line {head}", f.name);
                returns += 1;
            }
        }
    }
    assert!(returns > 0, "the scan's returns");
}

#[test]
fn the_source_map_names_no_file_of_core() {
    // every location a lowered program holds names a file of the program's crates: one inside core's macros is moved
    // to where the program calls the macro
    for name in ["shapes", "conveyor", "probe", "edges"] {
        let (_, code) = lower_in(&format!("files-{name}"), name, &program(name)).result.unwrap();
        for f in &code.files {
            assert!(!f.contains("library/core"), "{name}: {f} in {:?}", code.files);
        }
    }
}

#[test]
fn a_fault_in_a_program_s_second_file_names_that_file() {
    // the scan in `twofile.rs` calls `helper::ratio` in `helper.rs`, whose division by zero is the fault
    let source = program("twofile");
    let helper = source.with_file_name("helper.rs");
    let (_, code) = lower_in("twofile-fault", "twofile", &source).result.unwrap();
    let e = fault::<_, twofile::Outputs>(&code, &twofile::Inputs { a: 5, b: 0 });
    let (line, column) = place_of(&helper, "a / b");
    assert_eq!(e.kind, ErrorKind::Check(Check::DivisionByZero));
    assert_eq!(
        (code.files[e.at.file as usize].as_str(), e.at.line as usize, e.at.column as usize),
        (helper.display().to_string().as_str(), line, column),
        "files {:?}",
        code.files
    );
    // and the program runs as natively, its divisor never zero
    let inputs = |k: u64| twofile::Inputs { a: (mix(k, 1) % 1_000) as i32 - 500, b: (mix(k, 2) % 9) as i32 + 1 };
    let (outputs, _) = conform("twofile", twofile::Twofile::default(), "Twofile", &inputs, 200);
    both(&outputs, "a negative ratio", |o| o.v < 0);
}

#[test]
fn an_operation_over_two_lines_faults_at_its_first_line() {
    let source = program("lines");
    let (_, code) = lower("lines", &source).result.unwrap();
    let e = fault::<_, lines::Outputs>(&code, &lines::Inputs { a: 5, b: 0 });
    let (line, column) = place_of(&source, "i.a\n");
    assert_eq!(e.kind, ErrorKind::Check(Check::DivisionByZero));
    assert_eq!((e.at.line as usize, e.at.column as usize), (line, column));
}

#[test]
fn orcl_std_s_own_macro_keeps_its_lines() {
    // `impl_bits!` is orcl-std's: the statements of `get_bit` it expands to stand on the macro's lines, not where
    // the macro is called
    let (_, code) = lower_in("bits-lines", "probe", &program("probe")).result.unwrap();
    let bits = here().join("../orcl-std/src/bits.rs");
    let file = code.files.iter().position(|f| f.ends_with("orcl-std/src/bits.rs")).expect("bits.rs in the source map");
    let file = file as u32;
    let (inside, _) = place_of(&bits, "match self.checked_shr(n)");
    let (call, _) = place_of(&bits, "impl_bits!(u8");
    let lines: Vec<u32> = code
        .functions
        .iter()
        .filter(|f| f.name.contains("get_bit"))
        .flat_map(|f| f.blocks.iter().flat_map(|b| b.statements.iter().map(|s| s.at)))
        .filter(|at| at.file == file)
        .map(|at| at.line)
        .collect();
    assert!(!lines.is_empty(), "get_bit's statements");
    let (inside, call) = (inside as u32, call as u32);
    assert!(lines.contains(&inside) && !lines.contains(&call), "{lines:?}: inside {inside}, call {call}");
}

/// The program `name` of `programs/refused/`.
fn refused_program(name: &str) -> PathBuf {
    here().join("tests/programs/refused").join(format!("{name}.rs"))
}

/// Lowers `name` from `programs/refused/`, and asserts that the driver refused it with exactly `message` and
/// wrote no file.
fn refused(name: &str, message: &str) {
    let lowered = lower(name, &refused_program(name));
    let stderr = lowered.result.err().unwrap_or_else(|| panic!("{name} was lowered"));
    let line = format!("error: orcl-front: {message}");
    assert!(stderr.lines().any(|l| l == line), "{name}: wanted\n{line}\ngot\n{stderr}");
    assert!(!lowered.file.exists(), "{name}: no file written");
}

/// A refusal's message, at the place `snippet` stands in the refused program.
fn at(name: &str, snippet: &str, words: &str, what: &str) -> String {
    let source = refused_program(name);
    let (line, column) = place_of(&source, snippet);
    format!("{}:{line}:{column}: {words}: `{what}`", source.display())
}

const TYPE: &str = "a type outside ORCL's";

#[test]
fn a_raw_pointer_is_refused_where_the_program_holds_it() {
    refused("raw_pointer", &at("raw_pointer", "place: *const", TYPE, "*const u8"));
}

#[test]
fn a_string_is_refused_where_the_program_holds_it() {
    refused("string", &at("string", "name = String", TYPE, "std::string::String"));
}

#[test]
fn a_slice_is_refused_where_the_program_holds_it() {
    refused("slice", &at("slice", "view: &[u8]", TYPE, "[u8]"));
}

#[test]
fn a_closure_is_refused_where_the_program_holds_it() {
    let source = refused_program("closure");
    // a closure's type is named by its head, `|v: u8|`
    let (line, column) = place_of(&source, "|v: u8|");
    let closure = format!("{{closure@{}:{line}:{column}: {line}:{}}}", source.display(), column + "|v: u8|".len());
    refused("closure", &at("closure", "next = |", TYPE, &closure));
}

#[test]
fn a_trait_object_is_refused_where_the_program_holds_it() {
    refused("trait_object", &at("trait_object", "shown: &dyn", TYPE, "dyn std::fmt::Debug"));
}

#[test]
fn a_usize_the_program_declares_is_refused_where_it_declares_it() {
    refused("usize", &at("usize", "count: usize", TYPE, "usize"));
}

#[test]
fn an_isize_field_is_refused_where_the_program_declares_it() {
    refused("isize_field", &at("isize_field", "offset: isize", TYPE, "isize"));
}

#[test]
fn a_128_bit_integer_is_refused_where_the_program_declares_it() {
    refused("i128", &at("i128", "total: i128", TYPE, "i128"));
}

#[test]
fn a_u128_is_refused_where_the_program_declares_it() {
    refused("u128", &at("u128", "total: u128", TYPE, "u128"));
}

#[test]
fn a_usize_variable_is_refused_where_the_program_declares_it() {
    refused("usize_variable", &at("usize_variable", "at: usize", TYPE, "usize"));
}

#[test]
fn a_usize_parameter_is_refused_where_the_program_declares_it() {
    refused("usize_param", &at("usize_param", "at: usize", TYPE, "usize"));
}

#[test]
fn an_unnamed_usize_parameter_is_refused() {
    refused("unnamed", &at("unnamed", "_: usize", TYPE, "usize"));
}

#[test]
fn a_usize_result_is_refused_where_the_program_declares_it() {
    refused("usize_result", &at("usize_result", "usize {", TYPE, "usize"));
}

#[test]
fn a_usize_array_element_is_refused_where_the_program_declares_it() {
    refused("usize_array", &at("usize_array", "counts: [usize; 2]", TYPE, "usize"));
}

#[test]
fn a_usize_array_parameter_is_refused_at_the_parameter_not_at_the_caller_s_temporary() {
    refused("usize_array_param", &at("usize_array_param", "at: [usize; 2]", TYPE, "usize"));
}

#[test]
fn a_usize_in_a_tuple_field_is_refused_where_the_program_declares_it() {
    refused("usize_tuple", &at("usize_tuple", "pair: (usize, u8)", TYPE, "usize"));
}

#[test]
fn a_usize_behind_a_reference_parameter_is_refused_where_the_program_declares_it() {
    refused("usize_ref", &at("usize_ref", "at: &usize", TYPE, "usize"));
}

#[test]
fn a_usize_inside_an_option_field_is_refused_where_the_program_declares_it_not_inside_core() {
    refused("usize_option", &at("usize_option", "last: Option<usize>", TYPE, "usize"));
}

#[test]
fn a_usize_in_the_program_s_own_option_is_refused_at_its_payload() {
    refused("own_option", &at("own_option", "usize),", TYPE, "usize"));
}

#[test]
fn an_isize_deep_in_an_enum_s_payload_is_refused_where_the_program_declares_the_payload() {
    refused("isize_payload", &at("isize_payload", "Option<(u8, isize)>", TYPE, "isize"));
}

#[test]
fn a_usize_behind_an_array_an_option_a_tuple_and_a_reference_is_refused_where_the_variable_is_declared() {
    refused("usize_nested", &at("usize_nested", "held: [Option", TYPE, "usize"));
}

#[test]
fn a_type_with_drop_glue_is_refused_where_the_program_holds_it() {
    refused("drop_glue", &at("drop_glue", "guard = Guard", "a type with drop glue", "Guard"));
}

#[test]
fn a_call_into_core_outside_the_list_is_refused_by_name_where_it_is_called() {
    refused("core_call", &at("core_call", "(*i).clamp(2, 9)", "a call into core outside the list", "<u8 as Ord>::clamp"));
}

#[test]
fn ne_on_a_program_s_own_enum_is_refused_by_its_full_name() {
    // the trait's own arguments kept in the name
    let words = "a call into core outside the list";
    refused("ne", &at("ne", "(self.state != next)", words, "<State as PartialEq<State>>::ne"));
}

#[test]
fn a_statement_the_interpreter_does_not_have_is_refused_where_it_stands() {
    refused(
        "statement",
        &at("statement", "core::intrinsics::assume", "a statement the interpreter does not have", "Intrinsic(Assume)"),
    );
}

#[test]
fn a_value_the_interpreter_does_not_have_is_refused_where_it_stands() {
    refused(
        "value",
        &at("value", "core::mem::transmute", "a value the interpreter does not have", "Cast(Transmute)"),
    );
}

#[test]
fn an_array_repeated_past_what_the_front_end_builds_is_refused_where_it_stands() {
    let words = "a value the interpreter does not have";
    refused("repeat", &at("repeat", "[self.x; 70_000]", words, "an array of one value repeated past 65536"));
}

#[test]
fn a_terminator_the_interpreter_does_not_have_is_refused_where_it_stands() {
    refused("terminator", &at("terminator", "core::arch::asm!", "a terminator the interpreter does not have", "InlineAsm"));
}

#[test]
fn a_call_that_does_not_return_is_refused_where_it_is_called() {
    let words = "a terminator the interpreter does not have";
    refused("noret", &at("noret", "halt();", words, "a call that does not return"));
}

#[test]
fn a_call_out_of_the_program_s_crates_and_core_is_refused_by_name_where_it_is_called() {
    refused("call", &at("call", "std::thread::yield_now()", "a call the interpreter cannot make", "std::thread::yield_now"));
}

#[test]
fn a_crate_with_no_program_is_refused() {
    refused("no_program", "no program: `0 implementations of orcl_std::Program`");
}

#[test]
fn a_crate_with_two_programs_is_refused() {
    refused("two_programs", "no program: `2 implementations of orcl_std::Program`");
}

#[test]
fn what_core_s_macro_holds_is_refused_where_the_program_calls_the_macro() {
    refused("panic_macro", &at("panic_macro", "unreachable!(", TYPE, "std::fmt::Arguments<'_>"));
}

#[test]
fn a_derived_default_s_refusal_names_the_field() {
    let words = "a call into core outside the list";
    refused("derive_array", &at("derive_array", "table: [u8; 4]", words, "<[u8; 4] as Default>::default"));
}

#[test]
fn a_program_the_loader_refuses_writes_no_file() {
    refused("huge", "the lowered program does not load: region 0: memory past the host's limit");
}

#[test]
fn a_crate_rustc_refuses_after_its_analysis_writes_no_file() {
    // the front end has lowered it by then; rustc's own error stops the build, and the file is never written
    let lowered = lower("post_mono", &refused_program("post_mono"));
    let stderr = lowered.result.err().unwrap_or_else(|| panic!("post_mono was lowered"));
    assert!(stderr.contains("error[E0080]"), "{stderr}");
    assert!(!lowered.file.exists(), "no file written");
}

// Round 5's probes (c156-r5).

#[test]
fn usize_temps_temporaries_through_the_list_run_lowered_as_natively() {
    let inputs = |k: u64| usize_temps::Inputs { a: mix(k, 1) as u8, b: mix(k, 2) as i8 };
    let (outputs, _) = conform("usize_temps", usize_temps::UsizeTemps::default(), "UsizeTemps", &inputs, 400);
    both(&outputs, "a low value past 200", |o| o.low > 0);
    both(&outputs, "back wrapped", |o| o.back > 100);
}

#[test]
fn a_program_s_own_macro_faults_inside_the_macro() {
    let source = program("own_macro");
    let (_, code) = lower("own_macro", &source).result.unwrap_or_else(|e| panic!("{e}"));
    let e = fault::<_, own_macro::Outputs>(&code, &own_macro::Inputs { a: 5, b: 0 });
    let (line, column) = place_of(&source, "top / bottom");
    assert_eq!(e.kind, ErrorKind::Check(Check::DivisionByZero));
    assert_eq!((e.at.line as usize, e.at.column as usize), (line, column), "inside the program's own macro");
    let inputs = |k: u64| own_macro::Inputs { a: (mix(k, 1) % 1_000) as i32 - 500, b: (mix(k, 2) % 9) as i32 + 1 };
    let (outputs, _) = conform("own_macro", own_macro::OwnMacro, "OwnMacro", &inputs, 200);
    both(&outputs, "negative", |o| o.v < 0);
}

#[test]
fn two_const_arguments_run_lowered_as_natively() {
    let inputs = |k: u64| grid::Inputs { v: (mix(k, 1) % 1_000) as u16 };
    let (outputs, _) = conform("grid", grid::Grids::default(), "Grids", &inputs, 400);
    both(&outputs, "the two grids apart", |o| o.small != o.large);
}

const OUTSIDE_THE_LIST: &str = "a call into core outside the list";

#[test]
fn an_inherent_integer_operation_outside_the_list_is_refused_by_name() {
    refused("abs", &at("abs", "(*i).wrapping_neg()", OUTSIDE_THE_LIST, "core::num::<impl i8>::wrapping_neg"));
}

#[test]
fn an_inherent_option_operation_outside_the_list_is_refused_by_name() {
    refused("unwrap_or", &at("unwrap_or", "self.last.unwrap_or(7)", OUTSIDE_THE_LIST, "std::option::Option::<u8>::unwrap_or"));
}

#[test]
fn an_inherent_duration_operation_outside_the_list_is_refused_by_name() {
    refused("checked_sub", &at("checked_sub", "self.left.checked_sub", OUTSIDE_THE_LIST, "orcl_std::Duration::checked_sub"));
}

#[test]
fn a_scalar_s_clone_is_refused_at_the_derive_s_field() {
    refused("clone", &at("clone", "v: u8", OUTSIDE_THE_LIST, "<u8 as Clone>::clone"));
}

#[test]
fn a_generic_comparison_at_u8_is_refused_by_name() {
    refused("generic_cmp", &at("generic_cmp", "a > b", OUTSIDE_THE_LIST, "<u8 as PartialOrd<u8>>::gt"));
}

#[test]
fn a_union_is_refused_where_the_program_declares_it() {
    refused("union_type", &at("union_type", "word: Word", TYPE, "Word"));
}

#[test]
fn a_usize_behind_a_mutable_reference_is_refused_at_the_parameter() {
    refused("mut_usize", &at("mut_usize", "at: &mut usize", TYPE, "usize"));
}

#[test]
fn a_file_that_cannot_be_written_fails_the_driver() {
    let work = fresh("unwritable");
    compile_std(&work, &[]);
    let w = work.display().to_string();
    let file = work.join("no-such-folder").join("lines.orclb");
    let source = program("lines").display().to_string();
    let mut args = strings(&["--crate-name", "lines", "--crate-type", "rlib", "--edition", "2024", &source]);
    args.extend(strings(&["--extern", &format!("orcl_std={w}/liborcl_std.rlib"), "-L", &w, "--out-dir", &w]));
    args.extend(strings(&["--orcl-program", "lines", &format!("--orcl-out={}", file.display()), "--orcl-revision", "x"]));
    let out = driver(&args);
    assert!(!out.status.success(), "a file not written reported as written");
    assert!(String::from_utf8_lossy(&out.stderr).contains(&format!("error: orcl-front: {}", file.display())));
}

#[test]
fn locals_keep_the_source_s_names() {
    // the scan's parameters and a variable the program names, by the source's names (interpreter.md, "The functions")
    let (_, code) = lower_in("locals-names", "faults", &program("faults")).result.unwrap();
    let names: Vec<Option<&str>> = code.functions[0].locals.iter().map(|l| l.name.as_deref()).collect();
    assert_eq!(names.get(1..=4), Some(&[Some("self"), Some("_scan"), Some("i"), Some("o")][..]), "{names:?}");
}
