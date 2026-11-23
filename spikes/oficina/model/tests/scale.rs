//! How long the tree takes to read a folder of 100 000 files, the model alone (the spike's
//! point 1 measures it again through the interface). Printed, not judged: a timing judged here
//! would depend on the load of the machine.

use std::fs;
use std::time::Instant;

use oficina_spike_model::tree::Tree;

#[test]
fn a_folder_of_100000_files_is_read_once_and_counted() {
    // Under `OFICINA_SCRATCH` when a run names one (a guest names its own disk), the build's else.
    let base = std::env::var_os("OFICINA_SCRATCH").map(std::path::PathBuf::from).unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    let dir = tempfile::tempdir_in(base).expect("a scratch folder");
    let many = dir.path().join("many");
    fs::create_dir_all(&many).expect("a folder");
    for i in 0..100_000 {
        fs::File::create(many.join(format!("f{i:06}.log"))).expect("a file");
    }
    let mut t = Tree::open(dir.path()).expect("the folder opens");
    let folder = t.children(t.root()).expect("read")[0];
    let start = Instant::now();
    let listing = t.read_folder(folder).expect("it lists");
    let listed = start.elapsed();
    let n = t.attach(listing).len();
    eprintln!("100000 files: listed and sorted in {listed:?}, attached in {:?}", start.elapsed() - listed);
    assert_eq!(n, 100_000);
    assert_eq!(t.node(t.children(folder).expect("read")[0]).name, "f000000.log");
}
