#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

/// Veredas fork change 31: the formulas whose spelling reads back as another formula, found.
#[test]
fn the_formulas_whose_spelling_reads_back_otherwise_are_found() {
    let mut model = new_empty_model();
    model.set_user_input(0, 1, 1, "=(1&2)*1".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "=ROWS({1;2})".to_string()).unwrap();
    model.set_user_input(0, 3, 1, "=0.1+(0.2+0.3)".to_string()).unwrap();
    assert_eq!(model.formulas_reading_back_otherwise(), Vec::<(u32, i32, i32)>::new());
    // in a locale whose array rows are spelled `/`, which the lexer does not read
    model.set_locale("de").unwrap();
    assert_eq!(model.formulas_reading_back_otherwise(), vec![(0, 2, 1)]);
}

/// Veredas fork change 32: an INDEX beside the range operator is the reference it names.
#[test]
fn an_index_in_a_range_is_the_reference_it_names() {
    let mut model = new_empty_model();
    for r in 1..=3 {
        model.set_user_input(0, r, 1, r.to_string()).unwrap();
    }
    for (row, typed, value) in [
        (1, "=SUM(A1:INDEX(A1:A3,3))", "6"),
        (1, "=SUM(INDEX(A1:A3,2):A3)", "5"),
        (1, "=ROWS(A1:INDEX(A1:A3,2))", "2"),
        (2, "=@(A1:INDEX(A1:A3,3))", "2"),
        (1, "=SUM(INDEX(A1:A3,1):INDEX(A1:A3,2))", "3"),
        (1, "=SUM(A1:INDEX(A1:A3,4))", "#REF!"),
        // INDEX alone is the value it names, as before
        (1, "=INDEX(A1:A3,3)", "3"),
    ] {
        model.set_user_input(0, row, 6, typed.to_string()).unwrap();
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, row, 6).unwrap(), value, "{typed}");
        model.set_user_input(0, row, 6, String::new()).unwrap();
    }
}

/// Veredas fork change 32, refined: the range operator joins two references into the smallest area
/// holding both, in any order, of any size.
#[test]
fn a_range_joins_its_sides_into_the_area_holding_both() {
    let mut model = new_empty_model();
    for r in 1..=3 {
        model.set_user_input(0, r, 1, r.to_string()).unwrap();
        model.set_user_input(0, r, 2, (20 + r).to_string()).unwrap();
        model.set_user_input(0, r, 3, (30 + r).to_string()).unwrap();
    }
    for (typed, value) in [
        ("=SUM(INDEX(A1:A3,3):A1)", "6"),
        ("=ROWS(INDEX(A1:A3,3):A1)", "3"),
        ("=COUNT(INDEX(A1:A3,3):A1)", "3"),
        ("=MATCH(2,INDEX(A1:A3,3):A1,0)", "2"),
        ("=SUM(A1:INDEX(A1:C3,0,2))", "72"),
        ("=SUM(A1:INDEX(A1:C3,2,0))", "109"),
        ("=SUM(INDEX(A1:C3,0,2):C3)", "162"),
        ("=COLUMNS(INDEX(A1:C3,1,3):A1)", "3"),
    ] {
        model.set_user_input(0, 5, 6, typed.to_string()).unwrap();
        model.evaluate();
        assert_eq!(model.get_formatted_cell_value(0, 5, 6).unwrap(), value, "{typed}");
    }
}

/// Veredas fork change 31, refined: a formula spelled and checked in the reader's language and
/// locale, not the model's.
#[test]
fn the_reader_s_language_and_locale_are_the_check_s() {
    let mut model = new_empty_model();
    for r in 1..=3 {
        model.set_user_input(0, r, 1, r.to_string()).unwrap();
    }
    model.set_user_input(0, 6, 3, "=SUM(A1:A3)".to_string()).unwrap();
    // the model in Spanish: spelled for an English reader, and checked as one reads it
    model.set_language("es").unwrap();
    assert_eq!(model.get_cell_formula(0, 6, 3).unwrap().unwrap(), "=SUMA(A1:A3)");
    assert_eq!(model.get_cell_formula_in(0, 6, 3, "en", "en").unwrap().unwrap(), "=SUM(A1:A3)");
    assert_eq!(model.formulas_reading_back_otherwise_in("en", "en").unwrap(), Vec::<(u32, i32, i32)>::new());
    // for a reader of another locale than the model's, spelled in that locale (its decimal comma,
    // its separators), which it reads back
    model.set_language("en").unwrap();
    model.set_user_input(0, 9, 3, "=SUM(1.5,2)".to_string()).unwrap();
    assert_eq!(model.get_cell_formula_in(0, 9, 3, "de", "en").unwrap().unwrap(), "=SUM(1,5;2)");
    assert_eq!(model.formulas_reading_back_otherwise_in("de", "en").unwrap(), Vec::<(u32, i32, i32)>::new());
    assert!(model.formulas_reading_back_otherwise_in("xx", "en").is_err());
}
