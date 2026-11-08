//! Shared by the tail tests: a plant logger as a test writes one, with the rotation races a
//! real logger shows (a straggler landing late in the retired file, a torn write), which the
//! bench's emitter (C-025) never produces on purpose: it rotates cleanly, so these races are
//! played here, by the test, as the plant's writer.

#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use veredas_portal::LineEvent;

/// How rotated files are named.
#[derive(Clone, Debug)]
pub enum NamingScheme {
    /// `{prefix}{n}.log`, n from 1, unpadded (so a numeric sort is exercised).
    Indexed { prefix: String },
    /// The active file keeps one constant name; on rotation it is renamed to
    /// `{retired_prefix}{n}.log` and a fresh active is created.
    Fixed { active: String, retired_prefix: String },
}

/// How a rotation behaves relative to the final line of the old file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotateStyle {
    /// The old file is complete before its successor exists.
    CleanClose,
    /// The successor is created first; one last line lands in the retired file `straggle_delay`
    /// later: the writer's final flush racing the rotation. The race `drain_grace` exists for.
    LateFinalWrite,
}

pub struct FamilyWriter {
    dir: PathBuf,
    scheme: NamingScheme,
    cap_bytes: u64,
    pub rotate_style: RotateStyle,
    /// Write each line in two flushes with a pause between (torn writes).
    pub split_writes: bool,
    /// The pause before a `LateFinalWrite` straggler lands.
    pub straggle_delay: Duration,
    current: Option<File>,
    current_path: PathBuf,
    bytes_in_current: u64,
    lines_in_current: u64,
    generation: u32,
    /// The old file's handle, owed one straggler line.
    straggler: Option<File>,
    generations_finished: u32,
}

impl FamilyWriter {
    pub fn new(dir: impl Into<PathBuf>, scheme: NamingScheme, cap_bytes: u64) -> Self {
        FamilyWriter {
            dir: dir.into(),
            scheme,
            cap_bytes,
            rotate_style: RotateStyle::CleanClose,
            split_writes: false,
            straggle_delay: Duration::from_millis(15),
            current: None,
            current_path: PathBuf::new(),
            bytes_in_current: 0,
            lines_in_current: 0,
            generation: 0,
            straggler: None,
            generations_finished: 0,
        }
    }

    /// How many files this writer has completed (retired).
    pub fn generations_finished(&self) -> u32 {
        self.generations_finished
    }

    pub fn active_path(&self) -> &Path {
        &self.current_path
    }

    fn active_name(&self) -> String {
        match &self.scheme {
            NamingScheme::Indexed { prefix } => format!("{prefix}{}.log", self.generation),
            NamingScheme::Fixed { active, .. } => active.clone(),
        }
    }

    fn ensure_open(&mut self) {
        if self.current.is_none() {
            self.generation += 1;
            let name = self.active_name();
            self.current_path = self.dir.join(name);
            self.current = Some(OpenOptions::new().create(true).append(true).open(&self.current_path).unwrap());
            self.bytes_in_current = 0;
            self.lines_in_current = 0;
        }
    }

    fn write_to(file: &mut File, bytes: &[u8], split: bool) {
        if split && bytes.len() > 4 {
            let mid = bytes.len() / 2;
            file.write_all(&bytes[..mid]).unwrap();
            file.flush().unwrap();
            std::thread::sleep(Duration::from_millis(1));
            file.write_all(&bytes[mid..]).unwrap();
        } else {
            file.write_all(bytes).unwrap();
        }
        file.flush().unwrap();
    }

