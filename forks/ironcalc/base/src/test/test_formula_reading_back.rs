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

/// Veredas fork change 33: a named function's name and a range's operands read back as themselves,
/// and LET's and LAMBDA's names bind in any case.
#[test]
fn a_named_function_and_a_range_read_back_and_names_bind_in_any_case() {
    let mut model = new_empty_model();
    for r in 1..=3 {
        model.set_user_input(0, r, 1, r.to_string()).unwrap();
    }
    model.new_defined_name(crate::DefinedNameEntry::new("Fn", "LAMBDA(x,x*2)")).unwrap();
    for (row, typed) in [
        (1, "=Fn(3)"),
        (2, "=FN(3)"),
        (3, "=LET(F,LAMBDA(x,x*3),F(2))"),
        (4, "=MYUDF(1)"),
        (5, "=GETPIVOTDATA(\"x\",A1)"),
        (6, "=SUM(A1:(A2:A3))"),
        (7, "=SUM(A1:(A3))"),
        (8, "=SUM((A1):A3)"),
        (10, "=LET(x,1,X+1)"),
        (11, "=LAMBDA(a,A*2)(4)"),
        (12, "=LET(ss,1,ß+1)"),
    ] {
        model.set_user_input(0, row, 3, typed.to_string()).unwrap();
    }
    model.evaluate();
    assert_eq!(model.formulas_reading_back_otherwise(), Vec::<(u32, i32, i32)>::new());
    for (row, value) in [(1, "6"), (2, "6"), (3, "6"), (6, "6"), (7, "6"), (8, "6"), (10, "2"), (11, "8"), (12, "2")] {
        assert_eq!(model.get_formatted_cell_value(0, row, 3).unwrap(), value, "row {row}");
    }
    assert_eq!(model.get_cell_formula(0, 6, 3).unwrap().unwrap(), "=SUM(A1:(A2:A3))");
    assert_eq!(model.get_cell_formula(0, 1, 3).unwrap().unwrap(), "=fn(3)");
}

/// Veredas fork change 34: a bound name is written with `_xlpm.` in any case, as
/// evaluation binds it, so Excel reads the LET's and the LAMBDA's own names.
#[test]
fn a_bound_name_is_exported_with_its_prefix_in_any_case() {
    use crate::expressions::parser::stringify::to_excel_string;
    use crate::expressions::types::CellReferenceRC;
    let mut model = new_empty_model();
    let cell = CellReferenceRC { sheet: "Sheet1".to_string(), row: 1, column: 1 };
    for (typed, exported) in [
        ("LET(F,LAMBDA(y,y*2),F(3))", "_xlfn.LET(_xlpm.F,_xlfn.LAMBDA(_xlpm.y,_xlpm.y*2),_xlpm.f(3))"),
        ("LET(x,1,X+1)", "_xlfn.LET(_xlpm.x,1,_xlpm.X+1)"),
        ("LAMBDA(G,G(2))(LAMBDA(z,z+1))", "_xlfn.LAMBDA(_xlpm.G,_xlpm.g(2))(_xlfn.LAMBDA(_xlpm.z,_xlpm.z+1))"),
        ("LAMBDA(n,N*2)", "_xlfn.LAMBDA(_xlpm.n,_xlpm.N*2)"),
        // `ß` folds to `ss` the way a defined name does (change 34)
        ("LET(ss,1,ß+1)", "_xlfn.LET(_xlpm.ss,1,_xlpm.ß+1)"),
    ] {
        let node = model.parser.parse(typed, &cell);
        assert_eq!(to_excel_string(&node, &cell), exported, "{typed}");
    }
}

/// Veredas fork change 34: a defined name's identity is its case folded both ways, in the name
/// table evaluation reads, the parser's and every check: two names evaluation could not tell apart
/// are one.
#[test]
fn a_defined_name_is_one_whatever_case_folds_to_it() {
    use crate::DefinedNameEntry;
    let mut model = new_empty_model();
    model.set_user_input(0, 1, 1, "2".to_string()).unwrap();
    // a name opening with the Kelvin sign comes from a file (the engine's own API takes an ASCII
    // letter first): held as a file holds it
    model.workbook.defined_names.push(crate::types::DefinedName {
        name: "\u{212A}x".to_string(),
        formula: "Sheet1!$A$1".to_string(),
        sheet_id: None,
        hidden: false,
        comment: String::new(),
    });
    model.reset_parsed_structures();
    // the Kelvin sign's `K` folds to `k`, `ß` to `ss`: each the name already there
    assert!(model.new_defined_name(DefinedNameEntry::new("kx", "Sheet1!$A$2")).is_err());
    model.new_defined_name(DefinedNameEntry::new("Maß", "Sheet1!$A$1")).unwrap();
    assert!(model.new_defined_name(DefinedNameEntry::new("MASS", "Sheet1!$A$2")).is_err());
    model.set_user_input(0, 2, 1, "=kx*3".to_string()).unwrap();
    model.set_user_input(0, 3, 1, "=mass+1".to_string()).unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "6");
    assert_eq!(model.get_formatted_cell_value(0, 3, 1).unwrap(), "3");
}

/// Veredas fork change 34: of two names a file holds that fold to one, the first is the name the
/// parser and the evaluator read and the one deleted.
#[test]
fn of_two_names_a_file_holds_that_fold_to_one_the_first_is_read_and_deleted() {
    let mut model = new_empty_model();
    model.set_user_input(0, 1, 1, "2".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "7".to_string()).unwrap();
    for (name, formula) in [("\u{212A}x", "Sheet1!$A$1"), ("kx", "Sheet1!$A$2")] {
        model.workbook.defined_names.push(crate::types::DefinedName {
            name: name.to_string(),
            formula: formula.to_string(),
            sheet_id: None,
            hidden: false,
            comment: String::new(),
        });
    }
    model.reset_parsed_structures();
    model.set_user_input(0, 3, 1, "=kx*3".to_string()).unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 3, 1).unwrap(), "6");
    model.delete_defined_name("kx", None).unwrap();
    let left: Vec<&str> = model.workbook.defined_names.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(left, ["kx"]);
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 3, 1).unwrap(), "21");
}
