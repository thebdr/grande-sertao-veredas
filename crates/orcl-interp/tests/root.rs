//! The crate root, every line but its `//!` documentation, as it must stand: unsafe code forbidden and the panic
//! lints denied, so clippy's run refuses the panic paths those lints name; no source holding a panic macro they
//! miss (`unreachable!`, `todo!`, `unimplemented!`, the assertions); its modules and exports, nothing else.
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
            "pub use machine::{Arg, CallError, ErrorKind, HostError, Limits, Machine, ProgramError, RegionId, Step};",
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

#[test]
fn the_interpreter_depends_on_nothing_but_for_its_tests() {
    // cargo's own report of the crate's normal and build dependencies, on every target and with every feature
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let out = std::process::Command::new(env!("CARGO"))
        .args(["tree", "-p", "orcl-interp", "-e", "normal,build", "--depth", "1", "--prefix", "none", "--all-features"])
        .args(["--target", "all", "--offline", "--manifest-path", manifest])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let listed = String::from_utf8(out.stdout).unwrap();
    let crates: Vec<&str> = listed.lines().filter_map(|line| line.split_whitespace().next()).collect();
    assert_eq!(crates, ["orcl-interp"], "the interpreter alone, with nothing under it");
}

/// No panic macro the denied lints do not see (`unreachable!`, `todo!`, `unimplemented!`, the assertions).
#[test]
fn no_source_holds_a_panic_macro_the_denied_lints_miss() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text: String = std::fs::read_to_string(&path).unwrap().chars().filter(|c| !c.is_whitespace()).collect();
        for word in ["unreachable!", "todo!", "unimplemented!", "assert!", "assert_eq!", "assert_ne!", "debug_assert"] {
            assert!(!text.contains(word), "{} holds `{word}`", path.display());
        }
    }
}

#[test]
fn no_source_reaches_std_prints_or_reads_a_clock() {
    // std's prelude alone: no path into std (so no clock, thread, file, socket, environment or process), no print to
    // the terminal, no clock by name, no crate brought in by name; comment lines left out, the code's words read
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        // no block comment, so every line the reading drops as a comment is one (round 15)
        assert!(!text.contains("/*"), "{} holds a block comment", path.display());
        let code: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
        let code = code.join("\n");
        let words: Vec<&str> = code.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        // std's macros that reach a thread or the processor with no path named are std too (round 15)
        for refused in ["std", "SystemTime", "UNIX_EPOCH", "Instant", "print", "println", "eprint", "eprintln", "dbg", "thread_local", "is_x86_feature_detected"] {
            assert!(!words.contains(&refused), "{} names `{refused}`", path.display());
        }
        let squeezed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(!squeezed.contains("externcrate"), "{} brings a crate in by name", path.display());
    }
}

// The twenty-fifth refute round's kill-test, kept for the clause's next change and adopted with Q230's: a panic path
// a denied lint names (`unwrap`, `expect`, indexing), written in a form the lint does not see: inside a local
// `macro_rules!`, as a function value, or as a path call to `Index::index`.
#[test]
fn no_source_hides_a_panic_the_denied_lints_name_in_a_form_they_miss() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        // comment lines left out, as the root's reading of std does; the code's words read
        let code: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
        let code = code.join("\n");
        let words: Vec<&str> = code.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        for refused in ["macro_rules", "unwrap", "expect", "unwrap_err", "expect_err", "unwrap_unchecked"] {
            assert!(!words.contains(&refused), "{} names `{refused}`", path.display());
        }
        let squeezed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        for call in ["Index::index", "IndexMut::index_mut"] {
            assert!(!squeezed.contains(call), "{} calls `{call}`", path.display());
        }
    }
}
