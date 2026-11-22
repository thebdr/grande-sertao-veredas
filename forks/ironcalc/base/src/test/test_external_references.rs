#![allow(clippy::unwrap_used)]

//! `[1]Sheet!A1`: cells of other workbooks, read through the model's
//! external resolver.

use std::collections::HashMap;

use crate::expressions::token::Error;
use crate::external::{
    CachedResolver, ExternalBook, ExternalCellRef, ExternalLookup, ExternalSheet, ExternalValue,
    HostResolver,
};
use crate::test::util::new_empty_model;

fn linked_book() -> ExternalBook {
    let mut zone = HashMap::new();
    zone.insert((4, 2), ExternalValue::Number(2.0));
    zone.insert((5, 2), ExternalValue::Number(2.0));
    zone.insert(
        (6, 2),
        ExternalValue::String("L1.SEG%1.ZONE%2.STATE".to_string()),
    );
    zone.insert((7, 2), ExternalValue::Boolean(true));
    zone.insert((8, 2), ExternalValue::Error(Error::NA));
    for row in 1..=3 {
        zone.insert((row, 4), ExternalValue::Number(row as f64 * 10.0));
    }
    ExternalBook {
        target: "Linked_02.xlsx".to_string(),
        absolute_url: None,
        sheets: vec![
            ExternalSheet {
                name: "Cover".to_string(),
                cells: HashMap::new(),
            },
            ExternalSheet {
                name: "ZONES".to_string(),
                cells: zone,
            },
        ],
        defined_names: vec![],
    }
}

#[test]
fn cached_values_evaluate_like_excel_with_the_book_closed() {
    let mut model = new_empty_model();
    model.workbook.external_books.push(linked_book());
    model._set("A1", "=[1]ZONES!$B$4");
    model._set("A2", "=[1]ZONES!$B$6");
    model._set("A3", "=[1]ZONES!$B$7");
    model._set("A4", "=[1]ZONES!$B$8");
    // an empty cell of the linked book is 0, as a plain empty reference is
    model._set("A5", "=[1]ZONES!$F$88");
    model._set("A6", "=[1]ZONES!$F$88&\"x\"");
    // a sheet the cache does not know, and a book that does not exist
    model._set("A7", "=[1]Nope!$A$1");
    model._set("A8", "=[2]ZONES!$A$1");
    // a formula of a linking workbook's shape
    model._set("B7", "3");
    model._set("C15", "4");
    model._set(
        "A9",
        "=SUBSTITUTE(SUBSTITUTE([1]ZONES!$B$6,\"%1\",REPT(\"0\",[1]ZONES!$B$4-LEN($B$7))&$B$7),\"%2\",REPT(\"0\",[1]ZONES!$B$5-LEN(C15+1))&C15+1)",
    );
    // a range of the linked book
    model._set("A10", "=SUM([1]ZONES!$D$1:$D$3)");
    model._set("A11", "=INDEX([1]ZONES!$D$1:$D$3,2)");
    model.evaluate();

    assert_eq!(model.external_strategy(), "cached");
    assert_eq!(model._get_text("A1"), "2");
    assert_eq!(model._get_text("A2"), "L1.SEG%1.ZONE%2.STATE");
    assert_eq!(model._get_text("A3"), "TRUE");
    assert_eq!(model._get_text("A4"), "#N/A");
    assert_eq!(model._get_text("A5"), "0");
    assert_eq!(model._get_text("A6"), "x");
    assert_eq!(model._get_text("A7"), "#REF!");
    assert_eq!(model._get_text("A8"), "#REF!");
    assert_eq!(model._get_text("A9"), "L1.SEG03.ZONE05.STATE");
    assert_eq!(model._get_text("A10"), "60");
    assert_eq!(model._get_text("A11"), "20");

    // the formulas read back as written, the file spelling included
    assert_eq!(model._get_formula("A1"), "=[1]ZONES!$B$4");
    assert_eq!(model._get_formula("A10"), "=SUM([1]ZONES!$D$1:$D$3)");
}

#[test]
fn quoted_sheet_and_relative_references() {
    let mut model = new_empty_model();
    let mut book = linked_book();
    book.sheets[1].name = "ZONE LIST".to_string();
    model.workbook.external_books.push(book);
    model._set("C3", "='[1]ZONE LIST'!B4");
    // the same formula two rows down reads two rows down
    model._set("C5", "='[1]ZONE LIST'!B6");
    model.evaluate();
    assert_eq!(model._get_text("C3"), "2");
    assert_eq!(model._get_text("C5"), "L1.SEG%1.ZONE%2.STATE");
    assert_eq!(model._get_formula("C3"), "='[1]ZONE LIST'!B4");
    assert_eq!(model._get_formula("C5"), "='[1]ZONE LIST'!B6");
    // both cells share one stored formula
    let ws = model.workbook.worksheet(0).unwrap();
    assert_eq!(ws.shared_formulas.len(), 1);
    assert_eq!(ws.shared_formulas[0], "'[1]ZONE LIST'!R[1]C[-1]");
}

