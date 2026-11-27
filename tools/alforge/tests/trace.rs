//! C-291, each build traced (docs/questions.md Q448, ruled 2026-11-26 "Trace each build"): strace's
//! log of a build read back as the files each process other than cargo opened to read and the
//! programs it started, each judged against what the component's version names. The lines are
//! strace's own forms, as a release build of Ábaco traced on the development machine wrote them
//! (strace 7.0, `alforge::trace::STRACE`); the cut's trial in the builder (`tests/cut.rs`) runs the
//! real tracer on every component.

use std::collections::BTreeMap;

use alforge::inputs::{Inputs, LEFT_OUT, Packages};
use alforge::trace::{self, Read};

const SRC: &str = "/build/src";
const VENDOR: &str = "/build/vendor";

/// A build's log: cargo started in the tree (rustup's proxy, then the toolchain's own), reading
/// every member's manifest; rustc, a build script and the C compiler it runs, each started by a
/// thread of cargo's or of their own.
fn log() -> String {
    [
        r#"100 execve("/opt/cargo/bin/cargo", ["cargo", "build", "--release"], 0x7ffd /* 20 vars */) = 0"#,
        r#"100 execve("/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/bin/cargo", ["cargo"], 0x7ffd /* 20 vars */) = 0"#,
        // Cargo's own reads: its resolution.
        r#"100 openat(AT_FDCWD</build/src>, "/build/src/crates/veredas-outro/Cargo.toml", O_RDONLY|O_CLOEXEC) = 3</build/src/crates/veredas-outro/Cargo.toml>"#,
        r#"100 openat(AT_FDCWD</build/src>, "/build/src/Cargo.lock", O_RDONLY|O_CLOEXEC) = 3</build/src/Cargo.lock>"#,
        // A thread of cargo's, and a process it starts by vfork, cut in two by another's call.
        r#"100 clone3({flags=CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD, child_tid=0x7e50, parent_tid=0x7e50, exit_signal=0, stack=0x7e50, stack_size=0x1ff100, tls=0x7e50} => {parent_tid=[101]}, 88) = 101"#,
        r#"101 clone3({flags=CLONE_VM|CLONE_VFORK|CLONE_CLEAR_SIGHAND, exit_signal=SIGCHLD, stack=0x7522, stack_size=0x9000}, 88 <unfinished ...>"#,
        r#"101 openat(AT_FDCWD</build/src>, "/build/src/crates/veredas-outro/src/lib.rs", O_RDONLY|O_CLOEXEC) = 9</build/src/crates/veredas-outro/src/lib.rs>"#,
        r#"101 <... clone3 resumed>) = 102"#,
        r#"102 execve("/opt/cargo/bin/rustc", ["rustc", "--crate-name", "veredas_portal"], 0x7ffd /* 30 vars */) = 0"#,
        r#"102 execve("/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/bin/rustc", ["rustc"], 0x7ffd /* 30 vars */) = 0"#,
        r#"102 openat(AT_FDCWD</build/src>, "/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/lib/librustc_driver.so", O_RDONLY|O_CLOEXEC) = 3</opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/lib/librustc_driver.so>"#,
        r#"102 openat(AT_FDCWD</build/src>, "crates/veredas-portal/src/main.rs", O_RDONLY|O_CLOEXEC) = 4</build/src/crates/veredas-portal/src/main.rs>"#,
        r#"102 openat(AT_FDCWD</build/src>, "crates/veredas-portal/src/../../../docs/fora.rs", O_RDONLY|O_CLOEXEC) = 4</build/src/docs/fora.rs>"#,
        r#"102 openat(AT_FDCWD</build/src>, "/build/src/target/release/build/veredas-core/0123456789abcdef/out/libveredas_core-0123456789abcdef.rmeta", O_RDONLY|O_CLOEXEC) = 5</build/src/target/release/build/veredas-core/0123456789abcdef/out/libveredas_core-0123456789abcdef.rmeta>"#,
        r#"102 openat(AT_FDCWD</build/src>, "/build/src/target/release/build/veredas-outro/fedcba9876543210/out/gerado.rs", O_RDONLY|O_CLOEXEC) = 5</build/src/target/release/build/veredas-outro/fedcba9876543210/out/gerado.rs>"#,
        r#"102 openat(AT_FDCWD</build/src>, "/build/vendor/serde-1.0.200/src/lib.rs", O_RDONLY|O_CLOEXEC) = 6</build/vendor/serde-1.0.200/src/lib.rs>"#,
        r#"102 openat(AT_FDCWD</build/src>, "/build/vendor/serde-1.0.199/src/lib.rs", O_RDONLY|O_CLOEXEC) = 6</build/vendor/serde-1.0.199/src/lib.rs>"#,
        // What it writes alone, and a handle that reads nothing.
        r#"102 openat(AT_FDCWD</build/src>, "/build/src/crates/veredas-outro/escrito.txt", O_WRONLY|O_CREAT|O_TRUNC|O_CLOEXEC, 0644) = 7</build/src/crates/veredas-outro/escrito.txt>"#,
        r#"102 openat(AT_FDCWD</build/src>, "/build/src/crates/veredas-outro", O_RDONLY|O_PATH|O_CLOEXEC) = 7</build/src/crates/veredas-outro>"#,
        // A name holding `<`, `>` and a newline, as `-y` writes it.
        r#"102 openat(AT_FDCWD</build/src>, "crates/veredas-portal/src/a<1>\nb.txt", O_RDONLY|O_CLOEXEC) = 8</build/src/crates/veredas-portal/src/a\0741\76\nb.txt>"#,
        // A build script, in its package's folder, reading a left-out folder and running a script
        // there through the shell, which opens it.
        r#"100 vfork() = 103"#,
        r#"103 chdir("/build/src/crates/veredas-portal") = 0"#,
        r#"103 execve("/build/src/target/release/build/veredas-portal/1111222233334444/out/build_script_build", ["build_script_build"], 0x7ffd /* 40 vars */) = 0"#,
        r#"103 openat(AT_FDCWD</build/src/crates/veredas-portal>, "tests", O_RDONLY|O_NONBLOCK|O_CLOEXEC|O_DIRECTORY) = 3</build/src/crates/veredas-portal/tests>"#,
        r#"103 fork() = 104"#,
        r#"104 chdir("gerador") = 0"#,
        r#"104 execve("/bin/sh", ["sh", "./gera.sh"], 0x7ffd /* 40 vars */) = 0"#,
        r#"104 openat(AT_FDCWD</build/src/crates/veredas-portal/gerador>, "./gera.sh", O_RDONLY|O_CLOEXEC) = 3</build/src/crates/veredas-portal/gerador/gera.sh>"#,
        r#"104 fchdir(3</build/src/docs>) = 0"#,
        r#"104 execve("./ferramenta", ["./ferramenta"], 0x7ffd /* 40 vars */) = 0"#,
        // The C compiler and the linker it runs: the system's headers are the builder's.
        r#"103 vfork() = 105"#,
        r#"105 execve("/usr/bin/cc", ["cc", "-c", "x.c"], 0x7ffd /* 40 vars */) = 0"#,
        r#"105 openat(AT_FDCWD</build/src/crates/veredas-portal>, "/usr/include/stdio.h", O_RDONLY|O_NOCTTY) = 3</usr/include/stdio.h>"#,
        r#"105 openat(AT_FDCWD</build/src/crates/veredas-portal>, "../veredas-outro/include/x.h", O_RDONLY|O_NOCTTY) = 3</build/src/crates/veredas-outro/include/x.h>"#,
        r#"105 openat2(AT_FDCWD</build/src/crates/veredas-portal>, "x.h", {flags=O_RDONLY|O_CLOEXEC, resolve=RESOLVE_BENEATH}, 24) = 3</build/src/crates/veredas-portal/x.h>"#,
        r#"105 +++ exited with 0 +++"#,
        r#"103 --- SIGCHLD {si_signo=SIGCHLD, si_code=CLD_EXITED, si_pid=105, si_uid=0, si_status=0} ---"#,
        // The build script running cargo itself (`$CARGO metadata`, as the `cargo_metadata` crate
        // does): a program of the build, its reads judged (C-291's round 23).
        r#"103 vfork() = 106"#,
        r#"106 execve("/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/bin/cargo", ["cargo", "metadata", "--no-deps"], 0x7ffd /* 40 vars */) = 0"#,
        r#"106 openat(AT_FDCWD</build/src/crates/veredas-portal>, "/build/src/crates/veredas-outro/Cargo.toml", O_RDONLY|O_CLOEXEC) = 3</build/src/crates/veredas-outro/Cargo.toml>"#,
    ]
    .join("\n")
        + "\n"
}

