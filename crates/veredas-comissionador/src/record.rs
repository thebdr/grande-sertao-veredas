//! The runs folder (`docs/comissionador-plan.md`, phase 4; `docs/report-schema.md`): every run
//! kept under `runs/<id>/<nnnn>/` with the exact text that ran, its acceptance report and the rows
//! it read; read back, listed, noted and signed.
//!
//! The report is a TOML file, `veredas-report` version 1, the record of the run; a PDF or a page is
//! a drawing of it (`docs/questions.md` Q312, taken: (a) and (d)). A signed report is frozen: the
//! engine refuses a note or a second signature on it, a correction being a new run. That is the
//! engine's refusal alone, nothing sealed in the file, so a later ruling can change it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::check::Checked;
use crate::eval::{Value, time_text};
use crate::plant::{Plant, Table};
use crate::run::{RunMode, RunResult};

/// The report's format and the one version this reader knows.
pub const REPORT_FORMAT: &str = "veredas-report";
pub const REPORT_VERSION: u32 = 1;
const REPORT_NOUN: &str = "a report";
/// The report's file in a run's folder; written last, so a folder holding it is a whole run.
pub const REPORT_FILE: &str = "report.toml";
/// The copy of the test that ran.
pub const TEST_FILE: &str = "test.test";

/// Why a run was not kept, or a report not read or changed.
#[derive(Debug, Error)]
pub enum RecordError {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{path}: {reason}")]
    NotAReport { path: String, reason: String },
    #[error("the test has no `id`, so its runs have no folder")]
    NoId,
    #[error("the store did not answer for the evidence of `{table}`: {reason}")]
    Evidence { table: String, reason: String },
    #[error("{path}: the report is signed, so it is frozen: a correction is a new run")]
    Signed { path: String },
    #[error("a signature names its role and its signer, neither blank")]
    BlankSignature,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> RecordError + '_ {
    move |source| RecordError::Io { path: path.display().to_string(), source }
}

/// The acceptance report of one run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub format: String,
    pub version: u32,
    /// The tester's note, empty when none.
    #[serde(default)]
    pub note: String,
    pub test: TestPart,
    pub run: RunPart,
    #[serde(default, rename = "step")]
    pub steps: Vec<StepPart>,
    #[serde(default, rename = "criterion")]
    pub criteria: Vec<CriterionPart>,
    #[serde(default, rename = "reported")]
    pub reported: Vec<ReportedPart>,
    #[serde(default, rename = "write")]
    pub writes: Vec<WritePart>,
    #[serde(default, rename = "evidence")]
    pub evidence: Vec<EvidencePart>,
    #[serde(default, rename = "signature")]
    pub signatures: Vec<Signature>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestPart {
    pub id: String,
    pub title: String,
    /// The copy of the test that ran, in the run's folder.
    pub file: String,
    #[serde(default)]
    pub functions: Vec<FunctionPart>,
}

/// A function file the test used: as its `use` wrote it, and its copy in the run's folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionPart {
    pub used_as: String,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPart {
    pub number: u32,
    /// `live` or `replay`.
    pub mode: String,
    /// The portal session's name.
    pub session: String,
    /// The window the run covered on the store's axis, as the plant's clock reads.
    pub from: String,
    pub to: String,
    /// `passed`, `failed` or `aborted`.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepPart {
    pub step: String,
    pub attempt: u32,
    /// Absent for a step the run stopped inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub detail: String,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionPart {
    pub step: String,
    pub attempt: u32,
    pub text: String,
    #[serde(default)]
    pub measured: Vec<String>,
    pub result: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedPart {
    pub step: String,
    pub attempt: u32,
    pub text: String,
    pub note: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WritePart {
    pub step: String,
    pub attempt: u32,
    pub text: String,
    pub sent: bool,
    pub answer: String,
    pub ok: bool,
}

/// The rows of one table the run read, kept as CSV beside the report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePart {
    pub table: String,
    pub file: String,
    pub rows: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    /// What the signer signs as: `tester`, `witness`.
    pub role: String,
    pub name: String,
    /// When, as the plant's clock reads.
    pub at: String,
}

impl Report {
    /// Whether anyone has signed it: then it is frozen.
    pub fn is_signed(&self) -> bool {
        !self.signatures.is_empty()
    }
}

/// A CSV cell: quoted when it holds a comma, a quote or a line end, quotes doubled.
pub fn csv_cell(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

/// A file name for a table's evidence: its name when it is plain, else its place.
fn evidence_name(i: usize, table: &str) -> String {
    if !table.is_empty() && table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        format!("evidence/{table}.csv")
    } else {
        format!("evidence/table-{}.csv", i + 1)
    }
}

/// The folder of a test's runs: `<runs>/<id, lowercased>`.
pub fn test_folder(runs: &Path, c: &Checked) -> Result<PathBuf, RecordError> {
    let id = c.test.id().ok_or(RecordError::NoId)?;
    Ok(runs.join(id.to_ascii_lowercase()))
}

/// The run numbers kept in a test's folder, as written (`0001`), in order.
fn numbers(dir: &Path) -> Vec<u32> {
    let mut out: Vec<u32> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                (name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit())).then(|| name.parse().ok())?
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort_unstable();
    out
}

