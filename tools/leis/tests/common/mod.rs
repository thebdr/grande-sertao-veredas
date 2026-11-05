//! Shared by the laws' test files: synthetic suites from manifest texts, exact violation strings,
//! and a throwaway tree on disk for the reader.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use leis::{Suite, Violation};

pub const NONE: Vec<String> = Vec::new();
pub const LEIS: &str = "[package]\nname = \"leis\"\nversion = \"0.1.0\"\n";

pub fn ws(members: &[&str]) -> String {
    let list: Vec<String> = members.iter().map(|m| format!("\"{m}\"")).collect();
    format!("[workspace]\nmembers = [{}]\n[workspace.package]\nedition = \"2024\"\n", list.join(", "))
}

pub fn pkg(name: &str, version: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n")
}

pub fn with_deps(name: &str, deps: &[&str]) -> String {
    let mut m = pkg(name, "2.0.0");
    m.push_str("[dependencies]\n");
    for d in deps {
        m.push_str(&format!("{d} = {{ path = \"../{d}\" }}\n"));
    }
    m
}

pub fn suite(workspace: &str, crates: &[(&str, &str)], plans: &[(&str, &str)]) -> Suite {
    let plans: BTreeMap<String, String> = plans.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Suite::parse(workspace, crates, plans).expect("the synthetic suite parses")
}

pub fn strings(v: Vec<Violation>) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

/// A tree on disk under the OS temp folder, removed on drop; files are `(relative path, text)`.
pub struct Tree {
    pub root: PathBuf,
}

impl Tree {
    pub fn new(name: &str, files: &[(&str, &str)]) -> Tree {
        let root = std::env::temp_dir().join(format!("leis-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (rel, text) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().expect("a file has a parent")).expect("the folder is made");
            fs::write(&path, text).expect("the file is written");
        }
        Tree { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
