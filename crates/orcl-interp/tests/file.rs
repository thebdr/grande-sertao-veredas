//! The program file: written and read back to the same program, its bytes pinned for one small program, a file of
//! another format or language refused by its header before anything else is read, and every truncated or damaged
//! copy of a file either refused at a byte or read to a program that writes back to the very same bytes.
mod common;
use common::*;
use orcl_interp::file::{self, FileError, Header, FORMAT, LANGUAGES, MAGIC};
use orcl_interp::*;

fn header() -> Header {
    Header { language: 1, nightly: "nightly-2026-11-21".into(), revision: "abc123".into() }
}

/// A program that holds every kind the format has: every type kind, projection, operand, scalar, value,
/// terminator, callee, check and core operation. It need not load; it must come back.
fn every_kind() -> Program {
    let mut b = Build::default();
    let mut types = vec![b.bool(), b.ty(Type::F32), b.ty(Type::F64), b.ty(Type::Duration)];
    for t in INTS {
        types.push(b.int(t));
    }
    let u8t = b.int(Int::U8);
    let s = b.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] });
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::I16,
        variants: vec![
            Variant { name: "A".into(), discriminant: -7, fields: vec![] },
            Variant { name: "B".into(), discriminant: 300, fields: vec![Field { name: "0".into(), ty: s }] },
        ],
    });
    let arr = b.ty(Type::Array { element: s, len: 9 });
    let r = b.reference(arr, true);
    let _ = b.reference(e, false);
    let p = |l| Place { local: Local(l), projections: vec![Projection::Field(1), Projection::Deref, Projection::Index(Local(2)), Projection::ConstantIndex(3), Projection::Downcast(4)] };
    let scalars = vec![
        Scalar::Uninit,
        Scalar::Bool(true),
        Scalar::Bool(false),
        int(Int::I64, i64::MIN as i128),
        int(Int::U64, u64::MAX as i128),
        Scalar::F32(-0.0),
        Scalar::F32(f32::from_bits(0x7fc0_0001)),
        Scalar::F64(f64::NEG_INFINITY),
        Scalar::Ref(77),
    ];
    let c = Operand::Const(Const { ty: r, slots: scalars });
    let mut statements = vec![];
    for op in [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem, BinOp::BitAnd, BinOp::BitOr, BinOp::BitXor, BinOp::Shl, BinOp::Shr, BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge] {
        statements.push(assign(p(1), Value::Binary(op, Operand::Copy(p(2)), Operand::Move(p(3))), 1));
        statements.push(assign(place(1), Value::Checked(op, copy(2), copy(0)), 2));
    }
    statements.extend([
        assign(p(0), Value::Use(c.clone()), 3),
        assign(p(0), Value::Unary(UnOp::Not, copy(1)), 4),
        assign(p(0), Value::Unary(UnOp::Neg, copy(1)), 4),
        assign(p(0), Value::Cast(copy(1), u8t), 5),
        assign(p(0), Value::Aggregate(s, None, vec![copy(1), c.clone()]), 6),
        assign(p(0), Value::Aggregate(e, Some(1), vec![]), 6),
        assign(p(0), Value::Discriminant(p(5)), 7),
        assign(p(0), Value::Ref(p(6), true), 8),
        assign(p(0), Value::Ref(p(6), false), 8),
    ]);
    let core_ops = {
        // every integer type once, through the first operation; every other operation at one type of its own
        let mut v: Vec<CoreOp> = INTS.iter().map(|t| CoreOp::SaturatingAdd(*t)).collect();
        v.extend([
            CoreOp::SaturatingSub(Int::I8),
            CoreOp::SaturatingMul(Int::U16),
            CoreOp::WrappingAdd(Int::I32),
            CoreOp::WrappingSub(Int::U64),
            CoreOp::WrappingMul(Int::Isize),
            CoreOp::CheckedShl(Int::Usize),
            CoreOp::CheckedShr(Int::I16),
            CoreOp::Min(Int::U8),
            CoreOp::Max(Int::I64),
            CoreOp::DurationFromMillis,
            CoreOp::DurationFromSecs,
            CoreOp::DurationSaturatingSub,
            CoreOp::DurationMin,
            CoreOp::DurationMax,
            CoreOp::OptionGetOrInsert,
            CoreOp::OptionIsNone,
            CoreOp::OptionIsSome,
            CoreOp::Default,
        ]);
        for op in [BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge] {
            v.push(CoreOp::DurationCompare(op));
        }
        v
    };
    let span = Span { file: 1, line: 70_000, column: 33 };
    let mut blocks = vec![block(statements, Terminator::Goto(BlockId(1)))];
    blocks.push(block(vec![], Terminator::SwitchInt { value: copy(1), targets: vec![(i128::MIN, BlockId(0)), (i128::MAX, BlockId(2))], otherwise: BlockId(3), at: span }));
    blocks.push(block(vec![], Terminator::Return { at: span }));
    blocks.push(block(vec![], Terminator::Unreachable { at: span }));
    for check in [Check::Overflow(BinOp::Shl), Check::NegOverflow, Check::DivisionByZero, Check::RemainderByZero, Check::Bounds] {
        blocks.push(block(vec![], Terminator::Assert { cond: copy(1), expected: true, check, next: BlockId(0), at: span }));
        blocks.push(block(vec![], Terminator::Assert { cond: copy(1), expected: false, check, next: BlockId(0), at: span }));
    }
    blocks.push(block(vec![], Terminator::Call { callee: Callee::Function(FunctionId(0)), args: vec![copy(1), c.clone()], dest: p(0), next: BlockId(1), at: span }));
    for op in core_ops {
        blocks.push(block(vec![], Terminator::Call { callee: Callee::Core(op), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: span }));
    }
    types.extend([s, e, arr, r]);
    let mut decls = locals(&types);
    decls[1].name = Some("named".into());
    decls[2].name = Some("ünïcode → ✓".into());
    b.function(Function { name: "Every::kind".into(), params: 3, locals: decls, blocks });
    b.function(Function { name: String::new(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![], Terminator::Return { at: Span::default() })] });
    b.program.files = vec!["src/lib.rs".into(), "src/a b/ç.rs".into()];
    b.program
}

