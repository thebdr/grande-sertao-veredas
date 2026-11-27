//! Oficina, the component (C-250): what cargo builds of it, with and without its window; that the
//! suite's default build carries the window through Oficina alone and the model needs no Qt; that
//! its members are core alone, without core's plant-facing
//! layers; that the bridge is one private seam; that the window that ships carries no test server;
//! and the window itself, opened and read through the driver.

mod common;

use std::collections::BTreeSet;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::Shell;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_manifest() -> PathBuf {
    crate_dir().join("../../Cargo.toml")
}

fn cargo(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO")).args(args).output().expect("cargo runs");
    assert!(out.status.success(), "cargo {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("UTF-8")
}

/// cargo's description of the workspace's own packages (`--no-deps`: its members, every one).
fn workspace() -> Value {
    let text = cargo(&["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path", workspace_manifest().to_str().expect("a UTF-8 path")]);
    serde_json::from_str(&text).expect("cargo's JSON")
}

/// The names of the workspace's members, as cargo states them, whatever they are called.
fn member_names() -> BTreeSet<String> {
    let meta = workspace();
    let ids: BTreeSet<&str> = meta["workspace_members"].as_array().expect("workspace_members").iter().map(|id| id.as_str().expect("an id")).collect();
    let names: BTreeSet<String> = meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter(|p| ids.contains(p["id"].as_str().expect("an id")))
        .map(|p| p["name"].as_str().expect("a name").to_string())
        .collect();
    assert_eq!(names.len(), ids.len(), "one name per member");
    names
}

fn package() -> Value {
    let meta = workspace();
    let manifest = crate_dir().join("Cargo.toml").canonicalize().expect("the manifest");
    meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().expect("a path")).canonicalize().expect("a manifest") == manifest)
        .expect("veredas-oficina among the workspace's packages")
        .clone()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().expect("an array").iter().map(|s| s.as_str().expect("a string").to_string()).collect()
}

/// The packages `cargo tree` lists for `args` on every target cargo knows (Windows' dependencies
/// included, whichever host runs the test), by name, once each.
fn tree(args: &[&str]) -> BTreeSet<String> {
    let manifest = workspace_manifest();
    let mut all = vec!["tree", "--offline", "--locked", "--target", "all", "--prefix", "none", "--format", "{p}", "--manifest-path", manifest.to_str().expect("a UTF-8 path")];
    all.extend_from_slice(args);
    cargo(&all).lines().filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect()
}

const QT: [&str; 4] = ["cxx-qt", "cxx-qt-lib", "cxx-qt-build", "cxx"];

#[test]
fn oficina_is_a_library_and_one_executable_its_window_behind_the_shell_feature() {
    let p = package();
    assert_eq!(p["name"], "veredas-oficina");
    assert_eq!(p["version"], "2.0.0-alpha.1");
    // GPL version 3 or later (Q35), the licence's text beside the manifest, unchanged.
    assert_eq!(p["license"], "GPL-3.0-or-later");
    assert!(p["license_file"].is_null(), "a licence file named: {}", p["license_file"]);
    // Unchanged: byte for byte the text the Free Software Foundation publishes (Debian's
    // `/usr/share/common-licenses/GPL-3`, its SHA-256 below), its lines ended LF everywhere
    // (`.gitattributes`); the length and the title say which text it is when it differs. A file of
    // its own: a link to a copy elsewhere (a system's, automake's) is no text a clone carries.
    let kind = std::fs::symlink_metadata(crate_dir().join("COPYING")).expect("COPYING beside the manifest").file_type();
    assert!(kind.is_file(), "COPYING a file of its own, not a link: {kind:?}");
    let copying = std::fs::read(crate_dir().join("COPYING")).expect("COPYING beside the manifest");
    assert_eq!(copying.len(), 35_149, "the GPL version 3 text's length");
    assert!(copying.starts_with(b"                    GNU GENERAL PUBLIC LICENSE\n                       Version 3, 29 June 2007\n"), "the GPL version 3's title");
    let sha256 = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    const GPL3: &str = "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986";
    assert_eq!(sha256(&copying), GPL3, "the GPL version 3 text, unchanged");
    // And in what a commit holds (git's index): a regular file, not a link, holding the same text,
    // so a clone and a release built from the tree carry it.
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(crate_dir()).args(args).output().expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    };
    let staged = String::from_utf8(git(&["ls-files", "--stage", "--", "COPYING"])).expect("UTF-8");
    assert!(staged.starts_with("100644 ") && staged.trim_end().ends_with("\tCOPYING") && staged.lines().count() == 1, "COPYING kept by git as a regular file: {staged:?}");
    assert_eq!(sha256(&git(&["cat-file", "blob", ":./COPYING"])), GPL3, "the GPL version 3 text as git keeps it");
    let mut targets: Vec<(String, String, Vec<String>)> = p["targets"]
        .as_array()
        .expect("targets")
        .iter()
        .map(|t| {
            let kinds = strings(&t["kind"]).join(",");
            let features = t.get("required-features").map(strings).unwrap_or_default();
            (kinds, t["name"].as_str().expect("a name").to_string(), features)
        })
        .collect();
    targets.sort();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            ("bin".to_string(), "veredas-oficina".to_string(), s(&["shell"])),
            ("custom-build".to_string(), "build-script-build".to_string(), vec![]),
            ("lib".to_string(), "veredas_oficina".to_string(), vec![]),
            ("test".to_string(), "component".to_string(), s(&["driver"])),
            ("test".to_string(), "tree".to_string(), s(&["driver"])),
            ("test".to_string(), "tree_model".to_string(), vec![]),
        ]
    );
    let features = p["features"].as_object().expect("features");
    let feature = |name: &str| strings(&features[name]);
    assert_eq!(features.len(), 3, "{features:?}");
    assert_eq!(feature("default"), s(&["shell"]), "the window built by default (Q370)");
    assert_eq!(feature("shell"), s(&["dep:cxx", "dep:cxx-qt", "dep:cxx-qt-lib", "dep:cxx-qt-build"]));
    assert_eq!(feature("driver"), s(&["shell"]));
}

