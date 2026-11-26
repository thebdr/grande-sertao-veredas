//! On stable the crate is a shell that names the nightly its driver is pinned to (`docs/questions.md` Q221, Q222),
//! and the crate's `rust-toolchain`, rustup's plain one-line form, names the same nightly.
// With the driver's feature the library links the compiler's own `rustc_driver`, and so does every test of it.
#![cfg_attr(feature = "driver", feature(rustc_private))]
#[cfg(feature = "driver")]
extern crate rustc_driver;

#[test]
fn the_toolchain_file_names_the_nightly_the_crate_names() {
    let file = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/rust-toolchain")).unwrap();
    assert_eq!(file, format!("{}\n", orcl_front::NIGHTLY), "one line, the nightly alone");
    assert!(orcl_front::NIGHTLY.starts_with("nightly-"), "a dated nightly");
}
