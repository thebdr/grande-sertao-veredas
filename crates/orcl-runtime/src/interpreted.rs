//! An interpreted program on the engine of a scan: a machine of `orcl-interp` and its scan entry, run by a task as a
//! native program is (`docs/orcl/interpreter.md`, "Running").
use crate::task::TaskProgram;
use core::marker::PhantomData;
use core::time::Duration;
use orcl_interp::{Arg, CallError, FunctionId, HostError, Int, LoadError, Machine, Program, ProgramError, RegionId, Scalar, Type, TypeId};
use orcl_std::Scan;
use std::vec::Vec;

/// A value of a program's inputs or outputs as the interpreter's slots, so that a native program's own types are
/// those of its interpreted twin.
pub trait Image: Copy + Default {
    /// Pushes the value's slots onto `slots`, in the order the program's type lays them out.
    fn write(&self, slots: &mut Vec<Scalar>);

    /// The value the slots hold, or `None` when they hold none.
    fn read(slots: &[Scalar]) -> Option<Self>;
}

/// The program's state.
pub const STATE: RegionId = RegionId(0);
/// The scan it is handed: `orcl_std::Scan` as the front end lays it out, its time, its period and its cycle.
pub const SCAN: RegionId = RegionId(1);
/// The inputs the scan reads.
pub const INPUTS: RegionId = RegionId(2);
/// The outputs the scan writes, holding the image the task hands it.
pub const OUTPUTS: RegionId = RegionId(3);

/// The four regions' types, as the program's scan entry takes them:
/// `fn scan(&mut self, scan: &Scan, inputs: &I, outputs: &mut O)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    pub state: TypeId,
    pub scan: TypeId,
    pub inputs: TypeId,
    pub outputs: TypeId,
}

/// Why an interpreted program cannot be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MakeError {
    /// The machine refused the program.
    Load(LoadError),
    /// The first state's slots are not a value of the state's type.
    State,
    /// The scan's type is not a scan's: a structure of its time and its period, each a `Duration`, and its cycle, a
    /// `u64`, in that order.
    Scan,
    /// The inputs' default is not a value of the inputs' type.
    Inputs,
    /// The outputs' default, or the safe outputs, are not a value of the outputs' type, or read back as none.
    Outputs,
    /// The entry is no function of the program, or does not take references to the four regions.
    Entry(HostError),
}

/// Why a scan of an interpreted program failed: the program's own error, or, in an adapter made as it checks, none
/// of the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanError {
    /// The program failed where Rust would panic; the machine stands at that instruction.
    Program(ProgramError),
    /// The machine refused what the adapter handed it.
    Host(HostError),
    /// The outputs region holds no value of the outputs' type.
    Outputs,
}

/// An interpreted program a task runs: its machine, its scan entry, and its safe outputs.
pub struct Interpreted<I: Image, O: Image> {
    machine: Machine,
    entry: FunctionId,
    safe: O,
    /// Room for the slots handed to a region, reserved when the adapter is made for the largest region, so a scan
    /// allocates nothing while each image writes no more slots than its region holds.
    slots: Vec<Scalar>,
    inputs: PhantomData<fn(&I)>,
}