#[test]
fn every_kind_of_instruction_is_written_and_read_back_to_the_same_program() {
    let program = every_kind();
    let bytes = file::write(&header(), &program);
    let (h, back) = file::read(&bytes).expect("read back");
    assert_eq!(h, header());
    assert_eq!(format!("{back:?}"), format!("{program:?}"));
    // written again to the same bytes: every float's bits included, which neither `==` nor `Debug` can say of NaN
    assert_eq!(file::write(&h, &back), bytes);
}

#[test]
fn a_small_program_s_bytes_are_pinned() {
    // fn f() -> bool { _0 = const true; return }, at line 2, column 5 of file 0, "f.rs"
    let mut b = Build::default();
    let boolean = b.bool();
    let span = Span { file: 0, line: 2, column: 5 };
    b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[boolean]),
        blocks: vec![Block {
            statements: vec![Statement { place: place(0), value: Value::Use(konst(boolean, vec![Scalar::Bool(true)])), at: span }],
            terminator: Terminator::Return { at: span },
        }],
    });
    b.program.files.push("f.rs".into());
    let h = Header { language: 1, nightly: "n".into(), revision: "r".into() };
    let bytes = file::write(&h, &b.program);
    #[rustfmt::skip]
    let want: Vec<u8> = vec![
        b'O', b'R', b'C', b'L', b'B', 0,           // magic
        1, 0,                                       // format 1
        1, 0,                                       // ORCL 1
        1, 0, 0, 0, b'n',                           // the nightly
        1, 0, 0, 0, b'r',                           // the revision
        1, 0, 0, 0, 4, 0, 0, 0, b'f', b'.', b'r', b's', // one file
        1, 0, 0, 0, 0,                              // one type: bool
        1, 0, 0, 0,                                 // one function
        1, 0, 0, 0, b'f',                           //   its name
        0, 0, 0, 0,                                 //   no parameter
        1, 0, 0, 0, 0, 0, 0, 0, 0,                  //   one local, type 0, no name
        1, 0, 0, 0,                                 //   one block
        1, 0, 0, 0,                                 //     one statement
        0, 0, 0, 0, 0, 0, 0, 0,                     //       place _0, no projection
        0, 2, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1,         //       use a constant of type 0: one slot, a bool, true
        0, 0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0,         //       at 0:2:5
        2, 0, 0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0,      //     return, at 0:2:5
    ];
    assert_eq!(bytes, want);
    assert_eq!(&bytes[..6], &MAGIC);
    assert_eq!(FORMAT, 1);
}

#[test]
fn a_file_of_another_format_or_language_is_refused_by_its_header_before_anything_else() {
    let bytes = file::write(&header(), &every_kind());
    assert_eq!(file::read(b""), Err(FileError::NotAProgram));
    assert_eq!(file::read(b"ORCLB"), Err(FileError::NotAProgram));
    assert_eq!(file::read(b"ORCLC\0\x01\0\x01\0"), Err(FileError::NotAProgram));
    let mut other = bytes.clone();
    other[6] = 2;
    assert_eq!(file::read(&other), Err(FileError::Format { found: 2 }));
    for language in [0u16, 2, 1000] {
        // a language past the header, the rest damaged: the language is what is said
        let mut other = bytes[..10].to_vec();
        other[8..10].copy_from_slice(&language.to_le_bytes());
        other.extend([0xFF; 3]);
        assert_eq!(file::read(&other), Err(FileError::Language { found: language }));
    }
    assert!(LANGUAGES.contains(&orcl_std::LANGUAGE), "the interpreter runs the language orcl-std states");
    assert_eq!(LANGUAGES, &[1]);
}

