//! Each build of the cut traced (docs/questions.md Q448, ruled 2026-11-26 "Trace each build"): the
//! builder runs every component's `cargo build` under `strace -f` ([`STRACE`]), which logs each
//! file a process of the build opens, as the kernel's own path of what it opened, and each program
//! it starts. [`reads`] reads that log back: every file a process other than cargo itself opened to
//! read, and every program started, with the process's program. [`outside`] judges them against
//! what the component's version names (C-290's inputs), as the cut judges cargo's dep-info, so a
//! build script's undeclared read, a header the C compiler includes and a file a proc-macro reads
//! are judged too.
//!
//! The reads of the cargo the builder starts (and of its threads) are its resolution, pinned by
//! `--locked` (every member's manifest to load the workspace, the lock, the vendored sums), and are
//! left out; a cargo a build script starts (`$CARGO metadata`, as the `cargo_metadata` crate runs
//! it) is a program of the build, judged as any other. What lies outside the tree and the
//! vendored sources (the toolchain, the system's headers and libraries, Qt) is the builder's, not a
//! version's. A file read without being opened (a kernel ring's, `io_uring`) is in no trace, and a
//! file whose existence alone a build looks at (`stat`) is not judged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::inputs::{self, Inputs, Packages};

/// The tracer the builder runs each build under, its log the word after it: every process followed,
/// the successful calls alone, each file descriptor shown as the path the kernel opened, the calls
/// that open a file, start a program, change a folder or start a process.
pub const STRACE: &str = "strace -f -qq -z --seccomp-bpf -y -s 65535 -e trace=open,openat,openat2,creat,execve,execveat,chdir,fchdir,clone,clone3,fork,vfork -o";

/// A file a traced process opened to read, or a program it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The program the process ran (its last `execve`'s path).
    pub program: String,
    /// The path the kernel opened, or the program started, absolute.
    pub path: String,
}

/// A string strace wrote, unescaped (`\\`, `\"`, `\n`, `\t`, `\r`, `\v`, `\f`, `\NNN` in octal,
/// `\xNN`), to the bytes it stands for, read as UTF-8 (a byte that is not shown as its escape).
fn unescaped(text: &str) -> String {
    let b = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 == b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0b),
            b'f' => out.push(0x0c),
            b'x' => {
                let hex: String = b[i..].iter().take(2).take_while(|h| h.is_ascii_hexdigit()).map(|h| *h as char).collect();
                i += hex.len();
                out.push(u8::from_str_radix(&hex, 16).unwrap_or(0));
            }
            b'0'..=b'7' => {
                let mut v = u32::from(c - b'0');
                let mut n = 1;
                while n < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                    v = v * 8 + u32::from(b[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push((v & 0xff) as u8);
            }
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The first string literal in `args` (`"…"`, escapes kept), unescaped, and what follows it.
fn first_string(args: &str) -> Option<(String, &str)> {
    let start = args.find('"')?;
    let b = args.as_bytes();
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some((unescaped(&args[start + 1..i]), &args[i + 1..])),
            _ => i += 1,
        }
    }
    None
}

/// The path a decorated file descriptor names (`3</build/src/a.rs>` as `-y` writes it, its `<` and
/// `>` escaped inside), unescaped.
fn fd_path(text: &str) -> Option<String> {
    let open = text.find('<')?;
    let close = text.rfind('>')?;
    (close > open).then(|| unescaped(&text[open + 1..close]))
}

/// `path` resolved against the folder `cwd` (an absolute path stays as it is), its `.` and `..`
/// parts as the kernel reads them.
fn resolved(cwd: &str, path: &str) -> String {
    let joined = if path.starts_with('/') { path.to_string() } else { format!("{cwd}/{path}") };
    inputs::landed(&joined).display().to_string()
}

/// Whether a program is cargo (rustup's proxy or the toolchain's own).
fn is_cargo(program: &str) -> bool {
    Path::new(program).file_name().is_some_and(|n| n == "cargo")
}

