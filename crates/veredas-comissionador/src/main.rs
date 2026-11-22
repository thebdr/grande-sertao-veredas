//! `veredas-comissionador`: the testing platform from the command line.
//!
//! `veredas-comissionador check <file>` reads a test file (or a function file, `.fn`) and the
//! function files a test uses, checks them whole, and prints what the designer's problems bar
//! counts, or every problem with its place. Exit codes: 0 the file is sound, 1 it has problems,
//! 2 the command line is wrong.

use std::path::Path;
use std::process::ExitCode;

use veredas_comissionador::{load_functions, load_test};

const USAGE: &str = "usage: veredas-comissionador check <file.test | file.fn>";

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, file] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if command != "check" {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let path = Path::new(file);
    let outcome = if path.extension().is_some_and(|e| e == "fn") {
        load_functions(path).map(|ff| {
            let names: Vec<String> = ff.functions.iter().map(|f| f.signature()).collect();
            format!("{file}: {}: {}", plural(names.len(), "function", "functions"), names.join("; "))
        })
    } else {
        load_test(path).map(|c| {
            format!(
                "{file}: {} · {} · {} · {}",
                plural(c.counts.steps, "step", "steps"),
                plural(c.counts.branches, "branch", "branches"),
                plural(c.counts.functions, "function", "functions"),
                plural(c.counts.gated_writes, "gated write", "gated writes"),
            )
        })
    };
    match outcome {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for p in problems {
                eprintln!("{p}");
            }
            ExitCode::from(1)
        }
    }
}
