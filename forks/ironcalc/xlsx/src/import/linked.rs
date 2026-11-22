//! The *linked* strategy for `[1]Sheet!A1`: open the linked workbook from
//! disk, evaluate it and read live values — what Excel does when the linked
//! workbook is open, and what a user who asks to "open and calculate the
//! linked spreadsheets" means.
//!
//! Each external book is loaded once, lazily, on the first cell asked of it,
//! with the same loader as the workbook itself (`load_from_xlsx`). A linked
//! workbook may itself link further: it gets a resolver of its own with one
//! less level of `depth`, and the chain of files being opened is carried
//! down so that a cycle (A links B links A) is cut rather than followed. At
//! depth zero a linked workbook reads its own links from its cache.
//!
//! A linked file that cannot be opened — missing, unreadable, cyclic — is
//! reported in the resolver's [`LinkStatus`]es; its cells fall back to the
//! cache the workbook carries when `fallback_to_cache` is on (the default),
//! and are `#REF!` otherwise.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use ironcalc_base::{
    external::{CachedResolver, ExternalBook, ExternalLookup, ExternalResolver, ExternalValue},
    types::{Cell, FormulaValue, SpillValue},
    Model,
};

use crate::import::load_from_xlsx;

/// How one external book was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// Not asked for yet (books load on their first lookup; `preload` loads all).
    Pending,
    /// Opened and evaluated from this file.
    Loaded,
    /// Could not be opened: the reason. Cells come from the cache if the
    /// fallback is on, else `#REF!`.
    Unavailable(String),
}

/// One external book and where the linked strategy stands with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkStatus {
    /// 1-based, as written in formulas.
    pub index: u32,
    /// The target as the file spells it.
    pub target: String,
    /// The path the target resolved to on this machine.
    pub path: PathBuf,
    pub state: LinkState,
}

enum LinkedBook {
    Loaded(Box<Model<'static>>),
    Unavailable(String),
}

/// Reads cells of other workbooks by opening them. See the module docs.
pub struct LinkedResolver {
    /// The folder relative targets are resolved against: the folder of the
    /// workbook that holds the links.
    base_dir: PathBuf,
    /// How many levels of links this resolver opens: 0 opens nothing (the
    /// cache answers), 1 opens the books of its workbook and no further,
    /// and so on. A book it opens gets a resolver with one less.
    depth: u32,
    /// Canonical paths of the workbooks being opened above this one.
    chain: Vec<PathBuf>,
    fallback_to_cache: bool,
    locale: String,
    tz: String,
    language: &'static str,
    /// Loaded (or failed) books by 0-based index.
    books: HashMap<usize, LinkedBook>,
}

/// The default number of levels of links followed below the workbook.
pub const DEFAULT_LINK_DEPTH: u32 = 2;

impl LinkedResolver {
    /// A resolver for a workbook at `workbook_path`: relative link targets
    /// are files in its folder. `locale`, `tz` and `language` are what the
    /// linked workbooks are loaded with.
    pub fn for_workbook(
        workbook_path: &Path,
        locale: &str,
        tz: &str,
        language: &'static str,
    ) -> Self {
        let base_dir = workbook_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let chain = vec![canonical(workbook_path)];
        LinkedResolver {
            base_dir,
            depth: DEFAULT_LINK_DEPTH,
            chain,
            fallback_to_cache: true,
            locale: locale.to_string(),
            tz: tz.to_string(),
            language,
            books: HashMap::new(),
        }
    }

    /// How many levels of links are opened below this workbook (default 2):
    /// 0 reads everything from the cache, 1 opens the linked workbooks but
    /// reads their own links from their caches, 2 opens those too.
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// Whether a book that cannot be opened is read from the cache (default)
    /// or is `#REF!`.
    pub fn with_cache_fallback(mut self, fallback: bool) -> Self {
        self.fallback_to_cache = fallback;
        self
    }

    /// The path the target of `book` resolves to: an absolute path as is, a
    /// relative one against the workbook's folder, a URL by its file name in
    /// that folder (Excel keeps the relative name beside the URL; this is
    /// for the files where it did not).
    pub fn resolve_target(&self, book: &ExternalBook) -> PathBuf {
        let target = book.target.as_str();
        if target.starts_with("http://") || target.starts_with("https://") {
            let file_name = target.rsplit('/').next().unwrap_or(target);
            let file_name = percent_decode(file_name);
            return self.base_dir.join(file_name);
        }
        // file:///C:/x/y.xlsx
        if let Some(rest) = target.strip_prefix("file:///") {
            return PathBuf::from(percent_decode(rest));
        }
        let path = Path::new(target);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        }
    }

