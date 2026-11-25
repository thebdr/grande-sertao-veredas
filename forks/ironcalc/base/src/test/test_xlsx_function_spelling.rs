//! How a function is spelled in a file (fork change 25): `Function::to_xlsx_string` against what
//! Microsoft's grammar, XlsxWriter and Excel's own files say. The xlsx crate's
//! `excel_function_spelling` test holds the same list to every Excel-saved workbook of its tests.

use crate::functions::Function;
use crate::language::get_default_language;

/// MS-XLSX 2.2.2, the `future-function-list` production (revision of 2026-11-18): the names
/// inside its `"_xlfn." (...)` group, in the grammar's order.
const MS_XLSX_FUTURE: &[&str] = &[
    "AGGREGATE", "ACOT", "ACOTH", "ARABIC", "BASE", "BETA.DIST", "BETA.INV", "BINOM.DIST",
    "BINOM.DIST.RANGE", "BINOM.INV", "BITAND", "BITLSHIFT", "BITOR", "BITRSHIFT", "BITXOR",
    "BYCOL", "BYROW", "CEILING.MATH", "CEILING.PRECISE", "CHISQ.DIST", "CHISQ.DIST.RT",
    "CHISQ.INV", "CHISQ.INV.RT", "CHISQ.TEST", "CHOOSECOLS", "CHOOSEROWS", "COMBINA",
    "CONFIDENCE.NORM", "CONFIDENCE.T", "COPILOT", "COT", "COTH", "COVARIANCE.P", "COVARIANCE.S",
    "CSC", "CSCH", "DAYS", "DECIMAL", "DROP", "ERF.PRECISE", "ERFC.PRECISE", "EXPAND",
    "EXPON.DIST", "F.DIST", "F.DIST.RT", "F.INV", "F.INV.RT", "F.TEST", "FIELDVALUE",
    "FILTERXML", "FLOOR.MATH", "FLOOR.PRECISE", "FORMULATEXT", "GAMMA", "GAMMA.DIST",
    "GAMMA.INV", "GAMMALN.PRECISE", "GAUSS", "HSTACK", "HYPGEOM.DIST", "IFNA", "IMCOSH",
    "IMCOT", "IMCSC", "IMCSCH", "IMSEC", "IMSECH", "IMSINH", "IMTAN", "ISFORMULA", "ISOMITTED",
    "ISOWEEKNUM", "LAMBDA", "LET", "LOGNORM.DIST", "LOGNORM.INV", "LONGTEXT", "MAKEARRAY",
    "MAP", "MODE.MULT", "MODE.SNGL", "MUNIT", "NEGBINOM.DIST", "NORM.DIST", "NORM.INV",
    "NORM.S.DIST", "NORM.S.INV", "NUMBERVALUE", "PDURATION", "PERCENTILE.EXC", "PERCENTILE.INC",
    "PERCENTRANK.EXC", "PERCENTRANK.INC", "PERMUTATIONA", "PHI", "POISSON.DIST", "PQSOURCE",
    "PYTHON_STR", "PYTHON_TYPE", "PYTHON_TYPENAME", "QUARTILE.EXC", "QUARTILE.INC",
    "QUERYSTRING", "RANDARRAY", "RANK.AVG", "RANK.EQ", "REDUCE", "RRI", "SCAN", "SEC", "SECH",
    "SEQUENCE", "SHEET", "SHEETS", "SKEW.P", "SORTBY", "STDEV.P", "STDEV.S", "T.DIST",
    "T.DIST.2T", "T.DIST.RT", "T.INV", "T.INV.2T", "T.TEST", "TAKE", "TEXTAFTER", "TEXTBEFORE",
    "TEXTSPLIT", "TOCOL", "TOROW", "UNICHAR", "UNICODE", "UNIQUE", "VAR.P", "VAR.S", "VSTACK",
    "WEBSERVICE", "WEIBULL.DIST", "WRAPCOLS", "WRAPROWS", "XLOOKUP", "XOR", "Z.TEST"
];

