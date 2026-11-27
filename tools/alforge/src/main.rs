//! `alforge check`: read every suite release manifest of the repository the current folder is in
//! (`releases/*.toml`, the folder judged whole: a manifest is never judged apart from the others)
//! and check each against the repository's history and against each other, printing one line per
//! manifest.
//!
//! `alforge cut <release> [--trial] [--out <dir>]`: cut a suite release from HEAD of the
//! repository the current folder is in (docs/alforge-plan.md, "How a release is cut"): the whole
//! suite run first (a trial cut skips it and its manifest says so), the executables built in the
//! builder machine, the bundle packed under `target/alforge/<release>/` and run together, then the
//! manifest written into `releases/` (or `--out`).
//!
//! `alforge components <commit> [--release <release>]`: print the component list of a commit of the
//! repository the current folder is in, as the cut writes it into a release's bundle
//! (docs/alforge-plan.md, "The component list"): CycloneDX 1.6 JSON, from the commit alone.
//!
//! `alforge maquina <release>`: build the tester machine of a release already cut, from its bundle
//! under `target/alforge/<release>/`, into `veredas-<release>.ova` beside it
//! (docs/alforge-plan.md, "How the image is built").
//!
//! `alforge prova <release>`: prove the tester machine built for a release: its OVA's disk booted
//! copy-on-write under QEMU on VMware's disk and network devices, the release's sums checked at
//! boot, the plant read from the host, the desktop drawn, the disk unchanged
//! (docs/alforge-plan.md, "How a build is proven").
//!
//! Exit codes: 0 when every manifest checks, the release is cut, the machine built or proven; 2 when one is refused, the
//! reasons on stderr; 1 on a usage error.

use std::path::PathBuf;

use alforge::{git, manifest};

const USAGE: &str = "usage: alforge check\n       alforge cut <release> [--trial] [--out <dir>]\n       alforge components <commit> [--release <release>]\n       alforge maquina <release>\n       alforge prova <release>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") if args.len() == 1 => std::process::exit(check()),
        Some("cut") => std::process::exit(cut(&args[1..])),
        Some("components") if !args.get(1).is_some_and(|c| c.starts_with('-')) && (args.len() == 2 || (args.len() == 4 && args[2] == "--release")) => {
            std::process::exit(components(&args[1], args.get(3).map(String::as_str)))
        }
        Some("maquina") if args.len() == 2 => std::process::exit(maquina(&args[1])),
        Some("prova") if args.len() == 2 => std::process::exit(prova(&args[1])),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

/// The repository the current folder is in, or the reason it is none: git's answer taken as bytes,
/// only its final line break dropped (a path may end in a space or not be UTF-8), and refused
/// unless it is a folder.
fn repository() -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStrExt;
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match git::run(&here, &["rev-parse", "--show-toplevel"]) {
        Ok(Some(mut bytes)) => {
            if bytes.last() == Some(&b'\n') {
                bytes.pop();
            }
            let root = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
            if root.is_dir() { Ok(root) } else { Err(format!("{}: git names a repository root that is no folder", root.display())) }
        }
        Ok(None) => Err(format!("{}: not inside a git repository", here.display())),
        Err(e) => Err(e.to_string()),
    }
}

fn components(commit: &str, release: Option<&str>) -> i32 {
    let root = match repository() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if let Some(r) = release
        && semver::Version::parse(r).is_err()
    {
        eprintln!("`{r}` is not SemVer: a release is named by its version");
        return 1;
    }
    // The commit by its full id, as a release's manifest names it: the name resolved first, then
    // peeled to its commit (a `:/<text>` name takes all that follows as its pattern).
    let named = match git::text(&root, &["rev-parse", "--verify", "--quiet", "--end-of-options", commit]) {
        Ok(Some(id)) => id.trim().to_string(),
        Ok(None) => {
            eprintln!("{commit} is no commit of {}", root.display());
            return 2;
        }
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let id = match git::text(&root, &["rev-parse", "--verify", "--quiet", &format!("{named}^{{commit}}")]) {
        Ok(Some(id)) => id.trim().to_string(),
        Ok(None) => {
            eprintln!("{commit} is no commit of {}", root.display());
            return 2;
        }
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match alforge::components::list(&root, &id, release) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
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
    match alforge::maquina::from_release(&root, release, "x86_64-unknown-linux-gnu") {
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

fn prova(release: &str) -> i32 {
    let root = match repository() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match alforge::maquina::prove_release(&root, release, "x86_64-unknown-linux-gnu") {
        Ok((proof, faults)) => {
            let names = |s: &serde_json::Value| -> String {
                s["objects"].as_array().into_iter().flatten().map(|o| format!("{}={}", o["id"].as_str().unwrap_or("?"), o["value"])).collect::<Vec<_>>().join(" ")
            };
            println!("tester machine of release {release}, its disk {}", proof.vmdk.display());
            println!("  boot: {}", if proof.console.contains(&format!("VEREDAS-SUMS-OK {release}")) { "the release's sums checked" } else { "no check of the sums seen" });
            println!("  plant from the host: page {}, write {} {}", proof.plant.page.status, proof.plant.write.status, proof.plant.write.body);
            println!("  objects: {}", names(&proof.plant.second));
            println!("  screen: {} colours, the page in its canvas colour over {:.0} % of it ({})", proof.colours, proof.page_share * 100.0, proof.screen.display());
            println!("  the OVA as VMware reads it: {}", if proof.ova.is_empty() { "members, descriptor and sums right".to_string() } else { proof.ova.join("; ") });
            println!("  disk unchanged: {}", proof.before == proof.after);
            for f in &faults {
                eprintln!("{f}");
            }
            // The OVA keeps the disk: the copy taken out of it for the boot is not kept.
            if let Some(opened) = proof.vmdk.parent() {
                let _ = std::fs::remove_dir_all(opened);
            }
            if faults.is_empty() { 0 } else { 2 }
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
    let options = alforge::cut::Options {
        // A trial's manifest goes beside its bundle, never into releases/.
        out: out.unwrap_or_else(|| if trial { root.join("target/alforge").join(release.to_string()) } else { root.join("releases") }),
        work: root.join("target/alforge").join(release.to_string()),
        release,
        run_suite: !trial,
    };
    match alforge::cut::cut(&root, &options) {
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

fn check() -> i32 {
    let root = match repository() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // Every manifest that reads is judged, and every refusal said, whatever else was refused.
    let (all, refusals) = manifest::load_dir(&root.join("releases"));
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
