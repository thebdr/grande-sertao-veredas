//! Opening a workbook that links to other workbooks.
//!
//! The architect's ruling, made for Veredas 1's Ábaco on 2026-11-03 (its AB7) and carried here: the
//! values a workbook carries for the cells of other workbooks (its cache, what Excel shows with
//! the linked book closed) may be the default, but on opening a file that holds links the person
//! is asked whether the linked workbooks are opened and calculated. A setting of three values
//! governs the question: *always ask* (the default) asks; *always recalculate linked spreadsheets*
//! and *always use linked spreadsheets' cached values* do not.
//!
//! The answers map onto the engine's resolution strategies (`ironcalc_base::external`): the file's
//! cache (`CachedResolver`) and the linked workbooks opened and evaluated here
//! (`ironcalc::import::LinkedResolver`). Veredas 1 offered a third answer, Excel opening and
//! calculating the linked workbooks over its COM host; that host is not carried
//! (`docs/questions.md` Q342, ruled 2026-11-25), and the answer is not offered.

use std::path::{Path, PathBuf};

use ironcalc::import::{LinkStatus, LinkedResolver, load_from_xlsx};
use ironcalc_base::Model;
use ironcalc_base::external::{CachedResolver, ExternalBook};

/// Whether opening a workbook with links asks the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkPolicy {
    /// Ask on every file that has links (the default).
    #[default]
    AlwaysAsk,
    /// Open and calculate the linked workbooks, without asking.
    AlwaysRecalculateLinked,
    /// Use the cached values the file carries, without asking.
    AlwaysUseCached,
}

impl LinkPolicy {
    /// The answer a policy gives on its own, `None` when it asks.
    pub fn choice(self) -> Option<LinkChoice> {
        match self {
            LinkPolicy::AlwaysAsk => None,
            LinkPolicy::AlwaysRecalculateLinked => Some(LinkChoice::OpenAndCalculate),
            LinkPolicy::AlwaysUseCached => Some(LinkChoice::UseCached),
        }
    }

    /// The setting's name as a person reads it.
    pub fn label(self) -> &'static str {
        match self {
            LinkPolicy::AlwaysAsk => "always ask",
            LinkPolicy::AlwaysRecalculateLinked => "always recalculate linked spreadsheets",
            LinkPolicy::AlwaysUseCached => "always use linked spreadsheets' cached values",
        }
    }
}

/// How the links of one workbook are resolved for this opening.
// Veredas 1's third answer, "open and calculate in Excel", is not offered: the Excel host is not
// carried (docs/questions.md Q342, ruled 2026-11-25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkChoice {
    /// The values the file carries (Excel's behaviour with the linked book closed).
    UseCached,
    /// Open the linked workbooks here and read their live values.
    OpenAndCalculate,
}

impl LinkChoice {
    /// The answer's name as a person reads it.
    pub fn label(self) -> &'static str {
        match self {
            LinkChoice::UseCached => "use cached values",
            LinkChoice::OpenAndCalculate => "open and calculate",
        }
    }
}

/// What happened to the links when the workbook was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    /// The file links to no other workbook; the policy was not consulted.
    NoLinks,
    /// The cache answers for every linked cell.
    Cached { books: usize },
    /// The linked workbooks were opened: where each stands.
    Linked(Vec<LinkStatus>),
}

/// An opened workbook, evaluated: its path, its model and how its links were resolved.
pub struct OpenedWorkbook {
    pub path: PathBuf,
    pub model: Model<'static>,
    /// The answer applied, `None` when the file has no links.
    pub choice: Option<LinkChoice>,
    pub links: LinkOutcome,
}

/// Why a workbook could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("cannot open {path}: {source}")]
    Load {
        path: PathBuf,
        #[source]
        source: ironcalc::error::XlsxError,
    },
}

/// The locale, time zone and language the engine is driven with. A workbook stores its formulas
/// in English whatever the language of the Excel that wrote it, and Excel's cached values do not
/// depend on the time zone but for the volatile date functions.
pub const LOCALE: &str = "en";
pub const TZ: &str = "UTC";
pub const LANGUAGE: &str = "en";

/// Opens `path` under `policy` and evaluates it. `prompt` is called only when the policy is
/// [`LinkPolicy::AlwaysAsk`] *and* the file links to other workbooks; it is given those books
/// (targets, sheet names, cached cells) so the question can name them. A DDE or OLE link (a
/// spreadsheet reading a PLC through a DDE server, say) is no workbook: the engine keeps it as a
/// placeholder with no target, and it is neither asked about, counted nor listed.
pub fn open(
    path: &Path,
    policy: LinkPolicy,
    prompt: impl FnOnce(&[ExternalBook]) -> LinkChoice,
) -> Result<OpenedWorkbook, OpenError> {
    let path_str = path.to_string_lossy().to_string();
    let mut model = load_from_xlsx(&path_str, LOCALE, TZ, LANGUAGE)
        .map_err(|source| OpenError::Load { path: path.to_path_buf(), source })?;
    let workbooks: Vec<ExternalBook> = model.workbook.external_books.iter().filter(|b| is_workbook(b)).cloned().collect();
    if workbooks.is_empty() {
        model.evaluate();
        return Ok(OpenedWorkbook { path: path.to_path_buf(), model, choice: None, links: LinkOutcome::NoLinks });
    }
    let choice = match policy.choice() {
        Some(choice) => choice,
        None => prompt(&workbooks),
    };
    let links = match choice {
        LinkChoice::UseCached => {
            model.set_external_resolver(Box::new(CachedResolver));
            LinkOutcome::Cached { books: workbooks.len() }
        }
        LinkChoice::OpenAndCalculate => {
            let mut resolver = LinkedResolver::for_workbook(path, LOCALE, TZ, LANGUAGE);
            let statuses = resolver.preload(&model.workbook.external_books);
            model.set_external_resolver(Box::new(resolver));
            // the books keep their numbers (`[2]` is the second link whatever the first is)
            LinkOutcome::Linked(statuses.into_iter().filter(|s| !s.target.is_empty()).collect())
        }
    };
    model.evaluate();
    Ok(OpenedWorkbook { path: path.to_path_buf(), model, choice: Some(choice), links })
}

/// Whether a link is to another workbook: the engine reads a DDE or OLE link into a placeholder
/// with no target.
fn is_workbook(book: &ExternalBook) -> bool {
    !book.target.is_empty()
}