/// The names MS-XLSX's production lists after the `_xlfn.` group, unprefixed. XlsxWriter writes
/// these four plain too, and Excel's own files hold ISO.CEILING, NETWORKDAYS.INTL and
/// WORKDAY.INTL plain. (The production's tail also lists the FORECAST.ETS family and
/// FORECAST.LINEAR, which XlsxWriter, written against Excel's files, prefixes: the grammar's
/// parenthesis is taken as misplaced for those.)
const MS_XLSX_PLAIN: &[&str] = &["ECMA.CEILING", "ISO.CEILING", "NETWORKDAYS.INTL", "WORKDAY.INTL"];

/// rust_xlsxwriter's `future_functions()` (its `src/formula.rs`), the names it prefixes with
/// `_xlfn.`, its worksheet-only FILTER, SORT and PY aside.
const XLSXWRITER_FUTURE: &[&str] = &[
    "ACOTH", "ACOT", "AGGREGATE", "ARABIC", "ARRAYTOTEXT", "BASE", "BETA.DIST", "BETA.INV",
    "BINOM.DIST.RANGE", "BINOM.DIST", "BINOM.INV", "BITAND", "BITLSHIFT", "BITOR", "BITRSHIFT",
    "BITXOR", "CEILING.MATH", "CEILING.PRECISE", "CHISQ.DIST.RT", "CHISQ.DIST", "CHISQ.INV.RT",
    "CHISQ.INV", "CHISQ.TEST", "COMBINA", "CONCAT", "CONFIDENCE.NORM", "CONFIDENCE.T", "COTH",
    "COT", "COVARIANCE.P", "COVARIANCE.S", "CSCH", "CSC", "DAYS", "DECIMAL", "ERF.PRECISE",
    "ERFC.PRECISE", "EXPON.DIST", "F.DIST.RT", "F.DIST", "F.INV.RT", "F.INV", "F.TEST",
    "FIELDVALUE", "FILTERXML", "FLOOR.MATH", "FLOOR.PRECISE", "FORECAST.ETS.CONFINT",
    "FORECAST.ETS.SEASONALITY", "FORECAST.ETS.STAT", "FORECAST.ETS", "FORECAST.LINEAR",
    "FORMULATEXT", "GAMMA.DIST", "GAMMA.INV", "GAMMALN.PRECISE", "GAMMA", "GAUSS",
    "HYPGEOM.DIST", "IFNA", "IFS", "IMAGE", "IMCOSH", "IMCOT", "IMCSCH", "IMCSC", "IMSECH",
    "IMSEC", "IMSINH", "IMTAN", "ISFORMULA", "ISOMITTED", "ISOWEEKNUM", "LET", "LOGNORM.DIST",
    "LOGNORM.INV", "MAXIFS", "MINIFS", "MODE.MULT", "MODE.SNGL", "MUNIT", "NEGBINOM.DIST",
    "NORM.DIST", "NORM.INV", "NORM.S.DIST", "NORM.S.INV", "NUMBERVALUE", "PDURATION",
    "PERCENTILE.EXC", "PERCENTILE.INC", "PERCENTRANK.EXC", "PERCENTRANK.INC", "PERMUTATIONA",
    "PHI", "POISSON.DIST", "PQSOURCE", "PYTHON_STR", "PYTHON_TYPE", "PYTHON_TYPENAME",
    "QUARTILE.EXC", "QUARTILE.INC", "QUERYSTRING", "RANK.AVG", "RANK.EQ", "RRI", "SECH", "SEC",
    "SHEETS", "SHEET", "SKEW.P", "STDEV.P", "STDEV.S", "T.DIST.2T", "T.DIST.RT", "T.DIST",
    "T.INV.2T", "T.INV", "T.TEST", "TEXTAFTER", "TEXTBEFORE", "TEXTJOIN", "UNICHAR", "UNICODE",
    "VALUETOTEXT", "VAR.P", "VAR.S", "WEBSERVICE", "WEIBULL.DIST", "XMATCH", "XOR", "Z.TEST",
    "ANCHORARRAY", "BYCOL", "BYROW", "CHOOSECOLS", "CHOOSEROWS", "DROP", "EXPAND", "HSTACK",
    "LAMBDA", "MAKEARRAY", "MAP", "RANDARRAY", "REDUCE", "SCAN", "SEQUENCE", "SINGLE", "SORTBY",
    "SWITCH", "TAKE", "TEXTSPLIT", "TOCOL", "TOROW", "UNIQUE", "VSTACK", "WRAPCOLS", "WRAPROWS",
    "XLOOKUP"
];

