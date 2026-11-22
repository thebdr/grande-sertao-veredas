//! `abaco-oracle`: Ábaco's engine measured against the values Excel cached (`src/lib.rs`).
//!
//! ```text
//! abaco-oracle [--out report.md] [--baseline-out baseline.toml --measured-by TEXT --measured-on DATE]
//!              [--roundtrip-dir DIR | --no-roundtrip] [--examples N] [--links cached|linked|ask]
//!              [--today YYYY-MM-DD] [--corpus DIR] <workbook.xlsx>...
//! ```
//!
//! `--today` pins the engine's clock to noon UTC of that day (TODAY, NOW), so a measure compares
//! with one taken that day; without it the system's clock is read.
//!
//! ```text
//! ```
//!
//! The report is Markdown, on stdout or in `--out`. `--corpus DIR` adds every workbook under DIR
//! that Microsoft Excel saved. `--baseline-out` writes the run's numbers as a stamped baseline
//! (both resolutions of the links are measured for it), paths taken below the corpus folder. The
//! inputs are opened read-only; the round trip's copy goes to `--roundtrip-dir` (by default a
//! folder in the system's temporary directory), never beside the input.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use abaco_oracle::{BASELINE_FORMAT, BASELINE_VERSION, Baseline, BaselineFile, LinksMode, Options, Probe, corpus, run_file, write_report};

const USAGE: &str = "usage: abaco-oracle [--out report.md] [--baseline-out baseline.toml --measured-by TEXT --measured-on DATE] [--roundtrip-dir DIR | --no-roundtrip] [--examples N] [--links cached|linked|ask] [--today YYYY-MM-DD] [--corpus DIR] <workbook.xlsx>...";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut files: Vec<PathBuf> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut baseline_out: Option<PathBuf> = None;
    let mut measured_by = String::new();
    let mut measured_on = String::new();
    let mut corpus_dir: Option<PathBuf> = None;
    let mut roundtrip_dir = Some(std::env::temp_dir().join("abaco-oracle"));
    let mut max_examples = 8usize;
    let mut links = LinksMode::Cached;
    let mut clock: Option<i64> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--baseline-out" => baseline_out = args.next().map(PathBuf::from),
            "--measured-by" => measured_by = args.next().unwrap_or_default(),
            "--measured-on" => measured_on = args.next().unwrap_or_default(),
            "--roundtrip-dir" => roundtrip_dir = args.next().map(PathBuf::from),
            "--no-roundtrip" => roundtrip_dir = None,
            "--examples" => max_examples = args.next().and_then(|s| s.parse().ok()).unwrap_or(max_examples),
            "--corpus" => corpus_dir = args.next().map(PathBuf::from),
            "--today" => {
                let value = args.next().unwrap_or_default();
                match abaco_oracle::noon_utc(&value) {
                    Some(ms) => clock = Some(ms),
                    None => {
                        eprintln!("--today: expected YYYY-MM-DD, got `{value}`\n{USAGE}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--links" => {
                let value = args.next().unwrap_or_default();
                match LinksMode::parse(&value) {
                    Some(mode) => links = mode,
                    None => {
                        eprintln!("--links: expected cached|linked|ask, got `{value}`\n{USAGE}");
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                eprintln!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => files.push(PathBuf::from(other)),
        }
    }
    if let Some(dir) = &corpus_dir {
        match corpus(dir) {
            Ok(found) => files.extend(found),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if files.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let mut probe = Probe::default();
    let options = Options { links, max_examples, roundtrip_dir: roundtrip_dir.clone(), verbose: true, clock };
    let reports: Vec<_> = files.iter().map(|f| run_file(f, &options, &mut probe)).collect();
    let md = write_report(&reports, &probe, links, max_examples);
    match &out {
        Some(p) => {
            if let Err(e) = std::fs::write(p, md) {
                eprintln!("cannot write {}: {e}", p.display());
                return ExitCode::FAILURE;
            }
            eprintln!("report written to {}", p.display());
        }
        None => print!("{md}"),
    }

    if let Some(path) = baseline_out {
        let root = corpus_dir.clone().unwrap_or_default();
        let quiet = |links| Options { links, max_examples: 0, roundtrip_dir: None, verbose: false, clock };
        let mut entries = Vec::new();
        for (f, cached) in files.iter().zip(&reports) {
            let cached = if links == LinksMode::Cached { cached.measure() } else { run_file(f, &quiet(LinksMode::Cached), &mut probe).measure() };
            let linked = run_file(f, &quiet(LinksMode::Linked), &mut probe).measure();
            let unmeasured = (cached.is_none() && linked.is_none()).then(|| "not measured: its expected side or the engine failed".to_string());
            entries.push(BaselineFile { path: relative(f, &root), cached, linked, unmeasured });
        }
        let baseline = Baseline {
            format: BASELINE_FORMAT.to_string(),
            version: BASELINE_VERSION,
            measured_by,
            measured_on,
            corpus: root.to_string_lossy().replace('\\', "/"),
            file: entries,
        };
        match toml::to_string(&baseline) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("cannot write {}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            }
            Err(e) => {
                eprintln!("cannot write the baseline: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// `f` below `root`, forward slashes; `f` as it is when it is not below it.
fn relative(f: &Path, root: &Path) -> String {
    f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/")
}
