//! `orcl-std`'s blocks lowered by hand in the shape rustc gives them at opt-level 0, run in the machine beside the
//! native blocks over long traces: every scan's output, `q` and elapsed time the same. They write `copy` where rustc
//! writes `move`; moved operands are run on their own in `machine.rs`. The on-delay reaches four of the
//! list's operations on its real code path (`Option::get_or_insert`, `Duration::saturating_sub`, `Ord::min`,
//! `PartialOrd::ge`); the rising edge, `&&` as the switch rustc makes of it.
mod common;
use common::*;
use core::time::Duration;
use orcl_interp::*;

fn duration(d: Duration) -> [Scalar; 2] {
    [int(Int::U64, d.as_secs() as i128), int(Int::U32, d.subsec_nanos() as i128)]
}

struct Ton {
    machine: Machine,
    scan: FunctionId,
}

/// `Ton::scan` and the types it touches, as MIR has them.
fn ton() -> Ton {
    let mut b = Build::default();
    let (boolean, u64t, dur) = (b.bool(), b.int(Int::U64), b.ty(Type::Duration));
    let opt = b.option(dur);
    let ton = b.ty(Type::Struct {
        name: "Ton".into(),
        fields: vec![
            Field { name: "q".into(), ty: boolean },
            Field { name: "et".into(), ty: dur },
            Field { name: "since".into(), ty: opt },
        ],
    });
    let scan = b.ty(Type::Struct {
        name: "Scan".into(),
        fields: vec![
            Field { name: "now".into(), ty: dur },
            Field { name: "period".into(), ty: dur },
            Field { name: "cycle".into(), ty: u64t },
        ],
    });
    let (rton, rscan, ropt, rdur, sdur) =
        (b.reference(ton, true), b.reference(scan, false), b.reference(opt, true), b.reference(dur, true), b.reference(dur, false));
    let selff = |f: u32| proj(1, vec![Projection::Deref, Projection::Field(f)]);
    let now = || copy_at(proj(2, vec![Projection::Deref, Projection::Field(0)]));
    let core = |op, args, dest, next| Terminator::Call { callee: Callee::Core(op), args, dest, next: BlockId(next), at: at(20) };
    let f = b.function(Function {
        name: "Ton::scan".into(),
        params: 4,
        locals: locals(&[boolean, rton, rscan, boolean, dur, rdur, dur, dur, dur, ropt, sdur, sdur]),
        blocks: vec![
            // bb0: if input
            block(vec![], Terminator::SwitchInt { value: copy(3), targets: vec![(0, BlockId(3))], otherwise: BlockId(1), at: at(1) }),
            // bb1: let since = *self.since.get_or_insert(scan.now)
            block(
                vec![assign(place(9), Value::Ref(selff(2), true), 2), assign(place(8), Value::Use(now()), 2)],
                core(CoreOp::OptionGetOrInsert, vec![copy(9), copy(8)], place(5), 2),
            ),
            // bb2: let elapsed = scan.now.saturating_sub(since)
            block(
                vec![assign(place(6), Value::Use(copy_at(proj(5, vec![Projection::Deref]))), 2), assign(place(8), Value::Use(now()), 3)],
                core(CoreOp::DurationSaturatingSub, vec![copy(8), copy(6)], place(7), 4),
            ),
            // bb3: else { self.since = None; self.et = Duration::ZERO; self.q = false }
            block(
                vec![
                    assign(selff(2), Value::Aggregate(opt, Some(0), vec![]), 7),
                    assign(selff(1), Value::Use(konst(dur, duration(Duration::ZERO).to_vec())), 8),
                    assign(selff(0), Value::Use(konst(boolean, vec![Scalar::Bool(false)])), 9),
                ],
                Terminator::Goto(BlockId(6)),
            ),
            // bb4: self.et = elapsed.min(pt)
            block(vec![], core(CoreOp::DurationMin, vec![copy(7), copy(4)], selff(1), 5)),
            // bb5: self.q = elapsed >= pt
            block(
                vec![assign(place(10), Value::Ref(place(7), false), 5), assign(place(11), Value::Ref(place(4), false), 5)],
                core(CoreOp::DurationCompare(BinOp::Ge), vec![copy(10), copy(11)], selff(0), 6),
            ),
            // bb6: self.q
            block(vec![assign(place(0), Value::Use(copy_at(selff(0))), 11)], ret()),
        ],
    });
    let machine = Machine::load(b.program, &[ton, scan]).expect("the on-delay loads");
    Ton { machine, scan: f }
}

