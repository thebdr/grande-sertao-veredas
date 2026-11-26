//! C-294: what a release carries of its licences (docs/questions.md Q35): every component and
//! every package its builds may compile stating one, or the cut refuses it before it builds
//! (`tests/bundle.rs` shows the cut stopping); the bundle's `licenses/`, made from the component
//! list (C-295): each component's and each package by path's licence files as git stores them,
//! each crate's as its vendored sources ship them, Qt's and KDDockWidgets' texts with where their
//! sources are, and `THIRD-PARTY.md` naming every entry.

use std::path::Path;
use std::process::Command;

struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Repo {
        let r = Repo { dir: tempfile::tempdir().unwrap() };
        r.git(&["init", "-q", "-b", "main"]);
        r
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

#[test]
fn a_packages_licence_files_are_known_by_their_names() {
    for name in ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "license.txt", "COPYING", "COPYING.LESSER", "NOTICE", "UNLICENSE", "COPYRIGHT", "Licence.md"] {
        assert!(alforge::licences::is_licence_file(name), "{name}");
    }
    for name in ["README.md", "Cargo.toml", "src", "build.rs", "notes.md"] {
        assert!(!alforge::licences::is_licence_file(name), "{name}");
    }
}

#[test]
fn an_entry_stating_no_licence_is_named() {
    let sbom = r#"{"components":[{"bom-ref":"a@1","licenses":[{"expression":"MIT"}]},{"bom-ref":"b@2"},{"bom-ref":"c@3","licenses":[{"license":{"name":"the package's own file c/LICENCA"}}]}]}"#;
    assert_eq!(alforge::licences::unlicensed(sbom), ["b@2"]);
}

#[test]
fn the_vendored_crates_licence_files_are_kept_and_nothing_else() {
    let d = tempfile::tempdir().unwrap();
    let vendor = d.path().join("vendor");
    for (f, t) in [("itoa-1.0.19/LICENSE-MIT", "mit"), ("itoa-1.0.19/LICENSE-APACHE", "apache"), ("itoa-1.0.19/src/lib.rs", "code"), ("itoa-1.0.19/Cargo.toml", "toml"), ("zlib-rs-0.5.0/COPYING", "zlib")] {
        std::fs::create_dir_all(vendor.join(f).parent().unwrap()).unwrap();
        std::fs::write(vendor.join(f), t).unwrap();
    }
    std::fs::write(vendor.join("stray.txt"), "a file, no crate").unwrap();
    let kept = d.path().join("kept");
    alforge::licences::keep_vendored(&vendor, &kept).unwrap();
    let mut found: Vec<String> = Vec::new();
    for c in std::fs::read_dir(&kept).unwrap() {
        let c = c.unwrap();
        for f in std::fs::read_dir(c.path()).unwrap() {
            found.push(format!("{}/{}", c.file_name().to_string_lossy(), f.unwrap().file_name().to_string_lossy()));
        }
    }
    found.sort();
    assert_eq!(found, ["itoa-1.0.19/LICENSE-APACHE", "itoa-1.0.19/LICENSE-MIT", "zlib-rs-0.5.0/COPYING"]);
    // No vendored sources at all: nothing kept, no error.
    alforge::licences::keep_vendored(&d.path().join("none"), &kept).unwrap();
    assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 0);
}

