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

