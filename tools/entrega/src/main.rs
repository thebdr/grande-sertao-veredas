//! `entrega check [<manifest>...]`: read the suite release manifests (by default every
//! `releases/*.toml` of the repository the current folder is in) and check each against the
//! repository's history and against each other, printing one line per manifest.
//!
//! `entrega cut <release> [--trial] [--out <dir>]`: cut a suite release from HEAD of the
//! repository the current folder is in (docs/entrega-plan.md, "How a release is cut"): the whole
//! suite run first (a trial cut skips it and its manifest says so), the executables built in the
//! builder machine, the bundle packed under `target/entrega/<release>/` and run together, then the
//! manifest written into `releases/` (or `--out`).
//!
//! `entrega maquina <release>`: build the tester machine of a release already cut, from its bundle
//! under `target/entrega/<release>/`, into `veredas-<release>.ova` beside it
//! (docs/entrega-plan.md, "How the image is built").
//!
//! Exit codes: 0 when every manifest checks, the release is cut or the machine built; 2 when one is refused, the
//! reasons on stderr; 1 on a usage error.

use std::path::{Path, PathBuf};

use entrega::{git, manifest};

const USAGE: &str = "usage: entrega check [<manifest.toml>...]\n       entrega cut <release> [--trial] [--out <dir>]\n       entrega maquina <release>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => std::process::exit(check(&args[1..])),
        Some("cut") => std::process::exit(cut(&args[1..])),
        Some("maquina") if args.len() == 2 => std::process::exit(maquina(&args[1])),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

/// The repository the current folder is in, or the reason it is none.
fn repository() -> Result<PathBuf, String> {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match git::text(&here, &["rev-parse", "--show-toplevel"]) {
        Ok(Some(t)) => Ok(PathBuf::from(t.trim())),
        Ok(None) => Err(format!("{}: not inside a git repository", here.display())),
        Err(e) => Err(e.to_string()),
    }
}

fn maquina(release: &str) -> i32 {
    let root = match repository() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match entrega::maquina::from_release(&root, release, "x86_64-unknown-linux-gnu") {
        Ok(p) => {
            println!("tester machine of release {release}");
            println!("  ova  {}", p.ova.display());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

fn cut(args: &[String]) -> i32 {
    let mut release = None;
    let mut trial = false;
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--trial" => trial = true,
            "--out" => match it.next() {
                Some(d) => out = Some(PathBuf::from(d)),
                None => {
                    eprintln!("{USAGE}");
                    return 1;
                }
            },
            r if release.is_none() && !r.starts_with("--") => release = Some(r.to_string()),
            _ => {
                eprintln!("{USAGE}");
                return 1;
            }
        }
    }
    let Some(release) = release else {
        eprintln!("{USAGE}");
        return 1;
    };
    let Ok(release) = semver::Version::parse(&release) else {
        eprintln!("`{release}` is not SemVer");
        return 1;
    };
    let root = match repository() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let options = entrega::cut::Options {
        out: out.unwrap_or_else(|| root.join("releases")),
        work: root.join("target/entrega").join(release.to_string()),
        release,
        run_suite: !trial,
    };
    match entrega::cut::cut(&root, &options) {
        Ok(c) => {
            println!("release {} cut from {}", c.manifest.release, c.manifest.commit);
            println!("  manifest {}", c.manifest_path.display());
            println!("  bundle   {}", c.bundle.display());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

fn check(files: &[String]) -> i32 {
    let root = match repository() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // Every manifest that reads is judged, and every refusal said, whatever else was refused.
    let (all, refusals) = if files.is_empty() {
        manifest::load_dir(&root.join("releases"))
    } else {
        let mut out = Vec::new();
        let mut refusals = Vec::new();
        for f in files {
            let p = Path::new(f);
            match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| manifest::load(&t).map_err(|e| e.to_string())) {
                Ok(m) => out.push((p.to_path_buf(), m)),
                Err(e) => refusals.push(format!("{f}: {e}")),
            }
        }
        (out, refusals)
    };
    for r in &refusals {
        eprintln!("{r}");
    }
    let mut failed = !refusals.is_empty();
    for (p, m) in &all {
        match manifest::check(&root, m) {
            Ok(r) if r.is_empty() => println!(
                "{}: release {} at {}, {} components, checked",
                p.display(),
                m.release,
                &m.commit[..12],
                m.components.len()
            ),
            Ok(r) => {
                failed = true;
                for x in r {
                    eprintln!("{}: {x}", p.display());
                }
            }
            Err(e) => {
                failed = true;
                eprintln!("{}: {e}", p.display());
            }
        }
    }
    match manifest::check_together(&root, &all) {
        Ok(r) => {
            for x in &r {
                eprintln!("{x}");
            }
            failed |= !r.is_empty();
        }
        Err(e) => {
            eprintln!("{e}");
            failed = true;
        }
    }
    if failed { 2 } else { 0 }
}
