//! The front end proven against Rust (C-156). Each conformance program under `programs/` is compiled natively into
//! this test and by the driver as a crate of its own (`orcl-std` first, as the driver compiles it, then the program
//! with `--extern`), into a folder of its own under `CARGO_TARGET_TMPDIR`. The lowered program is read back from its
//! file, its first state and its safe outputs computed by calling its `Default` and its `safe_outputs` on a machine
//! (as a host does until the file carries them, `docs/questions.md` Q231), and run on the engine of a scan beside
//! its native build, each on its own virtual clock: every scan's report and every write compared, over thousands of
//! scans with varied inputs, and its instruction count recorded. Each refusal is met by a program that triggers it,
//! its name and its location asserted exactly, and no file written.
//!
//! Run by the pinned nightly alone: `cargo +nightly-2026-11-21 test -p orcl-front --features driver --test lowering`.

mod cap;

#[global_allocator]
static CAP: cap::Capped = cap::Capped;

#[allow(dead_code)]
#[path = "programs/arith.rs"]
mod arith;
#[allow(dead_code)]
#[path = "programs/conveyor.rs"]
mod conveyor;
#[allow(dead_code)]
#[path = "programs/edges.rs"]
mod edges;
#[allow(dead_code)]
#[path = "programs/faults.rs"]
mod faults;
#[allow(dead_code)]
#[path = "programs/probe.rs"]
mod probe;
#[allow(dead_code)]
#[path = "programs/shapes.rs"]
mod shapes;
#[allow(dead_code)]
#[path = "programs/sound.rs"]
mod sound;
#[allow(dead_code)]
#[path = "programs/zst_const.rs"]
mod zst_const;

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

fn driver_with(args: &[String], env: &[(&str, &str)]) -> Output {
    let lib = match std::env::var("LD_LIBRARY_PATH") {
        Ok(p) if !p.is_empty() => format!("{}:{p}", sysroot_lib()),
        _ => sysroot_lib().to_string(),
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_orcl-front-driver"));
    command.env("LD_LIBRARY_PATH", lib).args(args);
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

fn program(name: &str) -> PathBuf {
    here().join("tests/programs").join(format!("{name}.rs"))
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

/// Runs `name` natively and lowered on the engine for `scans` scans of `inputs`, every report and write alike.
/// The native program's outputs, scan by scan, are returned for the test to say the run reached what it is to prove.
fn conform<P, I, O>(name: &str, native: P, state_name: &str, inputs: &dyn Fn(u64) -> I, scans: u64) -> Vec<O>
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
    assert!(instructions > 0);
    let record = format!("{name}: {scans} scans alike, {instructions} instructions in all, {most} at most in one scan\n");
    print!("{record}");
    fs::write(lowered.file.with_extension("instructions"), record).unwrap();
    native_feed.writes.into_iter().map(|(_, o)| o).collect()
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
    let outputs = conform("conveyor", conveyor::Conveyor::default(), "Conveyor", &inputs, 6_000);
    both(&outputs, "the motor", |o| o.motor_on);
    both(&outputs, "the door's alarm", |o| o.door_alarm);
    both(&outputs, "the running lamp", |o| o.lamp_running);
    both(&outputs, "the fault lamp", |o| o.lamp_fault);
    assert!(outputs.iter().map(|o| o.parts).max() >= Some(10), "parts counted");
}

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
    let outputs = conform("probe", probe::Probe::default(), "Probe", &inputs, 6_000);
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
    let outputs = conform("arith", arith::Arith::default(), "Arith", &arith_inputs, 4_000);
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
    let outputs = conform("shapes", shapes::Shapes::default(), "Shapes", &inputs, 4_000);
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
    let outputs = conform("edges", edges::Edges::default(), "Edges", &inputs, 2_000);
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
    let outputs = conform("sound", sound::Sound::default(), "Sound", &inputs, 3_000);
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
    let outputs = conform("zst_const", zst_const::ZstConst::default(), "ZstConst", &inputs, 200);
    both(&outputs, "an odd sum", |o| o.v % 2 == 1);
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
fn a_usize_variable_is_refused_where_the_program_declares_it() {
    refused("usize_variable", &at("usize_variable", "at: usize", TYPE, "usize"));
}

#[test]
fn a_usize_parameter_is_refused_where_the_program_declares_it() {
    refused("usize_param", &at("usize_param", "at: usize", TYPE, "usize"));
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