/// Every file a traced build's process other than the builder's cargo opened to read (not one
/// opened to write alone, nor an `O_PATH` handle), and every program started, from strace's log
/// `trace` of a build started in `cwd`; a call strace cut in two (`<unfinished ...>`,
/// `<... resumed>`) read whole. The builder's cargo is the process traced first and what it starts
/// by `clone` or `fork` (its threads) until one of them runs another program: a cargo a build
/// script runs is not it.
pub fn reads(trace: &str, cwd: &str) -> Vec<Read> {
    // strace may print a process's first lines before the line of its parent's `clone` that
    // returns its id (a busy machine runs the child first: 45 times in a real trace of Oficina's
    // build): a process's lines are held until that line is read, then read in their own order,
    // so a child inherits its parent's program, folder and standing before its own lines apply.
    let mut state = State { cwd: cwd.to_string(), ..State::default() };
    let mut known: BTreeSet<String> = BTreeSet::new();
    let mut held: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut first = true;
    for raw in trace.lines() {
        let Some((pid, _)) = raw.split_once(' ') else { continue };
        if first {
            state.folder.insert(pid.to_string(), cwd.to_string());
            state.builders.insert(pid.to_string(), true);
            known.insert(pid.to_string());
            first = false;
        }
        if !known.contains(pid) {
            held.entry(pid.to_string()).or_insert_with(|| {
                order.push(pid.to_string());
                Vec::new()
            }).push(raw);
            continue;
        }
        state.run(vec![raw], &mut known, &mut held);
    }
    // A process no line of the log says the start of: read as it came, inheriting nothing.
    for pid in order {
        if let Some(lines) = held.remove(&pid) {
            known.insert(pid);
            state.run(lines, &mut known, &mut held);
        }
    }
    state.out
}

/// What [`reads`] knows of each process as it reads the log: its program, its folder, whether it
/// is the builder's cargo, a call cut in two waiting for its second half; and the reads so far.
#[derive(Default)]
struct State {
    cwd: String,
    program: BTreeMap<String, String>,
    folder: BTreeMap<String, String>,
    pending: BTreeMap<String, String>,
    builders: BTreeMap<String, bool>,
    out: Vec<Read>,
}

