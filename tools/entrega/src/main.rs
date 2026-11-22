//! `entrega check [<manifest>...]`: read the suite release manifests (by default every
//! `releases/*.toml` of the repository the current folder is in) and check each against the
//! repository's history and against each other, printing one line per manifest.
//!
//! Exit codes: 0 when every manifest checks; 2 when one is refused, the reasons on stderr; 1 on a
//! usage error.

use std::path::{Path, PathBuf};

use entrega::{git, manifest};

const USAGE: &str = "usage: entrega check [<manifest.toml>...]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => std::process::exit(check(&args[1..])),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

fn check(files: &[String]) -> i32 {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = match git::text(&here, &["rev-parse", "--show-toplevel"]) {
        Ok(Some(t)) => PathBuf::from(t.trim()),
        Ok(None) => {
            eprintln!("{}: not inside a git repository", here.display());
            return 2;
        }
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let loaded = if files.is_empty() {
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
        if refusals.is_empty() { Ok(out) } else { Err(refusals) }
    };
    let all = match loaded {
        Ok(a) => a,
        Err(refusals) => {
            for r in refusals {
                eprintln!("{r}");
            }
            return 2;
        }
    };
    let mut failed = false;
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
