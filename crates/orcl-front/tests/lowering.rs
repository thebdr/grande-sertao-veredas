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
#[path = "programs/probe.rs"]
mod probe;
#[allow(dead_code)]
#[path = "programs/shapes.rs"]
mod shapes;

use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::Duration;

use orcl_interp::file::{self, Header};
use orcl_interp::{
    Arg, Check, ErrorKind, FunctionId, Int, Limits, Machine, Program as Code, ProgramError, RegionId, Scalar, Type,
    TypeId,
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
    let lib = match std::env::var("LD_LIBRARY_PATH") {
        Ok(p) if !p.is_empty() => format!("{}:{p}", sysroot_lib()),
        _ => sysroot_lib().to_string(),
    };
    Command::new(env!("CARGO_BIN_EXE_orcl-front-driver")).env("LD_LIBRARY_PATH", lib).args(args).output().unwrap()
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

/// Lowers the crate `source` as the program `name`, in the folder `folder`: one test's, since tests run at once.
fn lower_in(folder: &str, name: &str, source: &Path) -> Lowered {
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lowering").join(folder);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let w = work.display().to_string();
    let std_src = here().join("../orcl-std/src/lib.rs").display().to_string();
    let out = driver(&strings(&["--crate-name", "orcl_std", "--crate-type", "rlib", "--edition", "2024", &std_src, "--out-dir", &w]));
    assert!(out.status.success(), "orcl-std: {}", String::from_utf8_lossy(&out.stderr));
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
fn conform<P, I, O>(name: &str, native: P, state_name: &str, inputs: &dyn Fn(u64) -> I, scans: u64)
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
    conform("conveyor", conveyor::Conveyor::default(), "Conveyor", &inputs, 6_000);
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
    conform("probe", probe::Probe::default(), "Probe", &inputs, 6_000);
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
    conform("arith", arith::Arith::default(), "Arith", &arith_inputs, 4_000);
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
    conform("shapes", shapes::Shapes::default(), "Shapes", &inputs, 4_000);
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

#[test]
fn a_scan_s_program_error_names_its_place_in_the_source_map() {
    // a divisor of zero: the native program would panic; the lowered one fails at the division, by file, line and
    // column, the machine standing at the assertion
    let (_, code) = lower_in("arith-fault", "arith", &program("arith")).result.unwrap();
    let regions = regions(&code);
    let state = first_state(&code);
    let mut machine = load(&code, &[regions.state, regions.scan, regions.inputs, regions.outputs]);
    let mut slots = Vec::new();
    machine.set_region(RegionId(0), &state).unwrap();
    slots.extend([Scalar::Int(Int::U64, 0), Scalar::Int(Int::U32, 0), Scalar::Int(Int::U64, 0), Scalar::Int(Int::U32, 0)]);
    slots.push(Scalar::Int(Int::U64, 0));
    machine.set_region(RegionId(1), &slots).unwrap();
    let mut zero = arith_inputs(5);
    zero.b = 0;
    slots.clear();
    zero.write(&mut slots);
    machine.set_region(RegionId(2), &slots).unwrap();
    slots.clear();
    arith::Outputs::default().write(&mut slots);
    machine.set_region(RegionId(3), &slots).unwrap();
    let args = [0, 1, 2, 3].map(|r| Arg::Ref(RegionId(r)));
    let error: ProgramError = match machine.call(FunctionId(0), &args) {
        Err(orcl_interp::CallError::Program(e)) => e,
        other => panic!("a division by zero ran: {other:?}"),
    };
    assert_eq!(error.kind, ErrorKind::Check(Check::DivisionByZero));
    let (line, column) = place_of(&program("arith"), "i.a / i.b");
    assert_eq!(code.files[error.at.file as usize], program("arith").display().to_string());
    assert_eq!((error.at.line as usize, error.at.column as usize), (line, column), "the division's place");
}

/// Lowers `name` from `programs/refused/`, and asserts that the driver refused it with exactly `message` and
/// wrote no file.
fn refused(name: &str, message: &str) {
    let source = here().join("tests/programs/refused").join(format!("{name}.rs"));
    let lowered = lower(name, &source);
    let stderr = lowered.result.err().unwrap_or_else(|| panic!("{name} was lowered"));
    let line = format!("error: orcl-front: {message}");
    assert!(stderr.lines().any(|l| l == line), "{name}: wanted\n{line}\ngot\n{stderr}");
    assert!(!lowered.file.exists(), "{name}: no file written");
}

/// A refusal's message, at the place `snippet` stands in the refused program.
fn at(name: &str, snippet: &str, words: &str, what: &str) -> String {
    let source = here().join("tests/programs/refused").join(format!("{name}.rs"));
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
    let source = here().join("tests/programs/refused/closure.rs");
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
