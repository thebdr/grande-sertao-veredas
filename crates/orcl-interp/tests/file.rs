//! The program file: written and read back to the same program, its bytes pinned for one small program, a file of
//! another format or language refused by its header before anything else is read, and every truncated or damaged
//! copy of a file either refused at a byte or read to a program that writes back to the very same bytes.
mod cap;
mod common;
use common::*;

#[global_allocator]
static CAPPED: cap::Capped = cap::Capped;
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
    let mut written = Machine::load(program, &[], LIMITS).unwrap();
    let mut read = Machine::load(back, &[], LIMITS).unwrap();
    for n in [0, 1, 10, 1000] {
        let arg = [int(Int::U64, n)];
        let w = written.call(f, &[Arg::Value(&arg)]).map(|r| r.to_vec());
        let r = read.call(f, &[Arg::Value(&arg)]).map(|r| r.to_vec());
        assert_eq!(w, Ok(vec![int(Int::U64, n * (n + 1) / 2)]));
        assert_eq!((r, read.steps(), read.memory()), (w, written.steps(), written.memory()), "sum({n})");
    }
}

#[test]
#[ignore = "writes the fixture; run once by hand when the format changes on purpose, with its version"]
fn write_the_every_kind_fixture() {
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/every_kind.orclb"), file::write(&header(), &every_kind())).unwrap();
}

#[test]
fn every_kind_s_bytes_are_pinned_so_no_code_of_the_format_drifts_unseen() {
    // a code swapped in both the writer and the reader still reads its own files back; only the bytes say it moved
    let pinned: &[u8] = include_bytes!("every_kind.orclb");
    let bytes = file::write(&header(), &every_kind());
    assert!(bytes == pinned, "every kind's bytes differ from the pinned ones ({} against {} bytes)", bytes.len(), pinned.len());
    let (h, back) = file::read(pinned).unwrap();
    assert_eq!((h, format!("{back:?}")), (header(), format!("{:?}", every_kind())));
}

#[test]
fn a_file_of_another_format_is_said_so_whatever_its_language_says_or_however_short() {
    let mut bytes = file::write(&header(), &every_kind());
    bytes[6] = 2;
    bytes[8] = 7;
    assert_eq!(file::read(&bytes), Err(FileError::Format { found: 2 }), "format 2, its language field 7");
    assert_eq!(file::read(&bytes[..8]), Err(FileError::Format { found: 2 }), "format 2 and nothing after");
    assert_eq!(file::read(&bytes[..7]), Err(FileError::Malformed { at: 6, what: "cut short" }), "half a format");
    let mut ours = file::write(&header(), &every_kind());
    ours[8] = 7;
    assert_eq!(file::read(&ours[..10]), Err(FileError::Language { found: 7 }), "format 1, language 7, nothing after");
}

#[test]
fn an_integer_outside_its_type_and_a_bool_neither_0_nor_1_are_refused_at_their_bytes() {
    // fn f() -> u8 { _0 = const 1_u8 }, the constant's value set to 300, then its flag-free cousin with a bool
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[u8t]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 1)])), 1)], ret())],
    });
    let mut bytes = file::write(&header(), &b.program);
    let one = 1i128.to_le_bytes();
    let at = bytes.windows(16).position(|w| w == one).expect("the constant's value");
    bytes[at..at + 16].copy_from_slice(&300i128.to_le_bytes());
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at, what: "an integer outside its type" }));
    let mut b = Build::default();
    let boolean = b.bool();
    let r = b.reference(boolean, true);
    let _ = r;
    let mut bytes = file::write(&header(), &b.program);
    // the reference type's mutability, its last byte before the function count
    let at = bytes.len() - 5;
    assert_eq!(bytes[at], 1);
    bytes[at] = 2;
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at, what: "a bool that is neither 0 nor 1" }));
}

#[test]
fn an_empty_program_is_written_and_read_back() {
    let bytes = file::write(&header(), &Program::default());
    assert_eq!(file::read(&bytes), Ok((header(), Program::default())));
}

/// The pinned small program's file: `fn f() -> bool { _0 = const true; return }`, under the header "n", "r".
fn small() -> Vec<u8> {
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
    file::write(&Header { language: 1, nightly: "n".into(), revision: "r".into() }, &b.program)
}

#[test]
fn a_tag_no_program_has_is_refused_at_its_own_byte_for_every_kind_of_tag() {
    // the small file's bytes, as the pinned test spells them: the type's tag at 36, the value's at 75, the
    // operand's at 76, the scalar's at 85, the terminator's at 99
    let bytes = small();
    for (at, was, what) in [
        (36, 0, "no such type kind"),
        (75, 0, "no such value kind"),
        (76, 2, "no such operand kind"),
        (85, 1, "no such scalar kind"),
        (99, 2, "no such terminator"),
    ] {
        assert_eq!(bytes[at], was, "the tag at {at}");
        let mut d = bytes.clone();
        d[at] = 0xEE;
        assert_eq!(file::read(&d), Err(FileError::Malformed { at, what }), "a tag of 0xEE at {at}");
    }
}