    /// Where every book stands. Books not yet looked up are `Pending`;
    /// `preload` loads them all.
    pub fn status(&self, books: &[ExternalBook]) -> Vec<LinkStatus> {
        books
            .iter()
            .enumerate()
            .map(|(index, book)| LinkStatus {
                index: index as u32 + 1,
                target: book.target.clone(),
                path: self.resolve_target(book),
                state: match self.books.get(&index) {
                    None => LinkState::Pending,
                    Some(LinkedBook::Loaded(_)) => LinkState::Loaded,
                    Some(LinkedBook::Unavailable(reason)) => LinkState::Unavailable(reason.clone()),
                },
            })
            .collect()
    }

    /// Opens and evaluates every book now and reports where each stands.
    pub fn preload(&mut self, books: &[ExternalBook]) -> Vec<LinkStatus> {
        for (index, book) in books.iter().enumerate() {
            self.load(index, book);
        }
        self.status(books)
    }

    fn load(&mut self, index: usize, book: &ExternalBook) {
        if self.books.contains_key(&index) {
            return;
        }
        let loaded = self.open(book);
        self.books.insert(index, loaded);
    }

    fn open(&self, book: &ExternalBook) -> LinkedBook {
        if self.depth == 0 {
            return LinkedBook::Unavailable("the depth of links followed is exhausted".to_string());
        }
        if book.target.is_empty() {
            return LinkedBook::Unavailable("the link has no target".to_string());
        }
        let path = self.resolve_target(book);
        if !path.is_file() {
            return LinkedBook::Unavailable(format!("{} not found", path.display()));
        }
        let canonical_path = canonical(&path);
        if self.chain.contains(&canonical_path) {
            return LinkedBook::Unavailable(format!(
                "{} is already being opened (a cycle of links)",
                path.display()
            ));
        }
        let path_str = path.to_string_lossy().to_string();
        let mut model = match load_from_xlsx(&path_str, &self.locale, &self.tz, self.language) {
            Ok(model) => model,
            Err(error) => return LinkedBook::Unavailable(format!("{}: {error}", path.display())),
        };
        if !model.workbook.external_books.is_empty() {
            let mut chain = self.chain.clone();
            chain.push(canonical_path);
            let nested = LinkedResolver {
                base_dir: path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from(".")),
                depth: self.depth - 1,
                chain,
                fallback_to_cache: self.fallback_to_cache,
                locale: self.locale.clone(),
                tz: self.tz.clone(),
                language: self.language,
                books: HashMap::new(),
            };
            model.set_external_resolver(Box::new(nested));
        }
        model.evaluate();
        LinkedBook::Loaded(Box::new(model))
    }
}

/// The value of a cell of an evaluated model, as an external value.
fn value_of(model: &Model, sheet: u32, row: i32, column: i32) -> ExternalLookup {
    let Ok(worksheet) = model.workbook.worksheet(sheet) else {
        return ExternalLookup::NotAvailable;
    };
    let Some(cell) = worksheet.cell(row, column) else {
        return ExternalLookup::Empty;
    };
    let from_formula = |v: &FormulaValue| match v {
        FormulaValue::Boolean(b) => ExternalLookup::Value(ExternalValue::Boolean(*b)),
        FormulaValue::Number(n) => ExternalLookup::Value(ExternalValue::Number(*n)),
        FormulaValue::Text(s) => ExternalLookup::Value(ExternalValue::String(s.clone())),
        FormulaValue::Error { ei, .. } => ExternalLookup::Value(ExternalValue::Error(ei.clone())),
        // evaluated above; an unevaluated formula is an empty answer, never a guess
        FormulaValue::Unevaluated => ExternalLookup::NotAvailable,
    };
    match cell.as_ref() {
        Cell::EmptyCell { .. } => ExternalLookup::Empty,
        Cell::BooleanCell { v, .. } => ExternalLookup::Value(ExternalValue::Boolean(*v)),
        Cell::NumberCell { v, .. } => ExternalLookup::Value(ExternalValue::Number(*v)),
        Cell::ErrorCell { ei, .. } => ExternalLookup::Value(ExternalValue::Error(ei.clone())),
        Cell::SharedString { si, .. } => match model.workbook.shared_strings.get(*si as usize) {
            Some(s) => ExternalLookup::Value(ExternalValue::String(s.clone())),
            None => ExternalLookup::NotAvailable,
        },
        Cell::CellFormula { v, .. } | Cell::ArrayFormula { v, .. } => from_formula(v),
        Cell::SpillCell { v, .. } => match v {
            SpillValue::Boolean(b) => ExternalLookup::Value(ExternalValue::Boolean(*b)),
            SpillValue::Number(n) => ExternalLookup::Value(ExternalValue::Number(*n)),
            SpillValue::Text(s) => ExternalLookup::Value(ExternalValue::String(s.clone())),
            SpillValue::Error(e) => ExternalLookup::Value(ExternalValue::Error(e.clone())),
        },
    }
}