#[test]
fn the_suite_s_default_build_carries_the_window_through_oficina_alone_and_the_model_needs_no_qt() {
    // The suite's default build carries the window (Q370), Oficina's own default build too.
    let everyone = tree(&["--workspace", "-e", "normal,build,dev"]);
    let window = tree(&["-p", "veredas-oficina", "-e", "normal,build"]);
    for qt in QT {
        assert!(everyone.contains(qt), "{qt} missing from the suite's default build: {everyone:?}");
        assert!(window.contains(qt), "{qt} missing from Oficina's default build: {window:?}");
    }
    // Through Oficina alone: every member above a crate of the bridge is Oficina, the members being
    // those cargo names, a tool's of any name among them.
    let members = member_names();
    assert!(members.contains("veredas-oficina") && members.contains("leis"), "{members:?}");
    for qt in QT {
        let above = tree(&["--workspace", "-e", "normal,build,dev", "--invert", qt]);
        let reaching: Vec<&String> = above.iter().filter(|p| members.contains(*p)).collect();
        assert_eq!(reaching, [&"veredas-oficina".to_string()], "{qt} reached by another member: {above:?}");
    }
    // The model alone: no crate of the bridge, dev edges included.
    let model = tree(&["-p", "veredas-oficina", "--no-default-features", "-e", "normal,build,dev"]);
    assert!(model.contains("veredas-core"), "{model:?}");
    for qt in QT {
        assert!(!model.contains(qt), "{qt} in Oficina's model alone");
    }
}

#[test]
fn its_members_are_core_alone() {
    let p = package();
    let members: BTreeSet<String> = p["dependencies"]
        .as_array()
        .expect("dependencies")
        .iter()
        .filter(|d| d["path"].is_string() && d["kind"].as_str() != Some("dev"))
        .map(|d| d["name"].as_str().expect("a name").to_string())
        .collect();
    assert_eq!(members, BTreeSet::from(["veredas-core".to_string()]));
    let names: BTreeSet<String> = p["dependencies"].as_array().expect("dependencies").iter().map(|d| d["name"].as_str().expect("a name").to_string()).collect();
    assert!(!names.iter().any(|n| n.starts_with("veredas-") && n != "veredas-core"), "{names:?}");
}