#[test]
fn every_truncated_copy_is_refused_and_bytes_past_the_end_too() {
    let bytes = file::write(&header(), &every_kind());
    for len in 0..bytes.len() {
        match file::read(&bytes[..len]) {
            Err(FileError::NotAProgram) => assert!(len < MAGIC.len(), "cut at {len}"),
            Err(FileError::Malformed { at, .. }) => assert!(at <= len, "cut at {len}, refused at {at}"),
            other => panic!("cut at {len}: {other:?}"),
        }
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(file::read(&longer), Err(FileError::Malformed { at: bytes.len(), what: "bytes past the program's end" }));
}

#[test]
fn every_damaged_byte_is_refused_or_reads_to_a_program_that_writes_back_to_those_very_bytes() {
    let bytes = file::write(&header(), &every_kind());
    let (mut refused, mut read) = (0, 0);
    for i in 0..bytes.len() {
        for damage in [0xFFu8, 0x01, 0x80] {
            let mut d = bytes.clone();
            d[i] ^= damage;
            match file::read(&d) {
                Ok((h, p)) => {
                    assert_eq!(file::write(&h, &p), d, "byte {i} ^ {damage:#x} read, but not to itself");
                    read += 1;
                }
                Err(_) => refused += 1,
            }
        }
    }
    assert!(refused > 0 && read > 0, "{refused} refused, {read} read");
}

#[test]
fn a_count_past_the_end_and_text_that_is_not_utf_8_are_refused_where_they_stand() {
    let h = header();
    let mut bytes = file::write(&h, &Program::default());
    // the files' count, just after the header, set past the end
    let at = 6 + 2 + 2 + 4 + h.nightly.len() + 4 + h.revision.len();
    bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at, what: "a count past the file's end" }));
    let bad = Header { language: 1, nightly: "x".into(), revision: "y".into() };
    let mut bytes = file::write(&bad, &Program::default());
    bytes[14] = 0xC3; // "x" becomes half of a two-byte character
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at: 14, what: "text that is not UTF-8" }));
}

#[test]
fn a_program_read_from_its_file_runs_as_the_one_written() {
    // fn sum(n: u64) -> u64 { let mut s = 0; while n > 0 { s = s.saturating_add(n); n -= 1 (wrapping) } s }
    let mut b = Build::default();
    let (u64t, boolean) = (b.int(Int::U64), b.bool());
    let c = |v| konst(u64t, vec![int(Int::U64, v)]);
    let f = b.function(Function {
        name: "sum".into(),
        params: 1,
        locals: locals(&[u64t, u64t, u64t, boolean]),
        blocks: vec![
            block(vec![assign(place(2), Value::Use(c(0)), 1)], Terminator::Goto(BlockId(1))),
            block(
                vec![assign(place(3), Value::Binary(BinOp::Gt, copy(1), c(0)), 2)],
                Terminator::SwitchInt { value: copy(3), targets: vec![(0, BlockId(3))], otherwise: BlockId(2), at: at(2) },
            ),
            block(
                vec![],
                Terminator::Call {
                    callee: Callee::Core(CoreOp::SaturatingAdd(Int::U64)),
                    args: vec![copy(2), copy(1)],
                    dest: place(2),
                    next: BlockId(4),
                    at: at(3),
                },
            ),
            block(vec![assign(place(0), Value::Use(copy(2)), 5)], ret()),
            block(vec![assign(place(1), Value::Binary(BinOp::Sub, copy(1), c(1)), 4)], Terminator::Goto(BlockId(1))),
        ],
    });
    let program = b.program;
    let (_, back) = file::read(&file::write(&header(), &program)).unwrap();
    let mut written = Machine::load(program, &[]).unwrap();
    let mut read = Machine::load(back, &[]).unwrap();
    for n in [0, 1, 10, 1000] {
        let arg = [int(Int::U64, n)];
        let w = written.call(f, &[Arg::Value(&arg)]).map(|r| r.to_vec());
        let r = read.call(f, &[Arg::Value(&arg)]).map(|r| r.to_vec());
        assert_eq!(w, Ok(vec![int(Int::U64, n * (n + 1) / 2)]));
        assert_eq!((r, read.steps(), read.memory()), (w, written.steps(), written.memory()), "sum({n})");
    }
}