#[test]
fn a_traced_build_names_what_each_process_but_cargo_opened_to_read() {
    let reads = trace::reads(&log(), SRC);
    let rustc = "/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/bin/rustc";
    let script = "/build/src/target/release/build/veredas-portal/1111222233334444/out/build_script_build";
    let cargo = "/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/bin/cargo";
    let read = |program: &str, path: &str| Read { program: program.to_string(), path: path.to_string() };
    assert_eq!(
        reads,
        [
            // The proxy and the toolchain's rustc, each a program started.
            read("/opt/cargo/bin/rustc", "/opt/cargo/bin/rustc"),
            read(rustc, rustc),
            read(rustc, "/opt/rustup/toolchains/1.100.0-x86_64-unknown-linux-gnu/lib/librustc_driver.so"),
            read(rustc, "/build/src/crates/veredas-portal/src/main.rs"),
            read(rustc, "/build/src/docs/fora.rs"),
            read(rustc, "/build/src/target/release/build/veredas-core/0123456789abcdef/out/libveredas_core-0123456789abcdef.rmeta"),
            read(rustc, "/build/src/target/release/build/veredas-outro/fedcba9876543210/out/gerado.rs"),
            read(rustc, "/build/vendor/serde-1.0.200/src/lib.rs"),
            read(rustc, "/build/vendor/serde-1.0.199/src/lib.rs"),
            read(rustc, "/build/src/crates/veredas-portal/src/a<1>\nb.txt"),
            read(script, script),
            read(script, "/build/src/crates/veredas-portal/tests"),
            read("/bin/sh", "/bin/sh"),
            read("/bin/sh", "/build/src/crates/veredas-portal/gerador/gera.sh"),
            // Started from the folder `fchdir` moved it to.
            read("/build/src/docs/ferramenta", "/build/src/docs/ferramenta"),
            read("/usr/bin/cc", "/usr/bin/cc"),
            read("/usr/bin/cc", "/usr/include/stdio.h"),
            read("/usr/bin/cc", "/build/src/crates/veredas-outro/include/x.h"),
            read("/usr/bin/cc", "/build/src/crates/veredas-portal/x.h"),
            read(cargo, cargo),
            read(cargo, "/build/src/crates/veredas-outro/Cargo.toml"),
        ]
    );

    // Judged against portal's version: its folder but its tests, core's folder, serde 1.0.200; its
    // build compiles portal, core and serde.
    let inputs = Inputs(
        [("crates/veredas-portal/", "1"), ("crates/veredas-portal/tests/", LEFT_OUT), ("crates/veredas-core/", "2"), ("Cargo.lock: serde 1.0.200", "3")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    );
    let packages = Packages {
        names: ["veredas-portal", "veredas-core", "serde"].into_iter().map(String::from).collect(),
        vendored: ["serde-1.0.200".to_string()].into_iter().collect(),
    };
    // Each refused with the program that read it, in the order of what was read; portal's own
    // folder, core's build output, the system's headers and the toolchain pass.
    assert_eq!(
        trace::outside(&reads, SRC, VENDOR, &inputs, &packages),
        [
            "rustc read /build/vendor/serde-1.0.199/src/lib.rs",
            "cargo read crates/veredas-outro/Cargo.toml",
            "cc read crates/veredas-outro/include/x.h",
            "build_script_build read crates/veredas-portal/tests",
            "ferramenta read docs/ferramenta",
            "rustc read docs/fora.rs",
            "rustc read target/release/build/veredas-outro/fedcba9876543210/out/gerado.rs",
        ]
    );
    // With every one of them named and built, nothing is refused.
    let all_named = Inputs(
        [("crates/veredas-portal/", "1"), ("crates/veredas-core/", "2"), ("crates/veredas-outro/", "4"), ("docs/", "5"), ("Cargo.lock: serde 1.0.199", "6")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    );
    let all_built = Packages {
        names: ["veredas-portal", "veredas-core", "veredas-outro", "serde"].into_iter().map(String::from).collect(),
        vendored: ["serde-1.0.200".to_string(), "serde-1.0.199".to_string()].into_iter().collect(),
    };
    assert_eq!(trace::outside(&reads, SRC, VENDOR, &all_named, &all_built), Vec::<String>::new());
}

#[test]
fn the_tracer_follows_every_process_and_shows_each_file_as_the_kernel_opened_it() {
    // The command the builder runs each build under (its log's path follows it).
    for word in ["-f", "-z", "-y", "--seccomp-bpf", "open,openat,openat2,creat,execve,execveat,chdir,fchdir,clone,clone3,fork,vfork"] {
        assert!(trace::STRACE.split_whitespace().any(|w| w == word || w.ends_with(word)), "{word}");
    }
    assert!(trace::STRACE.ends_with(" -o"));
    // A log naming nothing reads as nothing.
    assert_eq!(trace::reads("", SRC), Vec::<Read>::new());
}

/// A real build traced on this machine as the builder traces it (C-291's round 23): a member's
/// build script runs cargo itself (`$CARGO metadata`, as the `cargo_metadata` crate does) and
/// writes what it read of the workspace into its executable. That cargo's reads are judged as any
/// program's, and refused against the version, which names them not; the builder's own cargo reads
/// every member's manifest unjudged.
#[test]
fn a_cargo_a_build_script_runs_is_judged_as_any_program() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("repo");
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").arg("-C").arg(&root).args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    std::fs::create_dir_all(&root).unwrap();
    git(&["init", "-q", "-b", "main"]);
    write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n");
    write("crates/veredas-a/Cargo.toml", "[package]\nname = \"veredas-a\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    write(
        "crates/veredas-a/build.rs",
        "fn main() {\n    let cargo = std::env::var(\"CARGO\").unwrap();\n    let manifest = format!(\"{}/../../Cargo.toml\", std::env::var(\"CARGO_MANIFEST_DIR\").unwrap());\n    \
         let out = std::process::Command::new(cargo).args([\"metadata\", \"--no-deps\", \"--offline\", \"--format-version\", \"1\", \"--manifest-path\", &manifest]).output().unwrap();\n    \
         let text = String::from_utf8(out.stdout).unwrap();\n    let n = text.matches(\"\\\"version\\\":\\\"\").count();\n    println!(\"cargo::rustc-env=MEMBERS={n}\");\n}\n",
    );
    write("crates/veredas-a/src/main.rs", "fn main() { println!(\"{}\", env!(\"MEMBERS\")); }\n");
    write("crates/veredas-b/Cargo.toml", "[package]\nname = \"veredas-b\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    write("crates/veredas-b/src/lib.rs", "\n");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(&cargo).args(["generate-lockfile", "--offline"]).current_dir(&root).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "a build script running cargo"]);
    let c = git(&["rev-parse", "HEAD"]);
    let held = alforge::manifest::components_at(&root, &c, &mut Vec::new()).unwrap();
    let a = &held["veredas-a"];
    let inputs = a.inputs.as_ref().unwrap();
    assert!(!inputs.0.contains_key("crates/veredas-b/") && !inputs.0.contains_key("crates/veredas-b/Cargo.toml"), "{inputs:?}");

    // Traced as the builder traces a build, its target folder its own.
    let log = d.path().join("a.trace");
    let mut words: Vec<String> = trace::STRACE.split_whitespace().map(String::from).collect();
    words.push(log.display().to_string());
    let out = std::process::Command::new(&words[0])
        .args(&words[1..])
        .args([&cargo, "build", "--offline", "--locked", "-p", "veredas-a@2.0.0", "--bins"])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(&log).unwrap();
    let src = root.canonicalize().unwrap().display().to_string();
    let reads = trace::reads(&text, &src);
    let refused = trace::outside(&reads, &src, &d.path().join("vendor").display().to_string(), inputs, &a.packages);
    // That cargo reads the root's manifest whole (the version names the parts cargo reads for the
    // component), lists the members' folder for its glob and reads the other member's manifest:
    // each refused, so the cut stops on such a build.
    assert_eq!(refused, ["cargo read Cargo.toml", "cargo read crates", "cargo read crates/veredas-b/Cargo.toml"], "{refused:?}");
}