#[test]
fn the_on_delay_lowered_by_hand_agrees_with_orcl_std_s_over_long_traces() {
    let mut rng = Rng(0x7011_0000_0000_0001);
    let presets = [0u64, 1, 10, 100, 1000, 1500, 7, 250_000];
    let mut scans = 0;
    for trace in 0..60 {
        let mut t = ton();
        let (ton_region, scan_region) = (RegionId(0), RegionId(1));
        // Ton::default(): off, nothing elapsed, never started
        let fresh = [Scalar::Bool(false), int(Int::U64, 0), int(Int::U32, 0), int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit];
        t.machine.set_region(ton_region, &fresh).unwrap();
        let mut native = orcl_std::Ton::default();
        let mut now = Duration::from_millis(rng.below(1000));
        let mut pt = Duration::from_millis(presets[rng.below(8) as usize]);
        let mut input = false;
        for cycle in 0..400u64 {
            match rng.below(20) {
                0 => now = now.saturating_sub(Duration::from_nanos(rng.below(300_000_000))),
                1 => pt = Duration::from_millis(presets[rng.below(8) as usize]),
                2 => pt = Duration::from_nanos(rng.below(2_000_000_000)),
                _ => now += Duration::from_nanos(rng.below(40_000_000)),
            }
            if rng.below(6) == 0 {
                input = !input;
            }
            let period = Duration::from_millis(10);
            let s = orcl_std::Scan { now, period, cycle };
            let mut slots = duration(now).to_vec();
            slots.extend(duration(period));
            slots.push(int(Int::U64, cycle as i128));
            t.machine.set_region(scan_region, &slots).unwrap();
            let input_slot = [Scalar::Bool(input)];
            let pt_slots = duration(pt);
            let got = t
                .machine
                .call(t.scan, &[Arg::Ref(ton_region), Arg::Ref(scan_region), Arg::Value(&input_slot), Arg::Value(&pt_slots)])
                .unwrap()
                .to_vec();
            let want = native.scan(&s, input, pt);
            let state = t.machine.region(ton_region).unwrap();
            let context = format!("trace {trace}, scan {cycle}: now {now:?}, pt {pt:?}, input {input}");
            assert_eq!(got, vec![Scalar::Bool(want)], "{context}");
            assert_eq!(state[0], Scalar::Bool(native.q()), "q(), {context}");
            assert_eq!(state[1..3], duration(native.et()), "et(), {context}");
            scans += 1;
        }
    }
    assert_eq!(scans, 24_000);
}

#[test]
fn the_rising_edge_lowered_by_hand_agrees_with_orcl_std_s_over_every_trace_of_ten_scans() {
    let mut b = Build::default();
    let boolean = b.bool();
    let rtrig = b.ty(Type::Struct {
        name: "RTrig".into(),
        fields: vec![Field { name: "q".into(), ty: boolean }, Field { name: "last".into(), ty: boolean }],
    });
    let r = b.reference(rtrig, true);
    let selff = |f: u32| proj(1, vec![Projection::Deref, Projection::Field(f)]);
    // self.q = clk && !self.last; self.last = clk; self.q
    let scan = b.function(Function {
        name: "RTrig::scan".into(),
        params: 2,
        locals: locals(&[boolean, r, boolean, boolean, boolean]),
        blocks: vec![
            block(vec![], Terminator::SwitchInt { value: copy(2), targets: vec![(0, BlockId(2))], otherwise: BlockId(1), at: at(1) }),
            block(
                vec![assign(place(4), Value::Use(copy_at(selff(1))), 1), assign(place(3), Value::Unary(UnOp::Not, copy(4)), 1)],
                Terminator::Goto(BlockId(3)),
            ),
            block(vec![assign(place(3), Value::Use(konst(boolean, vec![Scalar::Bool(false)])), 1)], Terminator::Goto(BlockId(3))),
            block(
                vec![
                    assign(selff(0), Value::Use(copy(3)), 1),
                    assign(selff(1), Value::Use(copy(2)), 2),
                    assign(place(0), Value::Use(copy_at(selff(0))), 3),
                ],
                ret(),
            ),
        ],
    });
    let mut m = Machine::load(b.program, &[rtrig]).unwrap();
    for pattern in 0u32..1024 {
        m.set_region(RegionId(0), &[Scalar::Bool(false), Scalar::Bool(false)]).unwrap();
        let mut native = orcl_std::RTrig::default();
        for k in 0..10 {
            let clk = pattern >> k & 1 == 1;
            let arg = [Scalar::Bool(clk)];
            let got = m.call(scan, &[Arg::Ref(RegionId(0)), Arg::Value(&arg)]).unwrap().to_vec();
            assert_eq!(got, vec![Scalar::Bool(native.scan(clk))], "pattern {pattern:010b}, scan {k}");
            assert_eq!(m.region(RegionId(0)).unwrap()[0], Scalar::Bool(native.q()));
        }
    }
}
