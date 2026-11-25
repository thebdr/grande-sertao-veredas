#![allow(clippy::unwrap_used)]

//! Veredas fork change 29: a number in a formula is spelled so it reads back to the same number.

use crate::test::util::new_empty_model;

#[test]
fn a_number_its_fifteen_digits_do_not_hold_is_spelled_to_read_back() {
    let mut model = new_empty_model();
    for (typed, spelled) in [
        ("=0.30000000000000004", "=0.30000000000000004"),
        ("=1.0000000000000002", "=1.0000000000000002"),
        ("=1.2345678901234567", "=1.2345678901234567"),
        ("=9007199254740993", "=9007199254740992"),
        ("=1.7976931348623157e308", "=1.7976931348623157e308"),
        // Excel's own spelling where its 15 digits hold the number
        ("=0.1+0.2", "=0.1+0.2"),
        ("=123456789012345", "=123456789012345"),
        ("=5e-324", "=5e-324"),
        ("=1/3", "=1/3"),
    ] {
        model.set_user_input(0, 1, 1, typed.to_string()).unwrap();
        let formula = model.get_cell_formula(0, 1, 1).unwrap().unwrap();
        assert_eq!(formula, spelled, "{typed}");
        model.evaluate();
        let first = model.get_cell_value_by_index(0, 1, 1).unwrap();
        // entered again as spelled: the same number
        model.set_user_input(0, 1, 2, formula.clone()).unwrap();
        model.evaluate();
        assert_eq!(model.get_cell_value_by_index(0, 1, 2).unwrap(), first, "{typed}");
        assert_eq!(model.get_cell_formula(0, 1, 2).unwrap().unwrap(), formula, "{typed}");
    }
}

/// Veredas fork change 30: an operand that binds less than its operator is bracketed, so the
/// spelling reads back as the same formula; an array constant's numbers read back the same.
#[test]
fn a_formula_s_spelling_reads_back_as_the_same_formula() {
    let mut model = new_empty_model();
    for (r, v) in [(1, "3"), (2, "4"), (3, "5")] {
        model.set_user_input(0, r, 26, v.to_string()).unwrap();
    }
    for typed in [
        "=(Z1&Z2)*1",
        "=(2&3)+1",
        "=1-(2&3)",
        "=(Z1+Z2)%",
        "=2*(Z1+Z2)%",
        "=(Z1/Z2)%",
        "=(Z1^2)%",
        "=\"x\"&(Z1>0)",
        "=(Z1>0)&\"x\"",
        "=(Z1<Z2)=(Z2<Z3)",
        "=(1=1)=(1=1)",
        "=Z1+(Z2+Z3)",
        "=0.1+(0.2+0.3)",
        "=Z1-(Z2-Z3)",
        "=Z1*(Z2*Z3)",
        "=-(Z1*Z2)",
        "=SUM({0.30000000000000004})",
        "=INDEX({1.0000000000000002;2},1)-1",
        "=SUM({9007199254740993,1})",
    ] {
        model.set_user_input(0, 1, 1, typed.to_string()).unwrap();
        model.evaluate();
        let first = model.get_cell_value_by_index(0, 1, 1).unwrap();
        let spelled = model.get_cell_formula(0, 1, 1).unwrap().unwrap();
        // entered again as spelled: the same value, the same spelling
        model.set_user_input(0, 1, 2, spelled.clone()).unwrap();
        model.evaluate();
        assert_eq!(model.get_cell_value_by_index(0, 1, 2).unwrap(), first, "{typed} spelled {spelled}");
        assert_eq!(model.get_cell_formula(0, 1, 2).unwrap().unwrap(), spelled, "{typed}");
    }
    // the spellings themselves, where a dropped bracket changed the value
    for (typed, spelled) in [("=(Z1&Z2)*1", "=(Z1&Z2)*1"), ("=(Z1+Z2)%", "=(Z1+Z2)%"), ("=(Z1<Z2)=(Z2<Z3)", "=Z1<Z2=(Z2<Z3)"), ("=SUM({0.30000000000000004})", "=SUM({0.30000000000000004})")] {
        model.set_user_input(0, 1, 1, typed.to_string()).unwrap();
        assert_eq!(model.get_cell_formula(0, 1, 1).unwrap().unwrap(), spelled, "{typed}");
    }
}

/// Veredas fork change 30, refined: `@` and `#` take a primary, so an operation under them is
/// bracketed, and the formula reads back as itself, its value Excel's.
#[test]
fn an_operation_under_an_implicit_intersection_keeps_its_bracket() {
    let mut model = new_empty_model();
    for r in 1..=3 {
        model.set_user_input(0, r, 1, r.to_string()).unwrap();
        model.set_user_input(0, r, 2, (r * 10).to_string()).unwrap();
        model.set_user_input(0, r, 3, (r * 100).to_string()).unwrap();
    }
    for (typed, spelled, value) in [
        ("=@(A1:A3*B1)", "=@(A1:A3*B1)", "10"),
        ("=@(B1:B3*C1)", "=@(B1:B3*C1)", "1000"),
        ("=@A1:A3*B1", "=@A1:A3*B1", "20"),
        ("=@(A1:A3%)", "=@(A1:A3%)", "0.01"),
        // (its value read since fork change 32: `:` takes the cell INDEX names)
        ("=@(A1:INDEX(A1:A3,3))", "=@(A1:INDEX(A1:A3,3))", "2"),
    ] {
        model.set_user_input(0, 2, 6, typed.to_string()).unwrap();
        model.evaluate();
        assert_eq!(model.get_cell_formula(0, 2, 6).unwrap().unwrap(), spelled, "{typed}");
        if !value.is_empty() {
            assert_eq!(model.get_formatted_cell_value(0, 2, 6).unwrap(), value, "{typed}");
        }
        assert_eq!(model.formulas_reading_back_otherwise(), Vec::<(u32, i32, i32)>::new(), "{typed}");
    }
}
