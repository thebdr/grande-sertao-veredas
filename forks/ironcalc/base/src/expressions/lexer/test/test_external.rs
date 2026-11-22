#![allow(clippy::unwrap_used)]

use crate::language::get_language;
use crate::locale::get_locale;

use crate::expressions::{
    lexer::{Lexer, LexerMode},
    token::TokenType::*,
    types::ParsedReference,
};

fn new_lexer(formula: &str, a1_mode: bool) -> Lexer<'_> {
    let locale = get_locale("en").unwrap();
    let language = get_language("en").unwrap();
    let mode = if a1_mode {
        LexerMode::A1
    } else {
        LexerMode::R1C1
    };
    Lexer::new(formula, mode, locale, language)
}

// `[1]Sheet!A1`: a cell of the first external book
#[test]
fn external_reference_bare() {
    let mut lx = new_lexer("[1]DB_X!$F$88", true);
    assert_eq!(
        lx.next_token(),
        ExternalReference {
            book: 1,
            sheet: "DB_X".to_string(),
            row: 88,
            column: 6,
            absolute_column: true,
            absolute_row: true,
        }
    );
    assert_eq!(lx.next_token(), EOF);

    let mut lx = new_lexer("[12]Data!B4+1", true);
    assert_eq!(
        lx.next_token(),
        ExternalReference {
            book: 12,
            sheet: "Data".to_string(),
            row: 4,
            column: 2,
            absolute_column: false,
            absolute_row: false,
        }
    );
    assert_eq!(
        lx.next_token(),
        Addition(crate::expressions::token::OpSum::Add)
    );
    assert_eq!(lx.next_token(), Number(1.0));
    assert_eq!(lx.next_token(), EOF);
}

// `'[1]Sheet name'!A1`: the book index goes inside the quotes
#[test]
fn external_reference_quoted() {
    let mut lx = new_lexer("'[1]ZONE LIST'!$B$6", true);
    assert_eq!(
        lx.next_token(),
        ExternalReference {
            book: 1,
            sheet: "ZONE LIST".to_string(),
            row: 6,
            column: 2,
            absolute_column: true,
            absolute_row: true,
        }
    );
    assert_eq!(lx.next_token(), EOF);

    // escaped quote inside the name
    let mut lx = new_lexer("'[2]It''s'!A1", true);
    assert_eq!(
        lx.next_token(),
        ExternalReference {
            book: 2,
            sheet: "It's".to_string(),
            row: 1,
            column: 1,
            absolute_column: false,
            absolute_row: false,
        }
    );
}

#[test]
fn external_range() {
    let mut lx = new_lexer("SUM([1]DB_X!A1:$B$4)", true);
    assert_eq!(lx.next_token(), Ident("SUM".to_string()));
    assert_eq!(lx.next_token(), LeftParenthesis);
    assert_eq!(
        lx.next_token(),
        ExternalRange {
            book: 1,
            sheet: "DB_X".to_string(),
            left: ParsedReference {
                row: 1,
                column: 1,
                absolute_column: false,
                absolute_row: false
            },
            right: ParsedReference {
                row: 4,
                column: 2,
                absolute_column: true,
                absolute_row: true
            },
        }
    );
    assert_eq!(lx.next_token(), RightParenthesis);
    assert_eq!(lx.next_token(), EOF);
}

// The internal (R1C1) form carries the same prefix
#[test]
fn external_reference_r1c1() {
    let mut lx = new_lexer("[1]DB_X!R88C6", false);
    assert_eq!(
        lx.next_token(),
        ExternalReference {
            book: 1,
            sheet: "DB_X".to_string(),
            row: 88,
            column: 6,
            absolute_column: true,
            absolute_row: true,
        }
    );
    let mut lx = new_lexer("'[1]DB X'!R[-1]C[2]:R[3]C[2]", false);
    assert_eq!(
        lx.next_token(),
        ExternalRange {
            book: 1,
            sheet: "DB X".to_string(),
            left: ParsedReference {
                row: -1,
                column: 2,
                absolute_column: false,
                absolute_row: false
            },
            right: ParsedReference {
                row: 3,
                column: 2,
                absolute_column: false,
                absolute_row: false
            },
        }
    );
    assert_eq!(lx.next_token(), EOF);
}

// What is not an external reference stays what it was
#[test]
fn plain_brackets_are_untouched() {
    // a structured reference still goes through its own path
    let mut lx = new_lexer("Table1[Column]", true);
    assert!(matches!(lx.next_token(), StructuredReference { .. }));
    // `[` not followed by digits and `]` is a plain bracket
    let mut lx = new_lexer("[A1]", true);
    assert_eq!(lx.next_token(), LeftBracket);
    // `[0]` is not a book
    let mut lx = new_lexer("[0]Sheet!A1", true);
    assert_eq!(lx.next_token(), LeftBracket);
    // digits without the sheet: a plain bracket too
    let mut lx = new_lexer("[1]+1", true);
    assert_eq!(lx.next_token(), LeftBracket);
    assert_eq!(lx.next_token(), Number(1.0));
    // R1C1 offsets are consumed by the reference, never seen here
    let mut lx = new_lexer("R[1]C[1]", false);
    assert!(matches!(lx.next_token(), Reference { .. }));
}

// An external defined name is refused, not misread
#[test]
fn external_defined_names_are_an_error() {
    let mut lx = new_lexer("[1]DB_X!Total", true);
    assert!(matches!(lx.next_token(), Illegal(_)));
    let mut lx = new_lexer("'[1]DB X'!Total", true);
    assert!(matches!(lx.next_token(), Illegal(_)));
    // a reference glued to a name is not a reference either
    let mut lx = new_lexer("[1]DB_X!A1B", true);
    assert!(matches!(lx.next_token(), Illegal(_)));
}