/// Keep a run: the next number in the test's folder, the exact text of the test and of every
/// function file it used, the rows of every table it read over its window as CSV, and its report,
/// written last.
pub fn keep(runs: &Path, c: &Checked, r: &RunResult, mode: &RunMode, plant: &dyn Plant) -> Result<PathBuf, RecordError> {
    let dir = test_folder(runs, c)?;
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    // The next free number: a folder created, never one another keeper made meanwhile.
    let mut number = numbers(&dir).last().copied().unwrap_or(0) + 1;
    let folder = loop {
        let f = dir.join(format!("{number:04}"));
        match std::fs::create_dir(&f) {
            Ok(()) => break f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => number += 1,
            Err(e) => return Err(io(&f)(e)),
        }
    };
    let write = |name: &str, text: &str| -> Result<(), RecordError> {
        let p = folder.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&p, text).map_err(io(&p))
    };
    write(TEST_FILE, &c.text)?;
    let mut functions = Vec::new();
    for (i, u) in c.files.iter().enumerate() {
        let file = format!("functions/{}.fn", i + 1);
        write(&file, &u.text)?;
        functions.push(FunctionPart { used_as: u.path.clone(), file });
    }
    // The evidence: every row of every table the run read, stamped in its window; a live run's
    // read once the window's end is far enough past for its last rows to be stored, as every read
    // of a live run is (C-202).
    if let RunMode::Live = mode {
        while plant.now_ms() < r.end_ms + crate::run::SETTLE_MS {
            std::thread::sleep(crate::run::POLL);
        }
    }
    let tables: Vec<Table> = plant.tables().map_err(|e| RecordError::Evidence { table: String::new(), reason: e.to_string() })?;
    let mut evidence = Vec::new();
    for (i, name) in r.tables.iter().enumerate() {
        let Some(t) = tables.iter().find(|t| &t.name == name) else { continue };
        let rows = plant
            .rows(t, r.start_ms.saturating_sub(1), r.end_ms)
            .map_err(|e| RecordError::Evidence { table: name.clone(), reason: e.to_string() })?;
        let mut csv = String::from("time,arrival,source,key,seq");
        for col in &t.columns {
            csv.push(',');
            csv.push_str(&csv_cell(&col.name));
        }
        csv.push('\n');
        for row in &rows {
            let mut line =
                vec![time_text(row.time_ms), time_text(row.arrival_ms), csv_cell(&row.source), csv_cell(row.key.as_deref().unwrap_or("")), row.seq.to_string()];
            for (_, v) in &row.columns {
                line.push(v.as_ref().map(|v: &Value| csv_cell(&v.to_string())).unwrap_or_default());
            }
            csv.push_str(&line.join(","));
            csv.push('\n');
        }
        let file = evidence_name(i, name);
        write(&file, &csv)?;
        evidence.push(EvidencePart { table: name.clone(), file, rows: rows.len() as u64 });
    }
    let report = Report {
        format: REPORT_FORMAT.to_string(),
        version: REPORT_VERSION,
        note: String::new(),
        test: TestPart {
            id: c.test.id().unwrap_or_default().to_string(),
            title: c.test.title.value.clone(),
            file: TEST_FILE.to_string(),
            functions,
        },
        run: RunPart {
            number,
            mode: mode.mode().to_string(),
            session: plant.name(),
            from: time_text(r.start_ms),
            to: time_text(r.end_ms),
            verdict: r.verdict.to_string(),
            decided_at: r.decided_at.clone(),
            reason: r.reason.clone(),
        },
        steps: r
            .steps
            .iter()
            .map(|s| StepPart {
                step: s.step.clone(),
                attempt: s.attempt,
                outcome: s.outcome.map(|o| o.to_string()),
                detail: s.detail.clone(),
                from: time_text(s.start_ms),
                to: time_text(s.end_ms),
            })
            .collect(),
        criteria: r
            .criteria
            .iter()
            .map(|x| CriterionPart { step: x.step.clone(), attempt: x.attempt, text: x.text.clone(), measured: x.measured.clone(), result: x.result.clone() })
            .collect(),
        reported: r.reported.iter().map(|x| ReportedPart { step: x.step.clone(), attempt: x.attempt, text: x.text.clone(), note: x.note }).collect(),
        writes: r
            .writes
            .iter()
            .map(|w| WritePart { step: w.step.clone(), attempt: w.attempt, text: w.text.clone(), sent: w.sent, answer: w.answer.clone(), ok: w.ok })
            .collect(),
        evidence,
        signatures: Vec::new(),
    };
    store(&folder, &report)?;
    Ok(folder)
}