/// Prefixed in Excel's own files among the xlsx crate's tests, listed by neither source.
const EXCEL_FILES_FUTURE: &[&str] = &["ENCODEURL", "REGEXEXTRACT", "REGEXREPLACE", "REGEXTEST", "TRIMRANGE"];

/// Listed by no source and in no Excel-saved file of the tests: prefixed as every function Excel
/// added after 2010 is, a reasoned spelling no Excel file has shown (Veredas 2's Q346, ruled
/// 2026-11-25: it stands).
const UNLISTED_FUTURE: &[&str] = &["PERCENTOF"];

/// The worksheet-only functions (`worksheet-only-function-list`), written `_xlfn._xlws.`.
const WORKSHEET_ONLY: &[&str] = &["FILTER", "SORT"];

/// How Excel writes the function `name` in a file.
fn excel_spelling(name: &str) -> String {
    let any = |list: &[&str]| list.contains(&name);
    if any(WORKSHEET_ONLY) {
        format!("_xlfn._xlws.{name}")
    } else if any(MS_XLSX_FUTURE) || any(XLSXWRITER_FUTURE) || any(EXCEL_FILES_FUTURE) || any(UNLISTED_FUTURE) {
        format!("_xlfn.{name}")
    } else {
        name.to_string()
    }
}

#[test]
fn every_function_is_written_as_excel_writes_it() {
    let language = get_default_language();
    let wrong: Vec<String> = Function::into_iter()
        .map(|f| (f.to_localized_name(language), f.to_xlsx_string()))
        .filter(|(name, written)| *written != excel_spelling(name))
        .map(|(name, written)| format!("{name} written {written}, Excel writes {}", excel_spelling(&name)))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn the_sources_agree_and_the_functions_found_wrong_are_held() {
    // no name is both prefixed and plain
    for name in MS_XLSX_PLAIN {
        for list in [MS_XLSX_FUTURE, XLSXWRITER_FUTURE, EXCEL_FILES_FUTURE, UNLISTED_FUTURE, WORKSHEET_ONLY] {
            assert!(!list.contains(name), "{name}");
        }
    }
    // the spellings fork change 25 corrected, each by name
    let written = |name: &str| {
        let language = get_default_language();
        Function::into_iter()
            .find(|f| f.to_localized_name(language) == name)
            .map(|f| f.to_xlsx_string())
            .unwrap_or_else(|| panic!("no function {name}"))
    };
    for name in ["DAYS", "GAUSS", "GAMMA.DIST", "IMSECH", "FORECAST.LINEAR", "PERCENTOF"] {
        assert_eq!(written(name), format!("_xlfn.{name}"));
    }
    for name in ["CEILING", "GAMMALN", "ISO.CEILING", "ECMA.CEILING", "NETWORKDAYS.INTL", "WORKDAY.INTL", "SUM"] {
        assert_eq!(written(name), name);
    }
    assert_eq!(written("FILTER"), "_xlfn._xlws.FILTER");
    assert_eq!(written("CONCAT"), "_xlfn.CONCAT");
}
