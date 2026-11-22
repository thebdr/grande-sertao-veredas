#![allow(clippy::panic)]

use std::collections::HashMap;

use crate::expressions::lexer::LexerMode;
use crate::expressions::parser::stringify::{to_excel_string, to_rc_format};
use crate::expressions::parser::tests::utils::{new_parser, to_english_localized_string};
use crate::expressions::parser::Node;
use crate::expressions::types::CellReferenceRC;

fn context(row: i32, column: i32) -> CellReferenceRC {
    CellReferenceRC {
        sheet: "Sheet1".to_string(),
        row,
        column,
    }
}

// `[1]Sheet!A1` parses to a node that keeps the book and the sheet as written
#[test]
fn parses_to_external_nodes() {
    let mut parser = new_parser(vec!["Sheet1".to_string()], vec![], HashMap::new());
    let t = parser.parse("[1]DB_X!$F$88", &context(1, 1));
    assert_eq!(
        t,
        Node::ExternalReferenceKind {
            book: 1,
            sheet_name: "DB_X".to_string(),
            absolute_row: true,
            absolute_column: true,
            row: 88,
            column: 6,
        }
    );
    // relative coordinates are offsets from the formula's cell, like ReferenceKind
    let t = parser.parse("[1]DB_X!B4", &context(10, 3));
    assert_eq!(
        t,
        Node::ExternalReferenceKind {
            book: 1,
            sheet_name: "DB_X".to_string(),
            absolute_row: false,
            absolute_column: false,
            row: -6,
            column: -1,
        }
    );
    // a range, corners sorted
    let t = parser.parse("'[2]DB X'!$C$4:$A$1", &context(1, 1));
    assert_eq!(
        t,
        Node::ExternalRangeKind {
            book: 2,
            sheet_name: "DB X".to_string(),
            absolute_row1: true,
            absolute_column1: true,
            row1: 1,
            column1: 1,
            absolute_row2: true,
            absolute_column2: true,
            row2: 4,
            column2: 3,
        }
    );
}

// What the engine stores (R1C1) and what it shows (A1) agree, and the
// A1 form is spelled as Excel spells it
#[test]
fn roundtrip_a1_and_r1c1() {
    let mut parser = new_parser(vec!["Sheet1".to_string()], vec![], HashMap::new());
    let cases = [
        ("[1]DB_X!$F$88", "[1]DB_X!R88C6"),
        ("[1]DB_X!B4", "[1]DB_X!R[3]C[1]"),
        ("'[1]ZONE LIST'!$B$6", "'[1]ZONE LIST'!R6C2"),
        ("'[3]It''s'!A1", "'[3]It''s'!R[0]C[0]"),
        ("SUM([1]DB_X!A1:$B$4)", "SUM([1]DB_X!R[0]C[0]:R4C2)"),
        ("'[1]DB X'!A:A", "'[1]DB X'!R1C[0]:R1048576C[0]"),
        ("[1]DB_X!3:3", "[1]DB_X!R[2]C1:R[2]C16384"),
        (
            "SUBSTITUTE([1]ZONES!$B$6,\"%1\",REPT(\"0\",[1]ZONES!$B$4-LEN($B$7))&$B$7)",
            "SUBSTITUTE([1]ZONES!R6C2,\"%1\",REPT(\"0\",[1]ZONES!R4C2-LEN(R7C2))&R7C2)",
        ),
    ];
    for (a1, rc) in cases {
        let t = parser.parse(a1, &context(1, 1));
        assert!(
            !matches!(t, Node::ParseErrorKind { .. }),
            "{a1} did not parse: {t:?}"
        );
        assert_eq!(to_rc_format(&t), rc, "R1C1 of {a1}");
        assert_eq!(
            to_english_localized_string(&t, &context(1, 1)),
            a1,
            "A1 of {a1}"
        );
        // and the file form is the same text (no _xlfn. prefix is involved)
        assert_eq!(to_excel_string(&t, &context(1, 1)), a1, "xlsx form of {a1}");
        // the stored form parses back to the same node
        parser.set_lexer_mode(LexerMode::R1C1);
        let back = parser.parse(rc, &context(1, 1));
        parser.set_lexer_mode(LexerMode::A1);
        assert_eq!(back, t, "reparse of {rc}");
    }
}

// A shared formula is one R1C1 text for every copy: the A1 spelling of a
// relative external reference follows the cell, an absolute one does not
#[test]
fn relative_references_follow_the_cell() {
    let mut parser = new_parser(vec!["Sheet1".to_string()], vec![], HashMap::new());
    let t = parser.parse("[1]DB_X!B4+[1]DB_X!$B$4", &context(1, 1));
    assert_eq!(to_rc_format(&t), "[1]DB_X!R[3]C[1]+[1]DB_X!R4C2");
    assert_eq!(
        to_english_localized_string(&t, &context(5, 2)),
        "[1]DB_X!C8+[1]DB_X!$B$4"
    );
    // shifted off the sheet: #REF!
    assert_eq!(
        to_english_localized_string(&t, &context(-10, 1)),
        "#REF!+[1]DB_X!$B$4"
    );
}

#[test]
fn external_defined_names_are_parse_errors() {
    let mut parser = new_parser(vec!["Sheet1".to_string()], vec![], HashMap::new());
    let t = parser.parse("[1]DB_X!Total", &context(1, 1));
    assert!(matches!(t, Node::ParseErrorKind { .. }), "{t:?}");
}