#[test]
fn without_books_every_external_cell_is_ref_not_error() {
    let mut model = new_empty_model();
    model._set("A1", "=[1]DB_X!$F$88");
    model._set("A2", "=IFERROR([1]DB_X!$F$88,\"missing\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#REF!");
    assert_eq!(model._get_text("A2"), "missing");
    // the formula is kept, not degraded
    assert_eq!(model._get_formula("A1"), "=[1]DB_X!$F$88");
}

#[test]
fn a_host_resolver_answers_cell_by_cell() {
    let mut model = new_empty_model();
    model.workbook.external_books.push(linked_book());
    model._set("A1", "=[1]ZONES!$B$4*10");
    model._set("A2", "=[1]Cover!$A$1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");

    // the host sees the book it is asked about and answers what it likes
    model.set_external_resolver(Box::new(HostResolver::new(Box::new(
        |book, sheet, row, column| {
            assert_eq!(book.target, "Linked_02.xlsx");
            if sheet == "ZONES" && (row, column) == (4, 2) {
                ExternalLookup::Value(ExternalValue::Number(7.0))
            } else {
                ExternalLookup::NotAvailable
            }
        },
    ))));
    assert_eq!(model.external_strategy(), "host");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "70");
    assert_eq!(model._get_text("A2"), "#REF!");

    // a host that is not there resolves nothing
    model.set_external_resolver(Box::new(HostResolver::not_available()));
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#REF!");

    // and back to the cache
    model.set_external_resolver(Box::new(CachedResolver));
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");
    assert_eq!(model._get_text("A2"), "0");
}

#[test]
fn external_references_survive_sheet_operations() {
    let mut model = new_empty_model();
    model.workbook.external_books.push(linked_book());
    model._set("B2", "=[1]ZONES!$B$4+[1]ZONES!B4");
    // renaming a local sheet that happens to share the name leaves the
    // external one alone
    model.add_sheet("ZONES").unwrap();
    model.rename_sheet_by_index(1, "Renamed").unwrap();
    assert_eq!(
        model._get_formula("B2"),
        "=[1]ZONES!$B$4+[1]ZONES!B4"
    );
    // inserting rows above the formula moves the formula, not the cells it
    // reads in the other workbook: both keep their address, the relative one
    // too, as in Excel (fork change 19; change 1 kept its offset instead)
    model.insert_rows(0, 1, 1).unwrap();
    assert_eq!(
        model._get_formula("B3"),
        "=[1]ZONES!$B$4+[1]ZONES!B4"
    );
    model.evaluate();
    assert_eq!(model._get_text("B3"), "4");
    // and deleting rows above it: a relative reference near the top of the
    // other workbook's sheet is not pushed off it
    model._set("C3", "=[1]ZONES!D1+[1]ZONES!D1:D3");
    model.delete_rows(0, 1, 2).unwrap();
    assert_eq!(
        model._get_formula("B1"),
        "=[1]ZONES!$B$4+[1]ZONES!B4"
    );
    assert_eq!(model._get_formula("C1"), "=[1]ZONES!D1+[1]ZONES!D1:D3");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "4");
}

#[test]
fn an_external_range_too_large_is_refused() {
    let mut model = new_empty_model();
    model.workbook.external_books.push(linked_book());
    model._set("A1", "=SUM([1]ZONES!A:D)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#N/IMPL!");
}

#[test]
fn the_model_lists_every_external_cell_its_formulas_read() {
    let mut model = new_empty_model();
    model.workbook.external_books.push(linked_book());
    // absolute, relative (from C3: B4 is one row down, one column left),
    // a range, a nested one inside a function, and one repeated
    model._set("C3", "=[1]ZONES!$B$4+[1]ZONES!B4");
    model._set("C4", "=SUM([1]ZONES!$D$1:$D$3)+LEN([1]Cover!$A$1)");
    model._set("C5", "=[1]ZONES!$B$4");
    // a local formula contributes nothing
    model._set("C6", "=C3+C4");
    let refs: Vec<ExternalCellRef> = model.external_references().into_iter().collect();
    let r = |sheet: &str, row: i32, column: i32| ExternalCellRef {
        book: 1,
        sheet: sheet.to_string(),
        row,
        column,
    };
    assert_eq!(
        refs,
        vec![
            r("Cover", 1, 1),
            r("ZONES", 1, 4),
            r("ZONES", 2, 4),
            r("ZONES", 3, 4),
            r("ZONES", 4, 2),
        ]
    );
    // and the current strategy answers for one of them
    assert_eq!(
        model.resolve_external(1, "ZONES", 4, 2),
        ExternalLookup::Value(ExternalValue::Number(2.0))
    );
    assert_eq!(
        model.resolve_external(1, "ZONES", 88, 6),
        ExternalLookup::Empty
    );
    assert_eq!(
        model.resolve_external(2, "ZONES", 4, 2),
        ExternalLookup::NotAvailable
    );
}