/// The entry's arguments: a reference to each region, in its parameters' order.
const ARGS: [Arg<'static>; 4] = [Arg::Ref(STATE), Arg::Ref(SCAN), Arg::Ref(INPUTS), Arg::Ref(OUTPUTS)];

fn duration(slots: &mut Vec<Scalar>, d: Duration) {
    slots.push(Scalar::Int(Int::U64, i128::from(d.as_secs())));
    slots.push(Scalar::Int(Int::U32, i128::from(d.subsec_nanos())));
}

/// Whether `ty` is a scan's type: a structure of a time and a period, each a `Duration`, and a cycle, a `u64`. Its
/// shape is read, not one value written, since a type can hold a scan of zeros and refuse every later one.
fn is_a_scan(program: &Program, ty: TypeId) -> bool {
    let of = |ty: TypeId| program.types.get(ty.0 as usize);
    let Some(Type::Struct { fields, .. }) = of(ty) else { return false };
    match fields.as_slice() {
        [now, period, cycle] => {
            matches!(of(now.ty), Some(Type::Duration))
                && matches!(of(period.ty), Some(Type::Duration))
                && matches!(of(cycle.ty), Some(Type::Int(Int::U64)))
        }
        _ => false,
    }
}

fn scan_slots(slots: &mut Vec<Scalar>, scan: &Scan) {
    slots.clear();
    duration(slots, scan.now);
    duration(slots, scan.period);
    slots.push(Scalar::Int(Int::U64, i128::from(scan.cycle)));
}

impl<I: Image, O: Image> Interpreted<I, O> {
    /// Loads `program` with its four regions and checks that `entry` takes them: `state` is its first state and
    /// `safe` its safe outputs, both handed in by the host (`docs/questions.md` Q231, DECISION PENDING: the front end
    /// would compute them). The inputs' and the outputs' defaults, the safe outputs and a scan must each be a value of
    /// its region's type.
    pub fn new(program: Program, entry: FunctionId, regions: Regions, state: &[Scalar], safe: O) -> Result<Self, MakeError> {
        let types = [regions.state, regions.scan, regions.inputs, regions.outputs];
        let mut machine = Machine::load(program, &types).map_err(MakeError::Load)?;
        machine.set_region(STATE, state).map_err(|_| MakeError::State)?;
        if !is_a_scan(machine.program(), regions.scan) {
            return Err(MakeError::Scan);
        }
        let sizes = types.map(|ty| machine.size_of(ty).unwrap_or(0) as usize);
        let room = sizes.iter().copied().max().unwrap_or(0);
        let mut slots = Vec::with_capacity(room);
        scan_slots(&mut slots, &Scan { now: Duration::ZERO, period: Duration::ZERO, cycle: 0 });
        machine.set_region(SCAN, &slots).map_err(|_| MakeError::Scan)?;
        slots.clear();
        I::default().write(&mut slots);
        machine.set_region(INPUTS, &slots).map_err(|_| MakeError::Inputs)?;
        for value in [safe, O::default()] {
            slots.clear();
            value.write(&mut slots);
            machine.set_region(OUTPUTS, &slots).map_err(|_| MakeError::Outputs)?;
            machine.region(OUTPUTS).and_then(O::read).ok_or(MakeError::Outputs)?;
        }
        machine.start(entry, &ARGS).map_err(MakeError::Entry)?;
        machine.abort();
        Ok(Self { machine, entry, safe, slots, inputs: PhantomData })
    }

    /// The machine, as the last scan left it: where a failed scan stopped, its instruction count.
    pub fn machine(&self) -> &Machine {
        &self.machine
    }
}

impl<I: Image, O: Image> TaskProgram for Interpreted<I, O> {
    type Inputs = I;
    type Outputs = O;
    type Error = ScanError;

    /// Writes the scan, the inputs and the output image into their regions, calls the entry to its return and
    /// reads the outputs back. The output image is written too, so the region holds exactly what the task hands the
    /// program, as a native program's `&mut` does, whatever the region held before.
    fn scan(&mut self, scan: &Scan, inputs: &I, outputs: &mut O) -> Result<(), ScanError> {
        scan_slots(&mut self.slots, scan);
        self.machine.set_region(SCAN, &self.slots).map_err(ScanError::Host)?;
        self.slots.clear();
        inputs.write(&mut self.slots);
        self.machine.set_region(INPUTS, &self.slots).map_err(ScanError::Host)?;
        self.slots.clear();
        outputs.write(&mut self.slots);
        self.machine.set_region(OUTPUTS, &self.slots).map_err(ScanError::Host)?;
        self.machine.call(self.entry, &ARGS).map_err(|e| match e {
            CallError::Program(error) => ScanError::Program(error),
            CallError::Host(error) => ScanError::Host(error),
        })?;
        *outputs = self.machine.region(OUTPUTS).and_then(O::read).ok_or(ScanError::Outputs)?;
        Ok(())
    }

    /// The safe outputs handed in when the adapter was made; nothing of the program runs once it has faulted.
    fn safe_outputs(&mut self) -> O {
        self.safe
    }
}