/// The model and its tests built and linked with the default features off, as a machine without Qt would build
/// them: a fresh build folder (every build script run again), the home folder an empty one (no
/// `~/.local/opt/Qt` to be found), no folder holding a qmake on `PATH`, no `LIBRARY_PATH` or
/// `LD_LIBRARY_PATH`, `QMAKE` naming no qmake and `OFICINA_DEPS` unset; cargo's and rustup's own
/// homes kept, and rustc named, so the toolchain is still found. On Linux the build runs inside
/// bubblewrap with Qt's installation (the one `QMAKE` names, and `~/.local/opt/Qt`) and the C++
/// libraries' folder each under an empty folder, so no path to them, however written, reaches
/// them; a look inside first finds each empty. Elsewhere (Windows) the environment alone.
#[test]
fn the_model_and_its_tests_build_with_no_qt() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-qt");
    let _ = std::fs::remove_dir_all(&scratch);
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).expect("an empty home");
    let user_home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).expect("a home"));
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| user_home.join(".cargo"));
    let rustup_home = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).unwrap_or_else(|| user_home.join(".rustup"));
    let rustc = Path::new(env!("CARGO")).with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    let qmake_names = ["qmake", "qmake6", "qmake.exe", "qmake6.exe", "qmake.bat"];
    let path: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|d| !qmake_names.iter().any(|q| d.join(q).exists())).collect();
    let manifest = workspace_manifest();
    let mut build = if cfg!(target_os = "linux") {
        let qmake = PathBuf::from(std::env::var_os("QMAKE").expect("QMAKE naming Qt's qmake, in cargo's environment"));
        let prefix = qmake.parent().and_then(Path::parent).expect("Qt's installation").to_path_buf();
        let deps = std::env::var_os("OFICINA_DEPS").map(PathBuf::from).unwrap_or_else(|| user_home.join(".local").join("opt").join("oficina-deps"));
        let mut hidden = vec![prefix, deps];
        // The default places too, wherever `QMAKE` and `OFICINA_DEPS` point.
        for default in [user_home.join(".local").join("opt").join("Qt"), user_home.join(".local").join("opt").join("oficina-deps")] {
            if default.is_dir() && !hidden.contains(&default) {
                hidden.push(default);
            }
        }
        for d in &hidden {
            assert!(d.is_dir(), "{} to hide", d.display());
        }
        // A folder inside another hidden one goes with it (an empty folder over the outer).
        let outer: Vec<PathBuf> = hidden.iter().filter(|d| !hidden.iter().any(|o| o != *d && d.starts_with(o))).cloned().collect();
        let hidden = outer;
        let sandbox = || {
            let mut c = Command::new("bwrap");
            c.args(["--dev-bind", "/", "/", "--die-with-parent"]);
            for d in &hidden {
                c.arg("--tmpfs").arg(d);
            }
            c.arg("--");
            c
        };
        let look = sandbox().arg("find").args(&hidden).args(["-mindepth", "1"]).output().expect("bubblewrap (bwrap) runs the no-Qt build on Linux");
        assert!(look.status.success(), "the look inside: {}", String::from_utf8_lossy(&look.stderr));
        assert_eq!(String::from_utf8_lossy(&look.stdout), "", "Qt and the C++ libraries hidden inside");
        let mut c = sandbox();
        c.arg(env!("CARGO"));
        c
    } else {
        Command::new(env!("CARGO"))
    };
    let out = build
        .args(["build", "--offline", "--locked", "-p", "veredas-oficina", "--no-default-features", "--lib", "--tests", "--manifest-path", manifest.to_str().expect("a UTF-8 path")])
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CARGO_HOME", &cargo_home)
        .env("RUSTUP_HOME", &rustup_home)
        .env("RUSTC", &rustc)
        .env("PATH", std::env::join_paths(path).expect("a PATH"))
        .env("QMAKE", home.join("no-such-qmake"))
        .env_remove("OFICINA_DEPS")
        .env_remove("LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .output()
        .expect("cargo runs");
    assert!(out.status.success(), "the model's build with no Qt: {}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&scratch);
}

/// Core without its default features (C-073): its header reader, and no crate of a plant's stack in
/// Oficina's own graph, the window's included.
#[test]
fn its_own_graph_holds_no_plant_stack() {
    const PLANT: [&str; 7] = ["tokio", "async-opcua-client", "async-opcua-types", "socket2", "regex", "chrono", "duckdb"];
    for features in [&[][..], &["--no-default-features"][..], &["--features", "driver"][..]] {
        let mut args = vec!["-p", "veredas-oficina", "-e", "normal,build"];
        args.extend_from_slice(features);
        let graph = tree(&args);
        assert!(graph.contains("veredas-core"), "{graph:?}");
        for crate_ in PLANT {
            assert!(!graph.contains(crate_), "{crate_} in Oficina's graph with {features:?}: {graph:?}");
        }
    }
}

/// One private seam: the bridge (cxx and cxx-qt) is named only in the files under `src/shell/`;
/// the library declares that module without `pub`, and no file outside the seam names it but the
/// library's one `pub use shell::run;`; the seam re-exports nothing (`pub use`) and declares its
/// modules without `pub`, so the library's public items are the model's and `run`. A reading of the
/// sources, against accidents: a name built by a macro is outside it.
#[test]
fn the_bridge_is_one_private_seam() {
    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("a folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let src = crate_dir().join("src");
    let seam = src.join("shell");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut bridged = 0;
    // Any identifier beginning `cxx` (cxx, cxx_qt, cxx_qt_lib…), its comments set aside.
    fn names_bridge(text: &str) -> bool {
        let code: String = text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        code.match_indices("cxx").any(|(i, _)| !code[..i].chars().next_back().is_some_and(ident))
    }
    for file in &files {
        let text = std::fs::read_to_string(file).expect("a source");
        let names_bridge = names_bridge(&text);
        if file.starts_with(&seam) {
            bridged += usize::from(text.contains("#[cxx_qt::bridge]"));
            for line in text.lines().map(str::trim) {
                assert!(!(line.starts_with("pub use") || line.starts_with("pub(crate) use") || line.starts_with("pub(super) use")), "{}: the seam re-exports: {line}", file.display());
            }
        } else {
            assert!(!names_bridge, "{}: the bridge named outside the seam", file.display());
            let naming: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.starts_with("//") && l.contains("shell::")).collect();
            let allowed: &[&str] = if *file == src.join("lib.rs") { &["pub use shell::run;"] } else { &[] };
            assert_eq!(naming, allowed, "{}: the seam named outside it", file.display());
        }
    }
    assert_eq!(bridged, 2, "the seam's two bridges, the start and the tree");
    let lib = std::fs::read_to_string(src.join("lib.rs")).expect("lib.rs");
    let declared: Vec<&str> = lib.lines().map(str::trim).filter(|l| l.contains("mod shell")).collect();
    assert_eq!(declared, ["mod shell;"], "the seam declared without `pub`");
    let seam_mod = std::fs::read_to_string(seam.join("mod.rs")).expect("shell/mod.rs");
    for line in seam_mod.lines().map(str::trim).filter(|l| l.contains("mod ") && l.ends_with(';')) {
        assert!(!line.starts_with("pub"), "a module of the seam made public: {line}");
    }
}

/// Whether `bytes` hold `name`.
fn holds(bytes: &[u8], name: &[u8]) -> bool {
    bytes.windows(name.len()).any(|w| w == name)
}

/// What the build script reads of its environment, by name: Qt's qmake, the C++ libraries' place,
/// the home folder for its default, and the target's system; `driver` bound once, to the
/// compiler's word on the feature, and named elsewhere only by the blocks it opens. A reading,
/// against accidents: a name computed at run time is outside it.
#[test]
fn the_build_script_reads_its_environment_by_name_and_the_driver_from_the_compiler() {
    let text = std::fs::read_to_string(crate_dir().join("build.rs")).expect("build.rs");
    let code: String = text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let allowed = ["QMAKE", "OFICINA_DEPS", "HOME", "USERPROFILE", "CARGO_CFG_TARGET_OS"];
    for call in ["var(", "var_os("] {
        for (at, _) in code.match_indices(call) {
            let rest = &code[at + call.len()..];
            let name = rest.strip_prefix('"').and_then(|r| r.split('"').next()).unwrap_or_else(|| panic!("an environment variable not named by a literal: {}", &code[at..(at + 60).min(code.len())]));
            assert!(allowed.contains(&name), "the build script reads {name}");
        }
    }
    for refused in ["vars()", "vars_os()", "env!(", "option_env!("] {
        assert!(!code.contains(refused), "the build script reads its environment through {refused}");
    }
    let binding = "let driver = cfg!(feature = \"driver\");";
    assert_eq!(code.matches(binding).count(), 1, "driver bound to the compiler's word");
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let words = code.match_indices("driver").filter(|(i, _)| !word(code[..*i].chars().next_back()) && !word(code[i + 6..].chars().next())).count();
    assert_eq!(words, 2 + code.matches("if driver {").count(), "driver named only in its binding and the blocks it opens");
}

/// What ships carries no test server: the window built as the release tool builds it (`--release
/// --locked --offline -p veredas-oficina --bins --features veredas-oficina/shell`, the default
/// features with them: the window without the driver), into a build folder of its own and with the
/// environment set against it (cargo's variable for the driver feature, the driver's port), holds
/// no name of Spix's or AnyRPC's, where the driver's executable does; started with a port named,
/// it stays up and nothing answers on that port. The executable a former run left is removed
/// first, so the one read is this build's. The first build of that folder takes minutes.
#[test]
fn the_window_that_ships_carries_no_test_server() {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shipped");
    let exe = target.join("release").join(format!("veredas-oficina{}", std::env::consts::EXE_SUFFIX));
    match std::fs::remove_file(&exe) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("a former run's executable not removed: {e}"),
    }
    let manifest = workspace_manifest();
    let out = Command::new(env!("CARGO"))
        .args(["build", "--release", "--offline", "--locked", "-p", "veredas-oficina", "--bins", "--features", "veredas-oficina/shell", "--manifest-path", manifest.to_str().expect("a UTF-8 path")])
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_FEATURE_DRIVER", "1")
        .env("OFICINA_SPIX_PORT", "1")
        .output()
        .expect("cargo runs");
    assert!(out.status.success(), "the window as it ships: {}", String::from_utf8_lossy(&out.stderr));
    let shipped = std::fs::read(&exe).expect("the executable that ships");
    let driven = std::fs::read(env!("CARGO_BIN_EXE_veredas-oficina")).expect("the driver's executable");
    for name in [&b"spix"[..], b"Spix", b"AnyRpc", b"OFICINA_SPIX_PORT"] {
        assert!(!holds(&shipped, name), "{} in what ships", String::from_utf8_lossy(name));
    }
    assert!(holds(&driven, b"spix") && holds(&driven, b"OFICINA_SPIX_PORT"), "the reading finds Spix where it is");

    let port = TcpListener::bind("127.0.0.1:0").expect("a port").local_addr().expect("its address").port();
    let platform = std::env::var("OFICINA_TEST_PLATFORM").unwrap_or_else(|_| "offscreen".to_string());
    let mut child = Command::new(&exe)
        .env("QT_QPA_PLATFORM", platform)
        .env("OFICINA_SPIX_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the window that ships starts");
    std::thread::sleep(Duration::from_secs(3));
    let running = child.try_wait().expect("its state").is_none();
    let answered = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(1)).is_ok();
    let _ = child.kill();
    let _ = child.wait();
    assert!(running, "the window that ships stayed up");
    assert!(!answered, "something answered on port {port}");
}

