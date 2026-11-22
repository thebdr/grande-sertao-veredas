#![allow(clippy::unwrap_used)]

//! TEXT() on text that reads as a number formats the number, as Excel does:
//! a signal list's `TEXT(F179,"0000")` where F179 holds the string "25" gives
//! "0025", not "25".

use crate::test::util::new_empty_model;

#[test]
fn numeric_text_is_formatted_as_a_number() {
    let mut model = new_empty_model();
    // a text cell holding digits
    model._set("A1", "'25");
    model._set("B1", "=TEXT(A1,\"0000\")");
    model._set("B2", "=TEXT(\"25\",\"0000\")");
    model._set("B3", "=TEXT(\"1234.5\",\"#,##0.00\")");
    model._set("B4", "=TEXT(\"-7\",\"0000\")");
    model._set("B5", "=TEXT(\"1e2\",\"0\")");
    // text that is not a number comes back as it is
    model._set("B6", "=TEXT(\"abc\",\"0000\")");
    model._set("B7", "=TEXT(\"\",\"0000\")");
    // numbers and empties as before
    model._set("B8", "=TEXT(25,\"0000\")");
    model._set("B9", "=TEXT(C9,\"0000\")");
    // a device name built from a number held as text
    model._set("D1", "'25");
    model._set("E1", "'.LINE.M01");
    model._set("F1", "'.DRV1");
    model._set("G1", "'-T10001");
    model._set(
        "H1",
        "=LOWER(\"N\"&TEXT(D1,\"0000\")&\"-\"&SUBSTITUTE(MID(E1,2,20)&F1,\".\",\"-\"))&G1",
    );
    // the array form
    model._set("A10", "'7");
    model._set("A11", "x");
    model._set("A12", "8");
    model._set("B10", "=TEXT(A10:A12,\"000\")");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "0025");
    assert_eq!(model._get_text("B2"), "0025");
    assert_eq!(model._get_text("B3"), "1,234.50");
    assert_eq!(model._get_text("B4"), "-0007");
    assert_eq!(model._get_text("B5"), "100");
    assert_eq!(model._get_text("B6"), "abc");
    assert_eq!(model._get_text("B7"), "");
    assert_eq!(model._get_text("B8"), "0025");
    assert_eq!(model._get_text("B9"), "0000");
    assert_eq!(model._get_text("H1"), "n0025-line-m01-drv1-T10001");
    assert_eq!(model._get_text("B10"), "007");
    assert_eq!(model._get_text("B11"), "x");
    assert_eq!(model._get_text("B12"), "008");
}