#[test]
fn a_count_one_past_the_bytes_left_is_refused_at_the_count() {
    let mut bytes = small();
    // the functions' count, after the one type's byte
    let at = 37;
    assert_eq!(bytes[at..at + 4], [1, 0, 0, 0]);
    let left = (bytes.len() - at - 4) as u32;
    bytes[at..at + 4].copy_from_slice(&(left + 1).to_le_bytes());
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at, what: "a count past the file's end" }));
    // as many functions as bytes left is not refused for its count: the reader reads the one there is, then finds
    // the file's end
    bytes[at..at + 4].copy_from_slice(&left.to_le_bytes());
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at: bytes.len(), what: "cut short" }));
    // a count inside a function, its blocks' at 59, one past its own bytes left
    let mut bytes = small();
    let at = 59;
    assert_eq!(bytes[at..at + 4], [1, 0, 0, 0]);
    let left = (bytes.len() - at - 4) as u32;
    bytes[at..at + 4].copy_from_slice(&(left + 1).to_le_bytes());
    assert_eq!(file::read(&bytes), Err(FileError::Malformed { at, what: "a count past the file's end" }));
}

/// A program of one function whose only block holds `s` and ends in `t`, over the types `u8` and `[u8; 2]`, its
/// local 1 named `name`.
fn one_block(s: Statement, t: Terminator, name: Option<String>) -> Vec<u8> {
    let mut b = Build::default();
    let (u8t, arr) = (b.int(Int::U8), b.ty(Type::Array { element: TypeId(0), len: 2 }));
    let mut decls = locals(&[u8t, arr]);
    decls[1].name = name;
    b.function(Function { name: "f".into(), params: 0, locals: decls, blocks: vec![block(vec![s], t)] });
    file::write(&header(), &b.program)
}

fn first_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).position(|(x, y)| x != y).expect("a difference")
}

fn refused_at(bytes: &[u8], at: usize, what: &'static str) {
    let mut d = bytes.to_vec();
    d[at] = 0xEE;
    assert_eq!(file::read(&d), Err(FileError::Malformed { at, what }), "a tag of 0xEE at {at}");
}

#[test]
fn every_other_kind_of_tag_is_refused_at_its_own_byte() {
    // each tag found as the first byte where two programs differ only in it
    let use0 = |v: Value| assign(place(0), v, 1);
    let r = Terminator::Return { at: at(1) };
    let core = |op| Terminator::Call { callee: Callee::Core(op), args: vec![], dest: place(0), next: BlockId(0), at: at(1) };
    let c = || konst(TypeId(0), vec![int(Int::U8, 1)]);
    let types = |t: Int| {
        let mut b = Build::default();
        b.int(t);
        file::write(&header(), &b.program)
    };
    let (a, b2) = (types(Int::U8), types(Int::U16));
    refused_at(&a, first_difference(&a, &b2), "no such integer type");
    let a = one_block(assign(proj(1, vec![Projection::Field(0)]), Value::Use(c()), 1), r.clone(), None);
    let b2 = one_block(assign(proj(1, vec![Projection::ConstantIndex(0)]), Value::Use(c()), 1), r.clone(), None);
    refused_at(&a, first_difference(&a, &b2), "no such projection");
    let a = one_block(use0(Value::Use(c())), core(CoreOp::DurationFromMillis), None);
    let b2 = one_block(use0(Value::Use(c())), core(CoreOp::DurationFromSecs), None);
    refused_at(&a, first_difference(&a, &b2), "no such core operation");
    let a = one_block(use0(Value::Unary(UnOp::Not, c())), r.clone(), None);
    let b2 = one_block(use0(Value::Unary(UnOp::Neg, c())), r.clone(), None);
    refused_at(&a, first_difference(&a, &b2), "no such unary operation");
    let a = one_block(use0(Value::Binary(BinOp::Add, c(), c())), r.clone(), None);
    let b2 = one_block(use0(Value::Binary(BinOp::Sub, c(), c())), r.clone(), None);
    refused_at(&a, first_difference(&a, &b2), "no such operation");
    let assert = |check| Terminator::Assert { cond: c(), expected: true, check, next: BlockId(0), at: at(1) };
    let a = one_block(use0(Value::Use(c())), assert(Check::DivisionByZero), None);
    let b2 = one_block(use0(Value::Use(c())), assert(Check::RemainderByZero), None);
    refused_at(&a, first_difference(&a, &b2), "no such check");
    let call = |callee| Terminator::Call { callee, args: vec![], dest: place(0), next: BlockId(0), at: at(1) };
    let a = one_block(use0(Value::Use(c())), call(Callee::Function(FunctionId(0))), None);
    let b2 = one_block(use0(Value::Use(c())), call(Callee::Core(CoreOp::Min(Int::I8))), None);
    refused_at(&a, first_difference(&a, &b2), "no such callee");
    let a = one_block(use0(Value::Aggregate(TypeId(1), None, vec![])), r.clone(), None);
    let b2 = one_block(use0(Value::Aggregate(TypeId(1), Some(0), vec![])), r.clone(), None);
    refused_at(&a, first_difference(&a, &b2), "an option that is neither 0 nor 1");
    let a = one_block(use0(Value::Use(c())), r.clone(), None);
    let b2 = one_block(use0(Value::Use(c())), r.clone(), Some(String::new()));
    refused_at(&a, first_difference(&a, &b2), "an option that is neither 0 nor 1");
}