/// Write a report into its folder: beside it first, then moved into place, so a reader never
/// meets half a report.
fn store(folder: &Path, report: &Report) -> Result<(), RecordError> {
    let text = toml::to_string(report).map_err(|e| RecordError::NotAReport { path: folder.display().to_string(), reason: e.to_string() })?;
    let tmp = folder.join("report.toml.part");
    std::fs::write(&tmp, &text).map_err(io(&tmp))?;
    let path = folder.join(REPORT_FILE);
    std::fs::rename(&tmp, &path).map_err(io(&path))
}

/// Read a run's report: its stamp by core's one header reader, then the whole file, a key the
/// format does not know refused.
pub fn read(folder: &Path) -> Result<Report, RecordError> {
    let path = folder.join(REPORT_FILE);
    let text = std::fs::read_to_string(&path).map_err(io(&path))?;
    parse(&text).map_err(|reason| RecordError::NotAReport { path: path.display().to_string(), reason })
}

/// Read a report's text.
pub fn parse(text: &str) -> Result<Report, String> {
    veredas_core::schema::check(text, REPORT_NOUN, REPORT_FORMAT, REPORT_VERSION).map_err(|e| e.to_string())?;
    toml::from_str::<Report>(text).map_err(|e| e.to_string().trim_end().to_string())
}

/// The runs of a test, newest first: each folder with its report (a folder without one, a run
/// being kept or cut short, is passed over).
pub fn list(runs: &Path, c: &Checked) -> Result<Vec<(PathBuf, Report)>, RecordError> {
    let dir = test_folder(runs, c)?;
    let mut out = Vec::new();
    for n in numbers(&dir).into_iter().rev() {
        let folder = dir.join(format!("{n:04}"));
        if folder.join(REPORT_FILE).exists() {
            out.push((folder.clone(), read(&folder)?));
        }
    }
    Ok(out)
}

/// Set the tester's note; refused once the report is signed.
pub fn note(folder: &Path, text: &str) -> Result<Report, RecordError> {
    let mut r = read(folder)?;
    if r.is_signed() {
        return Err(RecordError::Signed { path: folder.display().to_string() });
    }
    r.note = text.to_string();
    store(folder, &r)?;
    Ok(r)
}

/// Sign the report as `role`, by `name`, now on the plant's clock; refused once anyone has signed
/// (Q312 (d), taken: a second signature, like a note, waits for a new run).
pub fn sign(folder: &Path, role: &str, name: &str) -> Result<Report, RecordError> {
    if role.trim().is_empty() || name.trim().is_empty() {
        return Err(RecordError::BlankSignature);
    }
    let mut r = read(folder)?;
    if r.is_signed() {
        return Err(RecordError::Signed { path: folder.display().to_string() });
    }
    r.signatures.push(Signature { role: role.trim().to_string(), name: name.trim().to_string(), at: time_text(veredas_portal::serve::store_now_ms()) });
    store(folder, &r)?;
    Ok(r)
}