impl ExternalResolver for LinkedResolver {
    fn resolve(
        &mut self,
        books: &[ExternalBook],
        book: u32,
        sheet: &str,
        row: i32,
        column: i32,
    ) -> ExternalLookup {
        let Some(index) = book.checked_sub(1).map(|i| i as usize) else {
            return ExternalLookup::NotAvailable;
        };
        let Some(external) = books.get(index) else {
            return ExternalLookup::NotAvailable;
        };
        self.load(index, external);
        match self.books.get(&index) {
            Some(LinkedBook::Loaded(model)) => {
                let sheet_index = model.workbook.worksheets.iter().position(|ws| {
                    ws.name.eq_ignore_ascii_case(sheet)
                        || ws.name.to_lowercase() == sheet.to_lowercase()
                });
                match sheet_index {
                    Some(i) => value_of(model, i as u32, row, column),
                    None => ExternalLookup::NotAvailable,
                }
            }
            Some(LinkedBook::Unavailable(_)) | None => {
                if self.fallback_to_cache {
                    CachedResolver.resolve(books, book, sheet, row, column)
                } else {
                    ExternalLookup::NotAvailable
                }
            }
        }
    }

    fn strategy(&self) -> &'static str {
        "linked"
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// `%20` → space, for a URL's file name. Bytes that are not valid UTF-8
/// after decoding are kept as `%XX`.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use ironcalc_base::external::ExternalSheet;

    use super::*;
    use crate::export::save_to_xlsx;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ironcalc_linked_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A workbook with `cells` (A1 text) on its first sheet, saved as `name`.
    fn save_book(
        dir: &Path,
        name: &str,
        cells: &[(&str, &str)],
        books: Vec<ExternalBook>,
    ) -> PathBuf {
        let mut model = Model::new_empty(name, "en", "UTC", "en").unwrap();
        model.workbook.external_books = books;
        for (reference, value) in cells {
            let r = model
                .parse_reference(&format!("Sheet1!{reference}"))
                .unwrap();
            model
                .set_user_input(r.sheet, r.row, r.column, value.to_string())
                .unwrap();
        }
        model.evaluate();
        let path = dir.join(name);
        save_to_xlsx(&model, path.to_str().unwrap()).unwrap();
        path
    }

    fn link_to(target: &str, cached_b2: f64) -> ExternalBook {
        let mut cells = HashMap::new();
        cells.insert((2, 2), ExternalValue::Number(cached_b2));
        ExternalBook {
            target: target.to_string(),
            absolute_url: None,
            sheets: vec![ExternalSheet {
                name: "Sheet1".to_string(),
                cells,
            }],
            defined_names: vec![],
        }
    }

    #[test]
    fn live_values_replace_the_cache_and_a_missing_book_falls_back() {
        let dir = temp_dir("live");
        // the companion: B2 is 10 live
        save_book(
            &dir,
            "companion.xlsx",
            &[("B2", "10"), ("B3", "=B2*2"), ("B4", "live text")],
            vec![],
        );
        // the main book caches B2 as 999 and links a second book that is not there
        let main = save_book(
            &dir,
            "main.xlsx",
            &[
                ("A1", "=[1]Sheet1!$B$2"),
                ("A2", "=[1]Sheet1!$B$3"),
                ("A3", "=[1]Sheet1!$B$4"),
                ("A4", "=[2]Sheet1!$B$2"),
                ("A5", "=[1]Sheet1!$Z$9"),
            ],
            vec![
                link_to("companion.xlsx", 999.0),
                link_to("missing.xlsx", 7.0),
            ],
        );
        let mut model = load_from_xlsx(main.to_str().unwrap(), "en", "UTC", "en").unwrap();
        // cached first
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "999");
        assert_eq!(model.get_formatted_cell_value(0, 4, 1).unwrap(), "7");

