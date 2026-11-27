#![allow(clippy::unwrap_used)]

use crate::model::DefinedNameEntry;
use crate::test::util::new_empty_model;

#[test]
fn simple_evaluation() {
    let mut model = new_empty_model();
    model._set("A1", "=LAMBDA(x, y, x + y)(1, 2)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"3");
}

#[test]
fn evaluation_in_let() {
    let mut model = new_empty_model();
    model._set("A1", "=LET(x, 1, y, LAMBDA(a, b, a + b), y(x, 22))");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"23");
}

#[test]
fn evaluation_with_defined_name() {
    let mut model = new_empty_model();
    model
        .new_defined_name(DefinedNameEntry::new("MySum", "=LAMBDA(x, y, x + y)"))
        .unwrap();
    model._set("A1", "=MySum(1, 2)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"3");
}

// Regression: a defined-name LAMBDA that returns an array must spill.
// Calling it (a `NamedFunctionKind`) was statically classified as a scalar,
// so the cell was never marked dynamic and the array result reached a
// scalar context instead of spilling.
#[test]
fn defined_name_lambda_spills_array() {
    let mut model = new_empty_model();
    model
        .new_defined_name(DefinedNameEntry::new("MySeq", "=LAMBDA(n, SEQUENCE(n))"))
        .unwrap();
    model._set("A1", "=MySeq(3)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"2");
    assert_eq!(model._get_text("A3"), *"3");
}

#[test]
fn wrong_number_of_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=LAMBDA(x, y, x + y)(1)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#VALUE!");
}

#[test]
fn returns_calculation_error() {
    let mut model = new_empty_model();
    model._set("A1", "=LAMBDA(x, y, x + y)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#CALC!");
}

#[test]
fn sheet_local_name_takes_precedence_over_global() {
    // Global MyFn = LAMBDA(x, x * 2)
    // Sheet2-local MyFn = LAMBDA(x, x * 3)
    // Sheet1!A1 calls MyFn(5) => uses global => 10
    // Sheet2!A1 calls MyFn(5) => uses local  => 15
    let mut model = new_empty_model();
    model.new_sheet(); // adds Sheet2 at index 1

    model
        .new_defined_name(DefinedNameEntry::new("MyFn", "=LAMBDA(x, x * 2)"))
        .unwrap();
    model
        .new_defined_name(DefinedNameEntry::new("MyFn", "=LAMBDA(x, x * 3)").scope(1))
        .unwrap();

    model._set("A1", "=MyFn(5)");
    model._set("Sheet2!A1", "=MyFn(5)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"10");
    assert_eq!(model._get_text("Sheet2!A1"), *"15");
}

#[test]
fn let_bound_variable_captured_in_lambda_body() {
    let mut model = new_empty_model();
    // `a` is bound by LET; the LAMBDA body references `a` as a closure variable.
    // f(2) should return 2 + 1 = 3.
    model._set("A1", "=LET(a, 1, f, LAMBDA(x, x + a), f(2))");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"3");
}

#[test]
fn optional_parameter_omitted() {
    let mut model = new_empty_model();
    // b is optional; calling with one arg should use default (a/2 = 6).
    model._set("A1", "=LAMBDA(a, [b], IF(ISOMITTED(b), a/2, a*b))(12)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"6");
}

#[test]
fn optional_parameter_provided() {
    let mut model = new_empty_model();
    model._set("A1", "=LAMBDA(a, [b], IF(ISOMITTED(b), a/2, a*b))(12, 3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"36");
}

#[test]
fn optional_parameter_too_many_args_is_error() {
    let mut model = new_empty_model();
    model._set("A1", "=LAMBDA(a, [b], a+1)(1, 2, 3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
}

#[test]
fn required_arg_missing_is_error() {
    let mut model = new_empty_model();
    // `a` is required; calling with no args is an error.
    model._set("A1", "=LAMBDA(a, [b], a+1)()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
}

// A reference given to a LAMBDA, or bound by a LET, stays a reference, also
// when it is a single cell. Where a value is wanted it is the cell's value.
#[test]
fn a_single_cell_is_passed_as_a_reference() {
    let mut model = new_empty_model();
    model._set("A1", "text");
    model._set("A2", "7");
    model._set("B5", "3");
    model._set("C1", "=LAMBDA(x, MAX(x))(A1)");
    model._set("C2", "=LAMBDA(x, MAX(x))(A2)");
    model._set("C3", "=LAMBDA(x, ROW(x))(B5)");
    model._set("C4", "=LAMBDA(x, COLUMN(x))(B5)");
    model._set("C5", "=LAMBDA(x, OFFSET(x, 1, 0))(A1)");
    model._set("C6", "=LAMBDA(x, ROWS(x))(B5)");
    model._set("C9", "=LET(x, A1, MAX(x))");
    model._set("C10", "=LET(x, B5, ROW(x))");
    model._set("C11", "=LET(x, A2, y, B5, OFFSET(x, 0, 0)+y)");
    model.evaluate();

    assert_eq!(model._get_text("C1"), *"0");
    assert_eq!(model._get_text("C2"), *"7");
    assert_eq!(model._get_text("C3"), *"5");
    assert_eq!(model._get_text("C4"), *"2");
    assert_eq!(model._get_text("C5"), *"7");
    assert_eq!(model._get_text("C6"), *"1");
    assert_eq!(model._get_text("C9"), *"0");
    assert_eq!(model._get_text("C10"), *"5");
    assert_eq!(model._get_text("C11"), *"10");
}

#[test]
fn a_single_cell_parameter_is_a_value_where_a_value_is_wanted() {
    let mut model = new_empty_model();
    model._set("A1", "text");
    model._set("A2", "7");
    model._set("C1", "=LAMBDA(x, x)(A1)");
    model._set("C2", "=LAMBDA(x, x)(A2)");
    model._set("C3", "=LAMBDA(x, x&\"!\")(A1)");
    model._set("C4", "=LAMBDA(x, x+1)(A2)");
    model._set("C5", "=LAMBDA(x, IF(x=\"text\", 1, 2))(A1)");
    model._set("C6", "=LAMBDA(x, ISBLANK(x))(A3)");
    model._set("C7", "=LAMBDA(x, ISTEXT(x))(A1)");
    model._set("C8", "=LAMBDA(x, LEN(x))(A1)");
    model._set("C9", "=LET(x, A2, x*2)");
    model._set("C10", "=LAMBDA(x, TYPE(x))(A2)");
    model.evaluate();

    assert_eq!(model._get_text("C1"), *"text");
    assert_eq!(model._get_text("C2"), *"7");
    assert_eq!(model._get_text("C3"), *"text!");
    assert_eq!(model._get_text("C4"), *"8");
    assert_eq!(model._get_text("C5"), *"1");
    assert_eq!(model._get_text("C6"), *"TRUE");
    assert_eq!(model._get_text("C7"), *"TRUE");
    assert_eq!(model._get_text("C8"), *"4");
    assert_eq!(model._get_text("C9"), *"14");
    assert_eq!(model._get_text("C10"), *"1");
    // Nothing spills from a single cell
    assert_eq!(model._get_text("D1"), *"");
    assert_eq!(model._get_text("C11"), *"");
}

// A defined name that stands for a cell is a reference wherever one is
// wanted, and the cell's value wherever a value is wanted
#[test]
fn a_defined_name_for_a_single_cell_is_a_reference() {
    let mut model = new_empty_model();
    model._set("K1", "text");
    model._set("K2", "7");
    model
        .new_defined_name(DefinedNameEntry::new("TextCell", "Sheet1!$K$1"))
        .unwrap();
    model
        .new_defined_name(DefinedNameEntry::new("Later", "=LAMBDA(x, x+1)"))
        .unwrap();
    model._set("A1", "=ROW(TextCell)");
    model._set("A2", "=COLUMN(TextCell)");
    model._set("A3", "=OFFSET(TextCell, 1, 0)");
    model._set("A4", "=ROWS(TextCell)");
    model._set("A5", "=TextCell&\"!\"");
    model._set("A6", "=LEN(TextCell)");
    model._set("A7", "=IF(TextCell=\"text\", 1, 2)");
    model._set("A8", "=TextCell");
    model._set("A9", "=Later(1)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"11");
    assert_eq!(model._get_text("A3"), *"7");
    assert_eq!(model._get_text("A4"), *"1");
    assert_eq!(model._get_text("A5"), *"text!");
    assert_eq!(model._get_text("A6"), *"4");
    assert_eq!(model._get_text("A7"), *"1");
    assert_eq!(model._get_text("A8"), *"text");
    assert_eq!(model._get_text("A9"), *"2");
}

// Veredas fork change 44: LAMBDA calls and defined names' formulas nested
// deeper than the evaluation's bound are `#NUM!` (whose message is
// `TOO_DEEP`), and on record, instead of overflowing the stack of a thread as
// small as a test's and aborting the process.
#[test]
fn a_recursion_too_deep_is_num_and_overflows_no_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut model = new_empty_model();
            for (name, formula) in [
                ("Soma", "=LAMBDA(n,IF(n<=0,0,n+Soma(n-1)))"),
                ("Aa", "=Aa+1"),
                ("Bb", "=1"),
                ("Cc", "=Bb+1"),
            ] {
                model
                    .new_defined_name(DefinedNameEntry::new(name, formula))
                    .unwrap();
            }
            // two names reading each other
            model
                .update_defined_name("Bb", None, DefinedNameEntry::new("Bb", "=Cc+1"))
                .unwrap();
            model._set("A1", "=Soma(5)");
            model._set("A2", "=Soma(5000)");
            model._set("A3", "=Aa");
            model._set("A4", "=Bb");
            model._set("A5", "=A2+1");
            model._set("A6", "=IFERROR(Soma(5000),-1)");
            model._set("A7", "=SUM(Aa)");
            model._set("A8", "=MAP(SEQUENCE(1),Soma)");
            model._set("A9", "=Soma(5000)/0");
            model.evaluate();
            assert_eq!(model._get_text("A1"), *"15");
            for cell in ["A2", "A3", "A4", "A5", "A7", "A9"] {
                assert_eq!(model._get_text(cell), *"#NUM!", "{cell}");
            }
            assert_eq!(model._get_text("A6"), *"-1");
            assert_eq!(model._get_text("A8"), *"1");
            assert_eq!(
                model.too_deep_cells(),
                vec![(0, 2, 1), (0, 3, 1), (0, 4, 1), (0, 5, 1), (0, 6, 1), (0, 7, 1), (0, 9, 1)]
            );
            // the next evaluation says it again, of the cells it leaves so
            model._set("A2", "=Soma(3)");
            model.evaluate();
            assert_eq!(model._get_text("A2"), *"6");
            assert_eq!(model._get_text("A5"), *"7");
            assert_eq!(
                model.too_deep_cells(),
                vec![(0, 3, 1), (0, 4, 1), (0, 6, 1), (0, 7, 1), (0, 9, 1)]
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

// Veredas fork change 44, refined: a cell's LAMBDA calls and names are
// counted from its own run, so what it computes does not turn on which cell
// reads it first or through how many cells (the stack of the readers' runs
// counted against its budget: `Soma(15)` was `#NUM!` behind a reader in a
// debug build); a conditional format's rule going too deep puts no cell on
// record.
#[test]
fn a_recursion_computes_the_same_whoever_reads_its_cell_first() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let soma = DefinedNameEntry::new("Soma", "=LAMBDA(n,IF(n<=0,0,n+Soma(n-1)))");
            // a chain of k cells, each reading the next through a name or through a
            // LAMBDA's body, the last computing `Soma(15)`
            for through_a_name in [true, false] {
                for k in [1usize, 8, 16, 32, 48, 64, 100] {
                    let mut model = new_empty_model();
                    model.new_defined_name(soma.clone()).unwrap();
                    for i in 1..=k {
                        let next = format!("Sheet1!$A${}", i + 1);
                        if through_a_name {
                            model
                                .new_defined_name(DefinedNameEntry::new(&format!("Nome_{i}"), &format!("={next}+0")))
                                .unwrap();
                            model._set(&format!("A{i}"), &format!("=Nome_{i}"));
                        } else {
                            model._set(&format!("A{i}"), &format!("=LAMBDA(x,x+{next})(0)"));
                        }
                    }
                    model._set(&format!("A{}", k + 1), "=Soma(15)");
                    model.evaluate();
                    for i in 1..=k + 1 {
                        assert_eq!(model._get_text(&format!("A{i}")), *"120", "{through_a_name} {k}: A{i}");
                    }
                    assert!(model.too_deep_cells().is_empty(), "{through_a_name} {k}");
                }
            }
            // a reader before the cell and one after it
            let mut model = new_empty_model();
            model.new_defined_name(soma).unwrap();
            model
                .new_defined_name(DefinedNameEntry::new("Rd", "=Sheet1!$B$1+0"))
                .unwrap();
            model._set("A1", "=Rd");
            model._set("B1", "=Soma(15)");
            model._set("C1", "=Rd");
            // a conditional format's rule too deep, over empty cells
            model
                .add_conditional_formatting(
                    0,
                    "D1:D2",
                    crate::cf_types::CfRuleInput::Formula {
                        formula: "=Soma(5000)>0".to_string(),
                        format: crate::types::Dxf::default(),
                        stop_if_true: false,
                    },
                )
                .unwrap();
            model.evaluate();
            for cell in ["A1", "B1", "C1"] {
                assert_eq!(model._get_text(cell), *"120", "{cell}");
            }
            assert_eq!(model.too_deep_cells(), Vec::<(u32, i32, i32)>::new());
        })
        .unwrap()
        .join()
        .unwrap();
}