impl State {
    /// `lines` read in order, each child a `clone` starts read at once after it, with the lines
    /// held for it, before the rest.
    fn run<'a>(&mut self, lines: Vec<&'a str>, known: &mut BTreeSet<String>, held: &mut BTreeMap<String, Vec<&'a str>>) {
        let mut queue: std::collections::VecDeque<&'a str> = lines.into();
        while let Some(line) = queue.pop_front() {
            if let Some(child) = self.step(line) {
                known.insert(child.clone());
                for l in held.remove(&child).unwrap_or_default().into_iter().rev() {
                    queue.push_front(l);
                }
            }
        }
    }

    /// One line of the log read: its reads added, its process's state moved; the id of the
    /// process it starts, if it starts one.
    fn step(&mut self, raw: &str) -> Option<String> {
        let (pid, rest) = raw.split_once(' ')?;
        let pid = pid.to_string();
        let rest = rest.trim_start();
        if rest.starts_with("---") || rest.starts_with("+++") {
            return None;
        }
        // A call cut in two by another process's: its two halves joined.
        let line: String = if let Some(head) = rest.strip_suffix("<unfinished ...>") {
            self.pending.insert(pid.clone(), head.to_string());
            return None;
        } else if let Some(tail) = rest.strip_prefix("<... ") {
            let (_, after) = tail.split_once(" resumed>")?;
            format!("{}{after}", self.pending.remove(&pid).unwrap_or_default())
        } else {
            rest.to_string()
        };
        let paren = line.find('(')?;
        let name = &line[..paren];
        let eq = line.rfind(" = ")?;
        let args = &line[paren + 1..eq];
        let ret = line[eq + 3..].trim();
        let here = self.folder.get(&pid).cloned().unwrap_or_else(|| self.cwd.clone());
        let running = self.program.get(&pid).cloned().unwrap_or_default();
        let builder = self.builders.get(&pid).copied().unwrap_or(false);
        match name {
            "clone" | "clone3" | "fork" | "vfork" => {
                let child = ret.split_whitespace().next().filter(|c| c.bytes().all(|b| b.is_ascii_digit()))?.to_string();
                self.program.insert(child.clone(), running);
                self.folder.insert(child.clone(), here);
                self.builders.insert(child.clone(), builder);
                return Some(child);
            }
            "execve" | "execveat" => {
                // `execveat(<fd>, "path", …)`: the path from that folder, as `-y` names it.
                let base = if name == "execveat" { args.split_once(',').and_then(|(fd, _)| fd_path(fd)).unwrap_or(here.clone()) } else { here.clone() };
                if let Some((path, _)) = first_string(args) {
                    let path = resolved(&base, &path);
                    self.program.insert(pid.clone(), path.clone());
                    // The builder's cargo stays it through rustup's proxy; a process that runs
                    // anything else, and all it starts, is the build's.
                    let still = builder && is_cargo(&path);
                    self.builders.insert(pid.clone(), still);
                    if !still {
                        self.out.push(Read { program: path.clone(), path });
                    }
                }
            }
            "chdir" => {
                if let Some((path, _)) = first_string(args) {
                    self.folder.insert(pid.clone(), resolved(&here, &path));
                }
            }
            "fchdir" => {
                if let Some(path) = fd_path(args) {
                    self.folder.insert(pid.clone(), path);
                }
            }
            "open" | "openat" | "openat2" => {
                let path = fd_path(ret)?;
                let flags = first_string(args).map(|(_, after)| after.to_string()).unwrap_or_default();
                let writes_only = flags.contains("O_WRONLY");
                if !(writes_only || flags.contains("O_PATH") || builder) {
                    self.out.push(Read { program: running, path });
                }
            }
            _ => {}
        }
        None
    }
}

/// What the reads of a traced build (`reads`) name that `inputs` does not: each file under `src`
/// (the tree's root in the builder) its version does not name, each folder of the build's output a
/// package of another build made (`src/target/release/build/<name>/…`), each vendored crate
/// (`vendor/<name>-<version>`) its build does not compile; each said with the program that read it.
/// The rest of the build's output is what cargo and rustc made of what was judged; what lies
/// outside the tree and the vendored sources is the builder's.
pub fn outside(reads: &[Read], src: &str, vendor: &str, inputs: &Inputs, packages: &Packages) -> Vec<String> {
    let built = PathBuf::from(src).join("target/release/build");
    let target = PathBuf::from(src).join("target");
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for r in reads {
        let landed = inputs::landed(&r.path);
        let by = Path::new(&r.program).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.program.clone());
        let refused = if let Ok(rest) = landed.strip_prefix(&built) {
            let mut parts = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
            match (parts.next(), parts.next()) {
                (None, _) => None,
                (Some(first), second) => (!(inputs::built_by(packages, &first, second.as_deref()) || second.is_none() && packages.names.contains(&first))).then(|| format!("target/release/build/{}", rest.display())),
            }
        } else if landed.starts_with(&target) {
            None
        } else if let Ok(rest) = landed.strip_prefix(vendor) {
            match rest.components().next() {
                None => None,
                Some(c) => (!packages.vendored.contains(&c.as_os_str().to_string_lossy().into_owned())).then(|| landed.display().to_string()),
            }
        } else if let Ok(rel) = landed.strip_prefix(src) {
            let rel = rel.display().to_string();
            (!inputs.covers(&rel)).then(|| if rel.is_empty() { "the tree's root".to_string() } else { rel })
        } else {
            None
        };
        if let Some(what) = refused {
            out.entry(what).or_insert(by);
        }
    }
    out.into_iter().map(|(what, by)| format!("{by} read {what}")).collect()
}