    /// Append one line, CRLF-terminated. The rotation happens once the cap is reached, in the
    /// configured style.
    pub fn write_line(&mut self, line: &str) {
        // A pending straggler soaks up this line: it lands, late, in the already-retired file
        // while the successor already exists on disk.
        if let Some(mut old) = self.straggler.take() {
            std::thread::sleep(self.straggle_delay);
            let bytes = format!("{line}\r\n");
            Self::write_to(&mut old, bytes.as_bytes(), self.split_writes);
            drop(old);
            self.generations_finished += 1;
            return;
        }

        self.ensure_open();
        let bytes = format!("{line}\r\n");
        {
            let file = self.current.as_mut().unwrap();
            Self::write_to(file, bytes.as_bytes(), self.split_writes);
        }
        self.bytes_in_current += bytes.len() as u64;
        self.lines_in_current += 1;

        if self.bytes_in_current >= self.cap_bytes && self.lines_in_current > 0 {
            match self.rotate_style {
                RotateStyle::CleanClose => self.rotate_clean(),
                RotateStyle::LateFinalWrite => self.rotate_keeping_straggler(),
            }
        }
    }

    fn rotate_clean(&mut self) {
        let old = self.current.take();
        drop(old);
        self.retire_current_name();
        self.generations_finished += 1;
        self.ensure_open();
    }

    fn rotate_keeping_straggler(&mut self) {
        let old = self.current.take().unwrap();
        self.retire_current_name();
        self.straggler = Some(old);
        self.ensure_open();
    }

    /// In the fixed scheme the active file moves aside for its successor; an indexed file
    /// already carries its final name.
    fn retire_current_name(&mut self) {
        if let NamingScheme::Fixed { retired_prefix, .. } = &self.scheme {
            let retired = self.dir.join(format!("{retired_prefix}{}.log", self.generation));
            std::fs::rename(&self.current_path, &retired).unwrap();
        }
    }

    /// Flush and close everything (delivering a still-owed straggler).
    pub fn finish(mut self) {
        if let Some(mut old) = self.straggler.take() {
            let _ = old.flush();
            self.generations_finished += 1;
        }
        if let Some(mut f) = self.current.take() {
            let _ = f.flush();
        }
    }
}

/// Collect events until one carries `done` as its line, or `timeout` passes; the sequence must
/// be dense from the first event collected.
pub fn collect_until(rx: &Receiver<LineEvent>, done: &str, timeout: Duration) -> Vec<LineEvent> {
    let deadline = Instant::now() + timeout;
    let mut out: Vec<LineEvent> = Vec::new();
    let mut last_seq: Option<u64> = None;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now()).expect("timed out waiting for the end marker");
        let ev = rx.recv_timeout(remaining.min(Duration::from_secs(10))).unwrap_or_else(|e| {
            panic!("the stream stalled ({e:?}) after {} lines (last: {:?})", out.len(), out.last().map(|e| &e.line))
        });
        if let Some(prev) = last_seq {
            assert_eq!(ev.seq, prev + 1, "seq gap at {:?}", ev.line);
        }
        last_seq = Some(ev.seq);
        let is_done = ev.line == done;
        out.push(ev);
        if is_done {
            return out;
        }
    }
}

/// Exactly `expected` in order, else a diagnosis of what is missing and what came twice.
pub fn assert_exact(name: &str, got: &[String], expected: &[String]) {
    let mut problems = Vec::new();
    if got.len() != expected.len() {
        problems.push(format!("count mismatch: got {} lines, expected {}", got.len(), expected.len()));
    }
    for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
        if g != e {
            problems.push(format!("first divergence at index {i}: got {g:?}, expected {e:?}"));
            break;
        }
    }
    if !problems.is_empty() {
        let mut seen = std::collections::HashMap::new();
        for line in got {
            *seen.entry(line.as_str()).or_insert(0u32) += 1;
        }
        let missing: Vec<&str> = expected.iter().filter(|e| !seen.contains_key(e.as_str())).map(String::as_str).collect();
        let dups: Vec<&str> = seen.iter().filter(|(_, c)| **c > 1).map(|(l, _)| *l).collect();
        panic!(
            "{name}: the sequence is broken:\n  {}\n  missing {} lines (first few: {:?})\n  duplicated {} lines (first few: {:?})",
            problems.join("\n  "),
            missing.len(),
            &missing[..missing.len().min(10)],
            dups.len(),
            &dups[..dups.len().min(10)],
        );
    }
}

/// The plant the session and the server tests meet, and the session over it.
#[cfg(feature = "store")] // the plant needs the session, which the light build has not
pub mod plant;
