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