/// The whole window as it draws now, through Spix's screenshot (a PNG it writes when it gets to
/// the request), read once the file is whole.
fn shot(shell: &Shell, name: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shots");
    std::fs::create_dir_all(&dir).expect("a folder for the screenshots");
    let file = dir.join(format!("{}-{name}.png", std::process::id()));
    let _ = std::fs::remove_file(&file);
    shell.screenshot("oficina", &file);
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(bytes) = std::fs::read(&file) {
            if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.ends_with(b"IEND\xaeB`\x82") {
                let _ = std::fs::remove_file(&file);
                return bytes;
            }
        }
        assert!(std::time::Instant::now() < until, "no screenshot {name} within 10 s\nthe shell's log:\n{}", shell.log_text());
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_window_opens_as_oficina_with_the_project_pane_docked() {
    let shell = Shell::start(&[], &[]);
    assert_eq!(shell.property("oficina", "title"), "Oficina");
    assert_eq!(shell.property("oficina", "visible"), "true");
    assert!(shell.visible("oficina/projectPane"), "the project pane is drawn");
    // Docked: its dock open and not floating, the pane inside the docking area (its parents reach
    // it) and drawn in the main window.
    assert_eq!(shell.property("oficina/area", "uniqueName"), "workbench");
    shell.invoke("oficina/area", "describeDocks", &[]);
    shell.wait_for("oficina/area", "docks", r#"[["project",true,false,true,true]]"#);
    // In its dock: closed, the dock takes the pane with it; opened again, the pane is back.
    let shown_within = |shown: bool, what: &str| {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while shell.visible("oficina/projectPane") != shown {
            assert!(std::time::Instant::now() < until, "{what}\nthe shell's log:\n{}", shell.log_text());
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // And by what the window draws, not only by Qt's flag: its pixels change when the dock closes,
    // and are the same again when it opens.
    let open = shot(&shell, "open");
    shell.invoke("oficina/area", "closeDock", &[common::X::Str("project".into())]);
    shown_within(false, "the pane still shown with its dock closed");
    let closed = shot(&shell, "closed");
    shell.invoke("oficina/area", "openDock", &[common::X::Str("project".into())]);
    shown_within(true, "the pane not shown again with its dock opened");
    let reopened = shot(&shell, "reopened");
    assert!(open != closed, "the window draws the same with the project's dock open and closed");
    assert!(open == reopened, "the window draws otherwise with the project's dock opened again");
    shell.invoke("oficina/area", "describeDocks", &[]);
    shell.wait_for("oficina/area", "docks", r#"[["project",true,false,true,true]]"#);
    // No folder named: the tree is empty and says nothing went wrong.
    assert_eq!(shell.property("oficina/projectError", "text"), "");
    assert_eq!(shell.rows(), Vec::<String>::new());
    assert_eq!(shell.errors(), Vec::<String>::new());
}
