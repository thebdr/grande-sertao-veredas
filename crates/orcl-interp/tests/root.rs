//! The crate root, every line but its `//!` documentation, as it must stand: unsafe code forbidden and the panic
//! lints denied, so clippy's run refuses a panic path in the library; its modules and exports, nothing else.
#[test]
fn the_crate_root_is_exactly_its_attributes_its_modules_and_its_exports() {
    let root: Vec<&str> = include_str!("../src/lib.rs")
        .lines()
        .filter(|line| !line.starts_with("//!") && !line.trim().is_empty())
        .collect();
    assert_eq!(
        root,
        vec![
            "#![forbid(unsafe_code)]",
            "#![deny(",
            "    clippy::arithmetic_side_effects,",
            "    clippy::indexing_slicing,",
            "    clippy::panic,",
            "    clippy::unwrap_used,",
            "    clippy::expect_used",
            ")]",
            "mod core_ops;",
            "pub mod file;",
            "mod layout;",
            "mod machine;",
            "mod ops;",
            "mod program;",
            "pub use core_ops::CoreOp;",
            "pub use layout::LoadError;",
            "pub use machine::{Arg, CallError, ErrorKind, HostError, Machine, ProgramError, RegionId, Step};",
            "pub use ops::Scalar;",
            "pub use program::{",
            "    BinOp, Block, BlockId, Callee, Check, Const, Field, Function, FunctionId, Int, Local, LocalDecl, Operand, Place,",
            "    Program, Projection, Span, Statement, Terminator, Type, TypeId, UnOp, Value, Variant,",
            "};",
        ]
    );
}

const SOURCES: [(&str, &str); 7] = [
    ("core_ops.rs", include_str!("../src/core_ops.rs")),
    ("file.rs", include_str!("../src/file.rs")),
    ("layout.rs", include_str!("../src/layout.rs")),
    ("lib.rs", include_str!("../src/lib.rs")),
    ("machine.rs", include_str!("../src/machine.rs")),
    ("ops.rs", include_str!("../src/ops.rs")),
    ("program.rs", include_str!("../src/program.rs")),
];

#[test]
fn no_source_allows_back_what_the_root_denies_and_every_source_is_read() {
    let mut listed: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    listed.sort();
    assert_eq!(listed, SOURCES.map(|(name, _)| name.to_string()).to_vec(), "every file of src/ is read here");
    for (name, text) in SOURCES {
        let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        for word in ["allow(", "expect(", "cfg_attr(", "#[path", "include!(", "unsafe"] {
            let attribute = squeezed.contains(&format!("#[{word}")) || squeezed.contains(&format!("#![{word}"));
            let anywhere = matches!(word, "#[path" | "include!(" | "unsafe") && squeezed.contains(word);
            let allowed = name == "lib.rs" && word == "unsafe" && squeezed.matches("unsafe").count() == 1;
            assert!(!(attribute || anywhere) || allowed, "{name} holds `{word}`");
        }
    }
}