        let mut resolver = LinkedResolver::for_workbook(&main, "en", "UTC", "en");
        let status = resolver.preload(&model.workbook.external_books);
        assert_eq!(status.len(), 2);
        assert_eq!(status[0].index, 1);
        assert_eq!(status[0].state, LinkState::Loaded);
        assert_eq!(status[0].path, dir.join("companion.xlsx"));
        assert_eq!(status[1].index, 2);
        assert!(
            matches!(&status[1].state, LinkState::Unavailable(reason) if reason.ends_with("missing.xlsx not found")),
            "{:?}",
            status[1]
        );
        model.set_external_resolver(Box::new(resolver));
        assert_eq!(model.external_strategy(), "linked");
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "10");
        assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "20");
        assert_eq!(
            model.get_formatted_cell_value(0, 3, 1).unwrap(),
            "live text"
        );
        // the missing book is read from the cache
        assert_eq!(model.get_formatted_cell_value(0, 4, 1).unwrap(), "7");
        // an empty cell of the live book is 0
        assert_eq!(model.get_formatted_cell_value(0, 5, 1).unwrap(), "0");

        // without the fallback the missing book is #REF!
        let resolver =
            LinkedResolver::for_workbook(&main, "en", "UTC", "en").with_cache_fallback(false);
        model.set_external_resolver(Box::new(resolver));
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "10");
        assert_eq!(model.get_formatted_cell_value(0, 4, 1).unwrap(), "#REF!");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_links_follow_depth_and_cycles_are_cut() {
        let dir = temp_dir("nested");
        // c.xlsx: plain values; b.xlsx links c; a.xlsx links b; and c links a (a cycle)
        let a_path = dir.join("a.xlsx");
        save_book(
            &dir,
            "c.xlsx",
            &[("B2", "5"), ("B3", "=[1]Sheet1!$B$2")],
            vec![link_to("a.xlsx", -1.0)],
        );
        save_book(
            &dir,
            "b.xlsx",
            &[("B2", "=[1]Sheet1!$B$2+1")],
            vec![link_to("c.xlsx", 100.0)],
        );
        save_book(
            &dir,
            "a.xlsx",
            &[("B2", "=[1]Sheet1!$B$2+1"), ("B3", "7")],
            vec![link_to("b.xlsx", 1000.0)],
        );
        assert_eq!(a_path, dir.join("a.xlsx"));

        let mut model = load_from_xlsx(a_path.to_str().unwrap(), "en", "UTC", "en").unwrap();
        // depth 2: a → b (live) → c (live): 5+1+1
        model.set_external_resolver(Box::new(LinkedResolver::for_workbook(
            &a_path, "en", "UTC", "en",
        )));
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 2, 2).unwrap(), "7");
        // depth 1: a → b (live) → c from b's cache (100): 100+1+1
        model.set_external_resolver(Box::new(
            LinkedResolver::for_workbook(&a_path, "en", "UTC", "en").with_depth(1),
        ));
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 2, 2).unwrap(), "102");
        // depth 0: a → b from a's cache (1000)
        model.set_external_resolver(Box::new(
            LinkedResolver::for_workbook(&a_path, "en", "UTC", "en").with_depth(0),
        ));
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 2, 2).unwrap(), "1001");

        // the cycle: c's B3 reads a's B2 through the chain a → b → c → a: cut,
        // c reads its cache for a (-1), and nothing hangs
        let c_path = dir.join("c.xlsx");
        let mut c = load_from_xlsx(c_path.to_str().unwrap(), "en", "UTC", "en").unwrap();
        let mut resolver = LinkedResolver::for_workbook(&c_path, "en", "UTC", "en").with_depth(10);
        let status = resolver.preload(&c.workbook.external_books);
        assert_eq!(status[0].state, LinkState::Loaded);
        c.set_external_resolver(Box::new(resolver));
        c.evaluate();
        // a's B2 = b's B2 + 1 = (c's B2 + 1) + 1 = 7 — a's own chain reaches c
        // while c is being opened above it, so a's view of c is c's… no: the
        // chain carried down is [c, a, b]; b opening c finds it in the chain
        // and reads b's cache for c (100). a's B2 = 100 + 1 + 1 = 102.
        assert_eq!(c.get_formatted_cell_value(0, 3, 2).unwrap(), "102");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn targets_resolve_against_the_workbook_folder() {
        let dir = PathBuf::from(if cfg!(windows) {
            r"C:\plant\books"
        } else {
            "/plant/books"
        });
        let main = dir.join("Main.xlsx");
        let resolver = LinkedResolver::for_workbook(&main, "en", "UTC", "en");
        let book = |target: &str| ExternalBook {
            target: target.to_string(),
            ..Default::default()
        };
        assert_eq!(
            resolver.resolve_target(&book("Linked.xlsx")),
            dir.join("Linked.xlsx")
        );
        assert_eq!(
            resolver.resolve_target(&book("../common/Linked.xlsx")),
            dir.join("../common/Linked.xlsx")
        );
        assert_eq!(
            resolver.resolve_target(&book("https://example.invalid/a/b/Linked%20book.xlsx")),
            dir.join("Linked book.xlsx")
        );
        if cfg!(windows) {
            assert_eq!(
                resolver.resolve_target(&book(r"D:\other\C.xlsx")),
                PathBuf::from(r"D:\other\C.xlsx")
            );
            assert_eq!(
                resolver.resolve_target(&book("file:///D:/other/C%20D.xlsx")),
                PathBuf::from("D:/other/C D.xlsx")
            );
        }
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
        assert_eq!(percent_decode("%ZZx"), "%ZZx");
    }
}