#[test]
fn a_bundles_licences_name_every_entry_of_its_component_list_with_its_files() {
    // A suite of a component by path, a fork by path, a real crate (locked offline by the machine's
    // own cargo), and the recipe naming Qt for Oficina's window.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-portal\"]\nexclude = [\"forks/garfo\"]\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.3\"\nedition = \"2024\"\nlicense = \"MIT OR Apache-2.0\"\n\n[dependencies]\ngarfo = { path = \"../../forks/garfo\" }\nitoa = \"1\"\n",
    );
    r.write("crates/veredas-portal/src/main.rs", "fn main() {}\n");
    r.write("crates/veredas-portal/LICENSE-MIT", "portal's MIT text\n");
    r.write("crates/veredas-portal/LICENSE-APACHE", "portal's Apache text\n");
    r.write("forks/garfo/Cargo.toml", "[workspace]\n\n[package]\nname = \"garfo\"\nversion = \"0.3.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n");
    r.write("forks/garfo/src/lib.rs", "\n");
    r.write("forks/garfo/LICENSE", "the fork's MIT text\n");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    r.write("tools/alforge/maquina/maquina.toml", &std::fs::read_to_string(root.join("tools/alforge/maquina/maquina.toml")).unwrap());
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "a suite with its licences"]);
    let c = r.git(&["rev-parse", "HEAD"]);
    let sbom = alforge::components::list(r.path(), &c, Some("2.0.0")).unwrap();
    assert_eq!(alforge::licences::unlicensed(&sbom), Vec::<String>::new());
    let doc: serde_json::Value = serde_json::from_str(&sbom).unwrap();
    let itoa = doc["components"].as_array().unwrap().iter().find(|e| e["name"] == "itoa").unwrap()["version"].as_str().unwrap().to_string();

    // The vendored crate's files, as the builder kept them; Debian's licence texts; KDDockWidgets'.
    let d = tempfile::tempdir().unwrap();
    let vendored = d.path().join("vendored");
    std::fs::create_dir_all(vendored.join(format!("itoa-{itoa}"))).unwrap();
    std::fs::write(vendored.join(format!("itoa-{itoa}/LICENSE-MIT")), "itoa's MIT text\n").unwrap();
    let common = d.path().join("common-licenses");
    std::fs::create_dir_all(&common).unwrap();
    for t in ["LGPL-3", "GPL-2", "GPL-3"] {
        std::fs::write(common.join(t), format!("the {t} text\n")).unwrap();
    }
    let kdd = d.path().join("kdd");
    std::fs::create_dir_all(kdd.join("LICENSES")).unwrap();
    std::fs::write(kdd.join("LICENSE.txt"), "KDDockWidgets is GPL-2.0-only or GPL-3.0-only\n").unwrap();
    for t in ["GPL-2.0-only", "GPL-3.0-only"] {
        std::fs::write(kdd.join(format!("LICENSES/{t}.txt")), format!("{t}\n")).unwrap();
    }
    let dest = d.path().join("licenses");
    alforge::licences::assemble(&sbom, r.path(), &c, &vendored, Some(&kdd), &common, &dest).unwrap();

    let read = |f: &str| std::fs::read_to_string(dest.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
    assert_eq!(read("components/veredas-portal/LICENSE-MIT"), "portal's MIT text\n");
    assert_eq!(read("components/veredas-portal/LICENSE-APACHE"), "portal's Apache text\n");
    assert_eq!(read("components/garfo/LICENSE"), "the fork's MIT text\n");
    assert_eq!(read(&format!("crates/itoa-{itoa}/LICENSE-MIT")), "itoa's MIT text\n");
    assert_eq!(read("qt/LGPL-3"), "the LGPL-3 text\n");
    assert_eq!(read("kddockwidgets/LICENSE.txt"), "KDDockWidgets is GPL-2.0-only or GPL-3.0-only\n");
    assert_eq!(read("kddockwidgets/GPL-3.0-only.txt"), "GPL-3.0-only\n");
    let third = read("THIRD-PARTY.md");
    assert!(third.contains(&format!("Release 2.0.0, cut from commit {c}.")), "{third}");
    assert!(third.contains("Oficina (`veredas-oficina`) is GPL-3.0-or-later"), "{third}");
    for row in [
        "| veredas-portal | 2.0.3 | MIT OR Apache-2.0 | this repository, crates/veredas-portal | components/veredas-portal/LICENSE-APACHE, components/veredas-portal/LICENSE-MIT |".to_string(),
        "| garfo | 0.3.0 | MIT | this repository, forks/garfo | components/garfo/LICENSE |".to_string(),
        format!("| itoa | {itoa} | MIT OR Apache-2.0 | pkg:cargo/itoa@{itoa} | crates/itoa-{itoa}/LICENSE-MIT |"),
        "| qt | 6.12.0 | LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only | https://download.qt.io/official_releases/qt/6.12/6.12.0/single/qt-everywhere-src-6.12.0.tar.xz | qt/LGPL-3, qt/GPL-2, qt/GPL-3 |".to_string(),
        "| kddockwidgets | 2.5.1 | GPL-2.0-only OR GPL-3.0-only | https://github.com/KDAB/KDDockWidgets/releases/tag/v2.5.1 | kddockwidgets/LICENSE.txt, kddockwidgets/GPL-2.0-only.txt, kddockwidgets/GPL-3.0-only.txt |".to_string(),
    ] {
        assert!(third.lines().any(|l| l == row), "{row}\n{third}");
    }
    // Every entry of the list has its row.
    assert_eq!(third.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| Package")).count(), doc["components"].as_array().unwrap().len());
    // Qt's texts missing where they are looked for: refused, naming the file.
    let none = d.path().join("none");
    let e = alforge::licences::assemble(&sbom, r.path(), &c, &vendored, Some(&kdd), &none, &d.path().join("again")).unwrap_err();
    assert!(e.to_string().contains(&format!("qt's licence text {}", none.join("LGPL-3").display())), "{e}");
}
