//! The test file and the function file (contract: C-201): the reference's worked example loaded
//! from `docs/test-schema.md` itself, read and checked; every statement of the reference about
//! reading and checking held against the reader; every refusal by its text and its place; the
//! reference's text held as a copy.

use std::collections::BTreeMap;
use std::path::Path;

use veredas_comissionador::lex::KEYWORDS;
use veredas_comissionador::model::*;
use veredas_comissionador::{Checked, Counts, Problem, check_functions, check_test, load_test, parse_functions, parse_test};

// ---- the reference -------------------------------------------------------------------------------

fn doc() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/test-schema.md");
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

/// The fenced blocks of a language, in order.
fn blocks(lang: &str) -> Vec<String> {
    let text = doc();
    let fence = format!("```{lang}\n");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find(&fence) {
        let body = &rest[start + fence.len()..];
        let end = body.find("```").unwrap();
        out.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    out
}

fn example() -> (String, String) {
    let t = blocks("test");
    let f = blocks("function");
    assert_eq!((t.len(), f.len()), (1, 1), "the reference holds one test block and one function block");
    (t[0].clone(), f[0].clone())
}

/// The reference's prose on one line: what a sentence of it says.
fn prose() -> String {
    doc().split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- reading a text --------------------------------------------------------------------------------

const HEAD: &str = "format = \"veredas-test\"\nversion = 1\n\n";
const FHEAD: &str = "format = \"veredas-functions\"\nversion = 1\n\n";

/// A test of `body` (its settings and steps), with `id T` first.
fn test_of(body: &str) -> String {
    format!("{HEAD}test \"t\" {{\n  id T\n{body}}}\n")
}

/// Check a test against function files given by their paths.
fn check_with(text: &str, files: &[(&str, &str)]) -> Result<Checked, Vec<Problem>> {
    let map: BTreeMap<String, String> = files.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect();
    check_test(text, "t.test", &|p| map.get(p).cloned().ok_or_else(|| "not there".to_string()))
}

fn checked(text: &str) -> Checked {
    check_with(text, &[]).unwrap_or_else(|p| panic!("{}", p.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n")))
}

/// Every problem of a test, as `line:column: message`.
fn problems(text: &str) -> Vec<String> {
    problems_with(text, &[])
}

fn problems_with(text: &str, files: &[(&str, &str)]) -> Vec<String> {
    match check_with(text, files) {
        Ok(_) => Vec::new(),
        Err(ps) => ps.iter().map(|p| format!("{}{}:{}: {}", file_of(p), p.line, p.column, p.message)).collect(),
    }
}

/// A problem in another file than the test names it.
fn file_of(p: &Problem) -> String {
    if p.file == "t.test" { String::new() } else { format!("{}:", p.file) }
}

/// Every problem of a function file alone.
fn fproblems(text: &str) -> Vec<String> {
    match check_functions(text, "f.fn") {
        Ok(_) => Vec::new(),
        Err(ps) => ps.iter().map(|p| format!("{}:{}: {}", p.line, p.column, p.message)).collect(),
    }
}

/// The steps' actions, as their keywords.
fn keywords(step: &Step) -> Vec<&'static str> {
    step.actions.iter().map(|a| a.keyword()).collect()
}

fn name(e: &Expr) -> &str {
    match &e.kind {
        ExprKind::Name(n) => n,
        other => panic!("not a name: {other:?}"),
    }
}

// ---- the worked example -----------------------------------------------------------------------------

#[test]
fn the_worked_example_reads_as_the_reference_says() {
    let (text, functions) = example();
    let t = parse_test(&text, "chk-012.test").unwrap();
    assert_eq!(t.title.value, "Destination 4 full, items come round");
    assert_eq!(t.id(), Some("CHK-012"));
    assert_eq!(t.area(), Some("outfeed"));
    assert_eq!(t.portal(), Some("the press hall"));
    assert_eq!(t.modes(), [Mode::Live, Mode::Replay]);
    assert_eq!(t.timeout_ms(), 120_000);
    assert_eq!(t.uses().iter().map(|u| u.value.as_str()).collect::<Vec<_>>(), ["functions/routing.fn"]);
    let lets = t.lets();
    assert_eq!(lets.len(), 1);
    assert_eq!((lets[0].0.value.as_str(), &lets[0].1.kind), ("items", &ExprKind::Int(20)));

    let names: Vec<&str> = t.steps.iter().map(|s| s.name.value.as_str()).collect();
    assert_eq!(names, ["start", "running", "route", "ratio", "verdict", "release", "summary", "fail_log"]);
    let blocks: Vec<Block> = t.steps.iter().map(Step::block).collect();
    assert_eq!(
        blocks,
        [Block::Write, Block::Wait, Block::Function, Block::Script, Block::Branch, Block::Write, Block::Report, Block::Fail]
    );
    assert_eq!(keywords(&t.steps[6]), ["report", "stop"]);
    assert_eq!(keywords(&t.steps[7]), ["send", "report", "stop"]);

    // start: a send of the channel's command, gated.
    let ActionKind::Send { channel, message, fields } = &t.steps[0].actions[0].kind else { panic!() };
    assert_eq!((channel.value.as_str(), message.value.as_str()), ("sorter", "command"));
    assert_eq!(fields.len(), 1);
    assert_eq!((fields[0].name.value.as_str(), &fields[0].value.kind), ("code", &ExprKind::Int(1)));
    assert!(t.steps[0].actions[0].is_gated());

    // running: a wait with its timeout, its timeout routed.
    let ActionKind::Wait { condition, timeout } = &t.steps[1].actions[0].kind else { panic!() };
    assert_eq!(timeout.as_ref().map(|t| t.value), Some(12_000));
    let ExprKind::Binary { op: BinOp::Cmp(CmpOp::Eq), left, right } = &condition.kind else { panic!("{condition:?}") };
    assert!(matches!(&left.kind, ExprKind::Field { base, field } if base.value == "status" && field.value == "running"));
    assert_eq!(right.kind, ExprKind::Int(1));
    assert_eq!(t.steps[1].on.len(), 1);
    assert_eq!(t.steps[1].on[0].outcomes.iter().map(|o| o.value).collect::<Vec<_>>(), [Outcome::Timeout]);
    assert!(matches!(&t.steps[1].on[0].route.value, Route::Step(s) if s.value == "fail_log"));

    // route: the call, its outputs on the line after it.
    let ActionKind::Call { function, args, outputs } = &t.steps[2].actions[0].kind else { panic!() };
    assert_eq!(function.value, "ItemRoute");
    assert_eq!(args.iter().map(|a| a.name.value.as_str()).collect::<Vec<_>>(), ["dest", "items"]);
    assert_eq!(args[0].value.kind, ExprKind::Int(4));
    assert_eq!(name(&args[1].value), "items");
    assert_eq!(outputs.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["delivered", "rerouted"]);

    // ratio: the script box's two assignments.
    let ActionKind::Script { body } = &t.steps[3].actions[0].kind else { panic!() };
    let targets: Vec<&str> = body
        .iter()
        .map(|s| match &s.kind {
            StmtKind::Assign { target, op: AssignOp::Set, .. } => target.value.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(targets, ["ratio", "margin"]);

    // verdict: three arms.
    let ActionKind::Branch { value, arms, otherwise } = &t.steps[4].actions[0].kind else { panic!() };
    assert_eq!(name(value), "ratio");
    assert_eq!(arms.len(), 2);
    assert!(matches!(&arms[0].patterns[..], [Pattern::Compare { op: CmpOp::Ge, value }] if value.kind == ExprKind::Float(0.95)));
    assert!(matches!(&arms[0].route.value, Route::Step(s) if s.value == "release"));
    assert!(matches!(&arms[1].patterns[..], [Pattern::Range { low, high }]
        if low.kind == ExprKind::Float(0.85) && high.kind == ExprKind::Float(0.95)));
    assert!(matches!(&arms[1].route.value, Route::Retry { step, max } if step.value == "route" && max.value == 1));
    assert!(matches!(&otherwise.as_ref().unwrap().value, Route::Step(s) if s.value == "fail_log"));

    // summary and fail_log: the report's value and note, and the two ends.
    let ActionKind::Report { items } = &t.steps[6].actions[0].kind else { panic!() };
    assert_eq!(items.iter().map(name).collect::<Vec<_>>(), ["ratio", "margin", "rerouted"]);
    assert!(matches!(t.steps[6].actions[1].kind, ActionKind::Stop { verdict: Verdict::Pass, reason: None }));
    let ActionKind::Report { items } = &t.steps[7].actions[1].kind else { panic!() };
    let ExprKind::Str(parts) = &items[0].kind else { panic!() };
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], StrPart::Text("rerouted ".to_string()));
    assert!(matches!(&parts[1], StrPart::Expr(e) if name(e) == "rerouted"));
    assert_eq!(parts[2], StrPart::Text("/".to_string()));
    assert!(matches!(&parts[3], StrPart::Expr(e) if name(e) == "items"));
    assert!(matches!(t.steps[7].actions[2].kind, ActionKind::Stop { verdict: Verdict::Fail, reason: None }));

    // The layout, last.
    let layout = t.layout.as_ref().unwrap();
    let pos: Vec<(&str, u32, u32)> = layout.positions.iter().map(|p| (p.step.value.as_str(), p.column, p.row)).collect();
    assert_eq!(
        pos,
        [
            ("start", 0, 1),
            ("running", 0, 2),
            ("route", 0, 3),
            ("ratio", 0, 4),
            ("verdict", 0, 5),
            ("fail_log", 1, 5),
            ("release", 0, 6),
            ("summary", 0, 7)
        ]
    );

    // The function file.
    let ff = parse_functions(&functions, "functions/routing.fn").unwrap();
    assert_eq!(ff.functions.len(), 1);
    let f = &ff.functions[0];
    assert_eq!(f.signature(), "ItemRoute(dest: int, items: int) -> (delivered: int, rerouted: int)");
    assert_eq!(
        f.description,
        "Follows the outcomes reported for one destination and counts where\nthe items ended up. Shows as a block: inputs left, outputs right."
    );
    assert_eq!(f.reads.iter().map(|r| r.value.as_str()).collect::<Vec<_>>(), ["outcome"]);
    assert_eq!(f.timeout.as_ref().map(|t| t.value), Some(60_000));
    assert_eq!(f.body.len(), 3);
    let StmtKind::For { var, body, .. } = &f.body[2].kind else { panic!() };
    assert_eq!(var.value, "n");
    let StmtKind::Await { bind, table, filter, timeout } = &body[0].kind else { panic!() };
    assert_eq!((bind.value.as_str(), table.value.as_str()), ("o", "outcome"));
    assert!(filter.is_some() && timeout.is_none());
    let StmtKind::If { arms, otherwise } = &body[1].kind else { panic!() };
    assert_eq!(arms.len(), 2, "`else if` on the line after the first block");
    assert!(otherwise.is_none());

    // Checked together: the counts the reference states, and every name settled.
    let c = check_with(&text, &[("functions/routing.fn", functions.as_str())]).unwrap();
    assert_eq!(c.counts, Counts { steps: 8, branches: 1, functions: 1, gated_writes: 3 });
    assert!(prose().contains("Read, the test has eight steps, one branch, one function and three gated writes, its three `send`s."));
    let ActionKind::Wait { condition, .. } = &c.test.steps[1].actions[0].kind else { panic!() };
    let ExprKind::Binary { left, .. } = &condition.kind else { panic!() };
    assert!(
        matches!(&left.kind, ExprKind::Plant { table, column } if table.value == "status" && column.value == "running"),
        "a name that is no variable reads plant data: {left:?}"
    );
    let f = c.function("ItemRoute").unwrap();
    let StmtKind::For { body, .. } = &f.body[2].kind else { panic!() };
    let StmtKind::If { arms, .. } = &body[1].kind else { panic!() };
    let ExprKind::Binary { left, .. } = &arms[0].0.kind else { panic!() };
    assert!(
        matches!(&left.kind, ExprKind::Field { base, field } if base.value == "o" && field.value == "result"),
        "a variable's name reads the row's column: {left:?}"
    );
    assert_eq!(c.files[0].file, "functions/routing.fn");
}

#[test]
fn the_worked_example_loads_from_disk_beside_its_function_folder() {
    let (text, functions) = example();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("functions")).unwrap();
    std::fs::write(dir.path().join("chk-012.test"), &text).unwrap();
    std::fs::write(dir.path().join("functions/routing.fn"), &functions).unwrap();
    let c = load_test(&dir.path().join("chk-012.test")).unwrap();
    assert_eq!(c.counts, Counts { steps: 8, branches: 1, functions: 1, gated_writes: 3 });
    // The same file with line ends of two bytes reads the same.
    let crlf = text.replace('\n', "\r\n");
    std::fs::write(dir.path().join("crlf.test"), &crlf).unwrap();
    let c2 = load_test(&dir.path().join("crlf.test")).unwrap();
    assert_eq!(c2.counts, c.counts);
    assert_eq!(c2.test.steps.iter().map(|s| s.name.value.clone()).collect::<Vec<_>>(), c.test.steps.iter().map(|s| s.name.value.clone()).collect::<Vec<_>>());
    // A `use` read relative to the test file's folder, and named so in a problem.
    std::fs::remove_file(dir.path().join("functions/routing.fn")).unwrap();
    let e = load_test(&dir.path().join("chk-012.test")).unwrap_err();
    assert_eq!(e.len(), 2, "{e:?}");
    assert_eq!((e[0].line, e[0].column), (11, 11));
    assert!(e[0].message.starts_with("`functions/routing.fn` cannot be read: "), "{}", e[0].message);
    assert_eq!(e[1].message, "`ItemRoute` is no function of the files this test uses");
}

// ---- the header ---------------------------------------------------------------------------------

#[test]
fn the_header_is_the_suite_s_stamp_read_before_the_body() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |text: &str| -> String {
        let e = parse_test(text, "t.test").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    assert_eq!(one(body), "1:1: no `format`: a test file states `format` and `version`");
    assert_eq!(one(&format!("format = \"veredas-test\"\n{body}")), "1:1: no `version`: a test file states `format` and `version`");
    assert_eq!(
        one(&format!("format = \"veredas-functions\"\nversion = 1\n{body}")),
        "1:1: `format` is `veredas-functions`, not `veredas-test`"
    );
    assert_eq!(
        one(&format!("format = \"veredas-test\"\nversion = 2\n{body}")),
        "1:1: version 2 of `veredas-test` is not known here (this reader knows 1)"
    );
    // The format decided whole before the version: a wrong format with a wrong version names the format.
    assert_eq!(one(&format!("format = \"veredas-bench\"\nversion = 9\n{body}")), "1:1: `format` is `veredas-bench`, not `veredas-test`");
    // The stamp alone: another key refused by its name, at its line.
    assert_eq!(
        one(&format!("format = \"veredas-test\"\nversion = 1\nname = \"x\"\n{body}")),
        "3:1: the header holds `format` and `version` alone, not `name`"
    );
    // Comments and blank lines anywhere in the header.
    assert!(parse_test(&format!("# a test\n\nformat = \"veredas-test\" # the stamp\n\nversion = 1\n\n# its body\n{body}"), "t").is_ok());
    // The body begins at the first line that opens with `test`: a key named `testing` is header.
    assert_eq!(
        one(&format!("format = \"veredas-test\"\nversion = 1\ntesting = 1\n{body}")),
        "3:1: the header holds `format` and `version` alone, not `testing`"
    );
    // A function file has its own format.
    let fbody = "function F() {\n}\n";
    let e = parse_functions(&format!("format = \"veredas-test\"\nversion = 1\n{fbody}"), "f.fn").unwrap_err();
    assert_eq!(e.message, "`format` is `veredas-test`, not `veredas-functions`");
    let e = parse_functions(fbody, "f.fn").unwrap_err();
    assert_eq!(e.message, "no `format`: a function file states `format` and `version`");
    assert!(parse_functions(&format!("{FHEAD}{fbody}"), "f.fn").is_ok());
    // A file that is header alone has no test.
    assert_eq!(one(HEAD), "4:1: the file's end where `test` was expected");
}

// ---- the file, line by line ---------------------------------------------------------------------

#[test]
fn comments_blank_lines_and_line_ends_go_anywhere() {
    let t = checked(&format!(
        "{HEAD}# before\ntest \"t\" {{ # after the brace\n\n  id T # the id\n\n  # between\n  step a wait 1s # a delay\n\n  step b report \"a # in a string\"\n}}\n# after\n"
    ));
    assert_eq!(t.test.id(), Some("T"));
    let ActionKind::Report { items } = &t.test.steps[1].actions[0].kind else { panic!() };
    assert_eq!(items[0].kind, ExprKind::Str(vec![StrPart::Text("a # in a string".to_string())]));
}

#[test]
fn the_language_s_own_words_name_nothing_else() {
    // The reference's list is the reader's.
    let p = prose();
    let at = p.find("These words are the language's own and name nothing else: ").unwrap();
    let rest = &p[at..];
    let list = &rest[..rest.find(". After a").unwrap()];
    let words: Vec<String> = list.split('`').skip(1).step_by(2).map(str::to_string).collect();
    assert_eq!(words, KEYWORDS, "the reference's words and the reader's");
    for w in KEYWORDS {
        let text = test_of(&format!("  step {w} stop pass\n"));
        assert_eq!(
            problems(&text),
            [format!("6:8: `{w}` is a word of the language, not a name (a step's name)")],
            "{w}"
        );
    }
    // After a `.` any name is a column's; a table so named is written in backquotes.
    let c = checked(&test_of("  step a await outcome where o.read == \"N\" -> o\n  step b wait `check`.value == 1\n"));
    let ActionKind::Await { filter, .. } = &c.test.steps[0].actions[0].kind else { panic!() };
    let ExprKind::Binary { left, .. } = &filter.as_ref().unwrap().kind else { panic!() };
    assert!(matches!(&left.kind, ExprKind::Field { base, field } if base.value == "o" && field.value == "read"));
    let ActionKind::Wait { condition, .. } = &c.test.steps[1].actions[0].kind else { panic!() };
    let ExprKind::Binary { left, .. } = &condition.kind else { panic!() };
    assert!(matches!(&left.kind, ExprKind::Plant { table, column } if table.value == "check" && column.value == "value"));
    // A name is ASCII letters, digits and `_`, not starting with a digit.
    assert_eq!(problems(&test_of("  step a_1 stop pass\n")), Vec::<String>::new());
    assert_eq!(problems(&test_of("  step 1a stop pass\n")), ["6:8: `1a`: a number run into letters (a duration's units are ms, s, min and h)"]);
    assert_eq!(problems(&test_of("  step é stop pass\n")), ["6:8: `é` is no part of the language"]);
    assert_eq!(problems(&test_of("  step a wait `` `.x == 1\n")), ["6:15: a name in backquotes is the name alone, not blank, no space around it"]);
    assert_eq!(problems(&test_of("  step a wait `t.x == 1\n")), ["6:15: a backquote not closed on its line"]);
}

#[test]
fn a_line_ends_what_it_holds_except_inside_parentheses_and_before_an_arrow() {
    let c = checked(&test_of(
        "  step a send sorter.command(\n    code: 1,\n    lane: 2\n  )\n  step b check (1 +\n    2) == 3\n  step c read t.x\n         -> x\n",
    ));
    let ActionKind::Send { fields, .. } = &c.test.steps[0].actions[0].kind else { panic!() };
    assert_eq!(fields.len(), 2);
    assert_eq!(keywords(&c.test.steps[1]), ["check"]);
    let ActionKind::Read { into, .. } = &c.test.steps[2].actions[0].kind else { panic!() };
    assert_eq!(into[0].value, "x");
    // Outside parentheses a line's end ends the action.
    assert_eq!(problems(&test_of("  step a check 1 +\n    2 == 3\n")), ["6:19: a line's end where a value was expected"]);
}

#[test]
fn literals_read_as_the_reference_says() {
    let c = checked(&test_of(
        "  let a = 20\n  let b = 9_000\n  let c = 0.95\n  let d = 1e3\n  let e = true\n  let f = false\n  let g = 250ms\n  let h = 5s\n  let i = 2min\n  let j = 1h\n  let k = 1.5s\n  let l = \"q\\\"b\\\\n\\n\\t\\{\\}\"\n  step x stop pass\n",
    ));
    let lets: Vec<ExprKind> = c.test.lets().iter().map(|(_, e)| e.kind.clone()).collect();
    assert_eq!(
        lets,
        [
            ExprKind::Int(20),
            ExprKind::Int(9000),
            ExprKind::Float(0.95),
            ExprKind::Float(1000.0),
            ExprKind::Bool(true),
            ExprKind::Bool(false),
            ExprKind::Duration(250),
            ExprKind::Duration(5000),
            ExprKind::Duration(120_000),
            ExprKind::Duration(3_600_000),
            ExprKind::Duration(1500),
            ExprKind::Str(vec![StrPart::Text("q\"b\\n\n\t{}".to_string())]),
        ]
    );
    // `{` an expression `}` in a string.
    let c = checked(&test_of("  let n = 2\n  step x report \"n is {n + 1}, {n}\"\n"));
    let ActionKind::Report { items } = &c.test.steps[0].actions[0].kind else { panic!() };
    let ExprKind::Str(parts) = &items[0].kind else { panic!() };
    assert_eq!(parts.len(), 4);
    assert!(matches!(&parts[1], StrPart::Expr(Expr { kind: ExprKind::Binary { op: BinOp::Add, .. }, .. })));
    // What cannot be read, each by its reason and place.
    let one = |line: &str| problems(&test_of(&format!("  step x {line}\n")));
    assert_eq!(one("wait 5sec"), ["6:15: `5sec`: a number run into letters (a duration's units are ms, s, min and h)"]);
    assert_eq!(one("wait 0.5ms"), ["6:15: `0.5ms` is not a whole number of milliseconds"]);
    assert_eq!(one("report 9_"), ["6:17: `9_`: a `_` stands between two digits"]);
    assert_eq!(one("report 99999999999999999999"), ["6:17: `99999999999999999999` is past a whole number's range"]);
    assert_eq!(one("report \"open"), ["6:17: a string not closed on its line"]);
    assert_eq!(one("report \"a \\q b\""), ["6:17: `\\q` is no escape (\\\", \\\\, \\n, \\t, \\{, \\})"]);
    assert_eq!(one("report \"a } b\""), ["6:17: a `}` alone in a string (written `\\}`)"]);
    assert_eq!(one("report \"a {b\""), ["6:17: a `{` in a string not closed by `}` (a `{` alone is written `\\{`)"]);
    assert_eq!(one("report \"a {} b\""), ["6:20: a `{` in a string holds an expression (a `{` alone is written `\\{`)"]);
    assert_eq!(one("report \"a {1 2} b\""), ["6:23: `2` where the `}` closing the expression was expected"]);
    assert_eq!(one("report 1 @ 2"), ["6:19: `@` is no part of the language"]);
}

// ---- settings -----------------------------------------------------------------------------------

#[test]
fn settings_come_first_with_their_defaults() {
    let bare = checked(&test_of("  step a stop pass\n"));
    assert_eq!((bare.test.area(), bare.test.portal()), (None, None));
    assert_eq!(bare.test.modes(), [Mode::Live, Mode::Replay], "modes default to both");
    assert_eq!(bare.test.timeout_ms(), 600_000, "a run's bound defaults to ten minutes");
    assert!(bare.test.uses().is_empty() && bare.test.lets().is_empty());
    // id, area, portal and use take the rest of their line: trimmed, a comment and a pair of
    // quotes taken off.
    let c = checked(&format!(
        "{HEAD}test \"t\" {{\n  id   CHK-012_b   # its id\n  area \"out # feed\"  # quoted, with a hash\n  portal  the press hall\n  modes replay\n  timeout 30s\n  step a stop pass\n}}\n"
    ));
    assert_eq!(c.test.id(), Some("CHK-012_b"));
    assert_eq!(c.test.area(), Some("out # feed"));
    assert_eq!(c.test.portal(), Some("the press hall"));
    assert_eq!(c.test.modes(), [Mode::Replay]);
    assert_eq!(c.test.timeout_ms(), 30_000);
    let one = |settings: &str| problems(&format!("{HEAD}test \"t\" {{\n{settings}  step a stop pass\n}}\n"));
    assert_eq!(one(""), ["4:1: the test has no `id`"]);
    assert_eq!(one("  id\n"), ["5:5: `id` has no value"]);
    assert_eq!(one("  id \"\"\n"), ["5:5: `id` has no value"]);
    assert_eq!(one("  id CHK 012\n"), ["5:6: `id` is `CHK 012`: letters, digits, `-` and `_`"]);
    assert_eq!(one("  id T\n  area a\n  area b\n"), ["7:3: `area` is given twice"]);
    assert_eq!(one("  id T\n  id U\n"), ["6:3: `id` is given twice"]);
    assert_eq!(one("  id T\n  portal p\n  portal q\n"), ["7:3: `portal` is given twice"]);
    assert_eq!(one("  id T\n  modes live\n  modes replay\n"), ["7:3: `modes` is given twice"]);
    assert_eq!(one("  id T\n  timeout 1s\n  timeout 2s\n"), ["7:3: `timeout` is given twice"]);
    assert_eq!(one("  id T\n  modes live, sometimes\n"), ["6:15: `sometimes` is no mode (live, replay)"]);
    assert_eq!(one("  id T\n  modes live, live\n"), ["6:15: `live` is named twice in `modes`"]);
    assert_eq!(one("  id T\n  timeout 5\n"), ["6:11: `5` where a duration (`5s`, `250ms`) was expected"]);
    assert_eq!(one("  id T\n  owner me\n"), ["6:3: `owner` is no setting (id, area, portal, modes, timeout, use, sign, let)"]);
    assert_eq!(one("  id T\n  let items = 1\n  let items = 2\n"), ["7:7: `items` is defined twice"]);
    assert_eq!(one("  id T\n  let a = b\n  let b = 1\n"), ["6:11: a `let` is made of literals and the constants above it: `b`"]);
    assert_eq!(one("  id T\n  let a = 1\n  let b = a * 2 + 1\n"), Vec::<String>::new());
    assert_eq!(one("  id T\n  let a = t.c\n"), ["6:11: a `let` is made of literals and the constants above it: `t.c`"]);
    assert_eq!(one("  id T\n  let step = 1\n"), ["6:7: `step` is a word of the language, not a name (a constant's name)"]);
    // A setting after a step.
    assert_eq!(
        problems(&format!("{HEAD}test \"t\" {{\n  id T\n  step a stop pass\n  area late\n}}\n")),
        ["7:3: `area` is a setting, and settings come before the first step"]
    );
    assert_eq!(
        problems(&format!("{HEAD}test \"t\" {{\n  id T\n  step a report 1\n  area late\n}}\n")),
        ["7:3: `area` is a setting, and settings come before the first step"]
    );
    // The title is a plain string.
    assert_eq!(problems(&format!("{HEAD}test t {{\n}}\n")), ["4:6: `t` where the test's title (a string) was expected"]);
    assert_eq!(problems(&format!("{HEAD}test \"t {{x}}\" {{\n}}\n")), ["4:9: the test's title (a string) is plain text: no `{` expression `}` in it"]);
}

#[test]
fn a_used_file_is_read_beside_the_test_and_its_functions_are_unique() {
    let f = format!("{FHEAD}function F(a: int) -> (b: int) {{\n  b = a\n}}\n");
    let g = format!("{FHEAD}function G() {{\n}}\n\nfunction F() {{\n}}\n");
    let text = test_of("  use f.fn\n  use \"g.fn\"\n  step a call F(a: 1) -> b\n");
    assert_eq!(
        problems_with(&text, &[("f.fn", &f), ("g.fn", &g)]),
        ["g.fn:7:10: two functions are named `F`: the other is at f.fn:4:10"]
    );
    let c = check_with(&test_of("  use f.fn\n  step a call F(a: 1) -> b\n"), &[("f.fn", &f)]).unwrap();
    assert_eq!(c.files.len(), 1);
    assert_eq!((c.files[0].path.as_str(), c.counts.functions), ("f.fn", 1));
    assert_eq!(problems(&test_of("  use none.fn\n  step a stop pass\n")), ["6:7: `none.fn` cannot be read: not there"]);
    assert_eq!(
        problems_with(&test_of("  use f.fn\n  use f.fn\n  step a stop pass\n"), &[("f.fn", &f)]),
        ["7:7: `f.fn` is used twice"]
    );
    // A problem of a used file is named by its path from the test file's folder.
    let bad = format!("{FHEAD}function F( {{\n}}\n");
    let e = check_test(&test_of("  use lib/f.fn\n  step a stop pass\n"), "tests/t.test", &|_| Ok(bad.clone())).unwrap_err();
    assert_eq!(e[0].to_string(), "tests/lib/f.fn:4:13: `{` where a port's name was expected");
}

// ---- steps ----------------------------------------------------------------------------------------

#[test]
fn every_action_reads_into_its_kind() {
    let c = checked(&test_of(concat!(
        "  step w1 write plc_main.run := 1\n",
        "  step w2 send sorter.command(code: 0)\n",
        "  step w3 wait plc_main.speed >= 2.0\n",
        "  step w4 wait plc_main.speed >= 2.0 timeout 5s\n",
        "  step w5 wait 2s\n",
        "  step w6 await announce -> a\n",
        "  step w7 await announce where a.lane == 2 timeout 1s -> a\n",
        "  step r1 read plc_main.speed, status.running -> s, r\n",
        "  step c1 check s >= 1.8\n",
        "  step c2 check s >= 1.8 as \"speed reached\"\n",
        "  step s1 script { x = 1 }\n",
        "  step b1 branch x {\n    1 -> r1\n  }\n",
        "  step p1 report x, \"a note\"\n",
        "  step e1 stop pass \"done\"\n",
        "  step e2 stop fail\n",
    )));
    let kw: Vec<&str> = c.test.steps.iter().map(|s| s.actions[0].keyword()).collect();
    assert_eq!(kw, ["write", "send", "wait", "wait", "wait", "await", "await", "read", "check", "check", "script", "branch", "report", "stop", "stop"]);
    let s = &c.test.steps;
    let ActionKind::Write { target, value } = &s[0].actions[0].kind else { panic!() };
    assert_eq!((target.table.value.as_str(), target.column.value.as_str(), &value.kind), ("plc_main", "run", &ExprKind::Int(1)));
    assert!(matches!(&s[2].actions[0].kind, ActionKind::Wait { timeout: None, .. }));
    assert!(matches!(&s[3].actions[0].kind, ActionKind::Wait { timeout: Some(t), .. } if t.value == 5000));
    assert!(matches!(&s[4].actions[0].kind, ActionKind::Delay { duration } if duration.value == 2000));
    assert!(matches!(&s[5].actions[0].kind, ActionKind::Await { filter: None, timeout: None, bind, .. } if bind.value == "a"));
    assert!(matches!(&s[6].actions[0].kind, ActionKind::Await { filter: Some(_), timeout: Some(t), .. } if t.value == 1000));
    let ActionKind::Read { refs, into } = &s[7].actions[0].kind else { panic!() };
    assert_eq!(refs.iter().map(|r| format!("{}.{}", r.table.value, r.column.value)).collect::<Vec<_>>(), ["plc_main.speed", "status.running"]);
    assert_eq!(into.iter().map(|n| n.value.as_str()).collect::<Vec<_>>(), ["s", "r"]);
    assert!(matches!(&s[8].actions[0].kind, ActionKind::Check { label: None, .. }));
    assert!(matches!(&s[9].actions[0].kind, ActionKind::Check { label: Some(l), .. } if l.value == "speed reached"));
    let ActionKind::Report { items } = &s[12].actions[0].kind else { panic!() };
    assert!(!items[0].is_string() && items[1].is_string());
    assert!(matches!(&s[13].actions[0].kind, ActionKind::Stop { verdict: Verdict::Pass, reason: Some(_) }));
    let blocks: Vec<Block> = s.iter().map(Step::block).collect();
    assert_eq!(
        blocks,
        [
            Block::Write,
            Block::Write,
            Block::Wait,
            Block::Wait,
            Block::Wait,
            Block::Wait,
            Block::Wait,
            Block::Read,
            Block::Check,
            Block::Check,
            Block::Script,
            Block::Branch,
            Block::Report,
            Block::End,
            Block::Fail
        ]
    );
    assert_eq!(c.counts, Counts { steps: 15, branches: 1, functions: 0, gated_writes: 2 });
    // What an action cannot be.
    let one = |line: &str| problems(&test_of(&format!("  step x {line}\n")));
    assert_eq!(one("frobnicate 1"), ["6:10: `frobnicate` is no action (write, send, wait, await, read, check, call, script, branch, report, stop)"]);
    assert_eq!(one("write plc_main.run = 1"), ["6:29: `=` where `:=` was expected"]);
    assert_eq!(one("write run := 1"), ["6:20: `:=` where `.` and a column's name was expected"]);
    assert_eq!(one("send sorter(code: 1)"), ["6:21: `(` where `.` and the message's name was expected"]);
    assert_eq!(one("send sorter.command(code: 1, code: 2)"), ["6:39: field `code` is given twice"]);
    assert_eq!(one("send sorter.command(1)"), ["6:30: `1` where a name and `:` was expected"]);
    assert_eq!(one("await announce where a.lane == 1"), ["6:42: a line's end where `->` and the name the row is bound to was expected"]);
    assert_eq!(one("read t.a, t.b -> a"), ["6:24: `read` reads 2 values into 1 name"]);
    assert_eq!(one("read t.a -> a, b"), ["6:19: `read` reads 1 value into 2 names"]);
    assert_eq!(one("check 1 == 1 as label"), ["6:26: `label` where a check's label (a string) was expected"]);
    assert_eq!(one("stop maybe"), ["6:15: `maybe` where `pass` or `fail` was expected"]);
    assert_eq!(one("wait 1 == 1 timeout 5"), ["6:30: `5` where a duration (`5s`, `250ms`) was expected"]);
}

#[test]
fn a_step_is_a_name_and_actions_in_order_branch_and_stop_last() {
    let c = checked(&test_of("  step a write t.x := 1\n         report \"one\"\n         wait 1s\n  step b stop pass\n"));
    assert_eq!(keywords(&c.test.steps[0]), ["write", "report", "wait"]);
    assert_eq!(c.counts.gated_writes, 1);
    assert_eq!(problems(&test_of("  step a stop pass\n  step a stop fail\n")), ["7:8: step `a` is defined twice"]);
    assert_eq!(
        problems(&test_of("  step v branch 1 {\n    else -> v\n  }\n         report 1\n")),
        ["9:10: `branch` is a step's last action: nothing follows it in step `v`"]
    );
    assert_eq!(problems(&test_of("  step v stop pass\n         report 1\n")), ["7:10: `stop` is a step's last action: nothing follows it in step `v`"]);
    assert_eq!(
        problems(&test_of("  step a wait 1s\n         on timeout -> a\n         report 1\n")),
        ["8:10: the actions come before the step's `on` lines"]
    );
    assert_eq!(problems(&test_of("  report 1\n")), ["6:3: `report` where `step` or the test's closing `}` was expected"]);
    assert_eq!(problems(&test_of("  step a stop pass\n  report 1\n")), ["7:3: `stop` is a step's last action: nothing follows it in step `a`"]);
    assert_eq!(problems(&format!("{HEAD}test \"t\" {{\n  id T\n  step a stop pass\n")), ["7:1: the file's end where `step` or the test's closing `}` was expected"]);
}

#[test]
fn routes_name_a_step_a_retry_or_an_end() {
    let c = checked(&test_of(concat!(
        "  step a wait 1 == 1 timeout 1s\n",
        "         on timeout, error -> c\n",
        "         on fail -> stop fail\n",
        "  step b branch 2 {\n",
        "    1, 3 -> retry a max 2\n",
        "    == 2 -> stop pass\n",
        "    else -> c\n",
        "  }\n",
        "         on fail -> stop fail\n",
        "  step c stop pass\n",
    )));
    let on = &c.test.steps[0].on;
    assert_eq!(on.len(), 2);
    assert_eq!(on[0].outcomes.iter().map(|o| o.value).collect::<Vec<_>>(), [Outcome::Timeout, Outcome::Error]);
    assert!(matches!(&on[1].route.value, Route::Stop(Verdict::Fail)));
    let ActionKind::Branch { arms, .. } = &c.test.steps[1].actions[0].kind else { panic!() };
    assert_eq!(arms[0].patterns.len(), 2);
    assert!(matches!(&arms[0].route.value, Route::Retry { max, .. } if max.value == 2));
    assert!(matches!(&arms[1].route.value, Route::Stop(Verdict::Pass)));
    let one = |body: &str| problems(&test_of(body));
    assert_eq!(one("  step a wait 1s\n         on timeout -> nowhere\n"), ["7:24: `nowhere` is no step of this test"]);
    assert_eq!(one("  step a branch 1 {\n    1 -> retry ghost max 1\n  }\n"), ["7:16: `ghost` is no step of this test"]);
    assert_eq!(one("  step a branch 1 {\n    1 -> retry a max 0\n  }\n"), ["7:22: `retry a max 0` never retries: `max` is at least 1"]);
    assert_eq!(one("  step a branch 1 {\n    1 -> retry a 2\n  }\n"), ["7:18: `2` where `max` was expected"]);
    assert_eq!(one("  step a branch 1 {\n    1 -> retry a max -1\n  }\n"), ["7:22: `-` where a whole number of retries was expected"]);
    assert_eq!(one("  step a wait 1s\n         on late -> a\n"), ["7:13: `late` is no outcome (pass, fail, timeout, error)"]);
    assert_eq!(one("  step a wait 1s\n         on timeout -> a\n         on fail, timeout -> a\n"), ["8:19: `timeout` is routed twice in step `a`"]);
    assert_eq!(
        one("  step a branch 1 {\n    else -> a\n  }\n         on pass -> a\n"),
        ["9:13: a branch routes its `pass` by its arms: `on pass` in step `a`"]
    );
    assert_eq!(one("  step a wait 1s\n         on pass -> a\n"), Vec::<String>::new(), "`on pass` routes a step that is no branch");
    assert_eq!(one("  step a branch 1 {\n    else -> a\n    2 -> a\n  }\n"), ["8:5: `else` is a branch's last arm"]);
    assert_eq!(one("  step a branch 1 {\n  }\n"), ["7:3: a branch has at least one arm"]);
    assert_eq!(one("  step a branch 1 {\n    1 a\n  }\n"), ["7:7: `a` where `->` was expected"]);
    assert_eq!(one("  step a wait 1s\n         on timeout -> step\n"), ["7:24: `step` is a word of the language, not a name (a step's name, `retry` or `stop`)"]);
}

#[test]
fn a_branch_s_patterns_compare_range_or_match_a_value() {
    let c = checked(&test_of(concat!(
        "  step a branch 5 {\n",
        "    == 1 -> a\n    != 2 -> a\n    < 3 -> a\n    <= 4 -> a\n    > 5 -> a\n    >= 6 -> a\n",
        "    7 .. 8 -> a\n    \"FULL\", true -> a\n    1 + 1 -> a\n",
        "  }\n",
    )));
    let ActionKind::Branch { arms, otherwise, .. } = &c.test.steps[0].actions[0].kind else { panic!() };
    assert!(otherwise.is_none());
    let ops: Vec<CmpOp> = arms[..6]
        .iter()
        .map(|a| match &a.patterns[0] {
            Pattern::Compare { op, .. } => *op,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(ops, [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge]);
    assert!(matches!(&arms[6].patterns[..], [Pattern::Range { low, high }] if low.kind == ExprKind::Int(7) && high.kind == ExprKind::Int(8)));
    assert!(matches!(&arms[7].patterns[..], [Pattern::Value(a), Pattern::Value(b)]
        if a.is_string() && b.kind == ExprKind::Bool(true)));
    assert!(matches!(&arms[8].patterns[..], [Pattern::Value(Expr { kind: ExprKind::Binary { op: BinOp::Add, .. }, .. })]));
}

#[test]
fn the_layout_places_steps_on_the_grid() {
    let one = |layout: &str| problems(&format!("{}{layout}\n", test_of("  step a stop pass\n  step b stop fail\n")));
    assert_eq!(one("layout { a 0,0\n  b 3,12 }"), Vec::<String>::new());
    assert_eq!(one("layout { a 0,0  ghost 1,1 }"), ["9:17: `ghost` in the layout is no step of this test"]);
    assert_eq!(one("layout { a 0,0  a 1,1 }"), ["9:17: `a` is placed twice in the layout"]);
    assert_eq!(one("layout { a -1,0 }"), ["9:12: `-` where a whole number from 0 was expected"]);
    assert_eq!(one("layout { a 1 }"), ["9:14: `}` where `,` was expected"]);
    assert_eq!(one("layout { a 0,0 }\nmore"), ["10:1: `more` where the file's end was expected"]);
    assert_eq!(one("more"), ["9:1: `more` where the file's end or the `layout` block was expected"]);
    let c = checked(&format!("{}layout {{ b 2,3 }}\n", test_of("  step a stop pass\n  step b stop fail\n")));
    let l = c.test.layout.unwrap();
    assert_eq!((l.positions[0].step.value.as_str(), l.positions[0].column, l.positions[0].row), ("b", 2, 3));
}

// ---- the function file ----------------------------------------------------------------------------

#[test]
fn a_function_file_holds_functions_with_ports_reads_and_a_timeout() {
    let text = format!(
        "{FHEAD}# Waits.\nfunction Settle(tag: float, band: float) {{\n  timeout 5s\n  reads plc_main\n  wait abs(plc_main.speed - tag) <= band\n}}\n\n# Two lines\n# of description.\nfunction Twice(a: int, b: float, c: bool, d: text, e: time, f: duration) -> (x: int, y: float) {{\n  x = a * 2\n  y = b\n}}\n"
    );
    let ff = check_functions(&text, "f.fn").unwrap();
    assert_eq!(ff.functions.len(), 2);
    let s = &ff.functions[0];
    assert_eq!(s.signature(), "Settle(tag: float, band: float)");
    assert_eq!(s.description, "Waits.");
    assert_eq!((s.timeout.as_ref().map(|t| t.value), s.reads.len()), (Some(5000), 1));
    assert_eq!(ff.functions[1].signature(), "Twice(a: int, b: float, c: bool, d: text, e: time, f: duration) -> (x: int, y: float)");
    assert_eq!(ff.functions[1].description, "Two lines\nof description.");
    let one = |f: &str| fproblems(&format!("{FHEAD}{f}"));
    assert_eq!(one("function F(a: integer) {\n}\n"), ["4:15: `integer` is no type (int, float, bool, text, time, duration)"]);
    assert_eq!(one("function F(a: int, a: int) {\n}\n"), ["4:20: `a` is named twice in `F`'s ports"]);
    assert_eq!(one("function F(a: int) -> (a: int) {\n  a = 1\n}\n"), ["4:24: `a` is named twice in `F`'s ports", "5:3: `a` is an input of `F`, read only"]);
    assert_eq!(one("function F() -> (x: int) {\n}\n"), ["4:18: `x` is an output of `F` that nothing assigns"]);
    assert_eq!(one("function F(a: int) {\n  a = 2\n}\n"), ["5:3: `a` is an input of `F`, read only"]);
    assert_eq!(one("function F(a: int) {\n  for a in 1 .. 2 { }\n}\n"), ["5:7: `a` is an input of `F`, read only"]);
    assert_eq!(one("function F() {\n  wait status.running == 1\n}\n"), ["5:8: `status` is not among `F`'s `reads`"]);
    assert_eq!(one("function F() {\n  o = await outcome\n}\n"), ["5:13: `outcome` is not among `F`'s `reads`"]);
    assert_eq!(one("function F() {\n  reads status\n  reads outcome\n}\n"), ["6:3: `reads` is given twice"]);
    assert_eq!(one("function F() {\n  timeout 1s\n  timeout 2s\n}\n"), ["6:3: `timeout` is given twice"]);
    assert_eq!(one("function min() {\n}\n"), ["4:10: `min` is a built-in call, not a function's name"]);
    assert_eq!(one("function step() {\n}\n"), ["4:10: `step` is a word of the language, not a name (a function's name)"]);
    assert_eq!(one("function F() {\n}\nfunction F() {\n}\n"), ["6:10: two functions are named `F`: the other is at f.fn:4:10"]);
    assert_eq!(one(""), ["4:1: the file's end where `function` was expected"]);
    // A function only reads.
    assert_eq!(one("function F() {\n  write plc_main.run := 1\n}\n"), ["5:3: `write` is not a statement: a function only reads"]);
    assert_eq!(one("function F() {\n  send sorter.command(code: 1)\n}\n"), ["5:3: `send` is not a statement: a function only reads"]);
    for w in ["report", "check", "stop", "read", "call", "script", "branch"] {
        assert_eq!(one(&format!("function F() {{\n  {w} 1\n}}\n")), [format!("5:3: `{w}` is a step's action, not a statement")], "{w}");
    }
    // Calls stay inside the file, never back to the caller.
    assert_eq!(one("function F() {\n  G()\n}\n"), ["5:4: `(` where `=`, `+=`, `-=`, `*=` or `/=` was expected"]);
    assert_eq!(one("function F() {\n  x = G()\n}\n"), ["5:7: `G` is no function of this file"]);
    assert_eq!(one("function F() -> (y: int) {\n  y = F()\n}\n"), ["4:10: `F` calls itself"]);
    assert_eq!(
        one("function A() -> (r: int) {\n  r = B()\n}\nfunction B() -> (r: int) {\n  if true { r = C() }\n}\nfunction C() -> (r: int) {\n  r = A()\n}\n"),
        ["4:10: `A` calls itself through `B`, `C`", "7:10: `B` calls itself through `C`, `A`", "10:10: `C` calls itself through `A`, `B`"]
    );
    assert_eq!(
        one("function F(a: int) -> (b: int, c: int) {\n  b = a\n  c = a\n}\nfunction G() {\n  x, y, z = F(a: 1)\n}\n"),
        ["9:9: `F` gives 2 outputs, not 3"]
    );
    assert_eq!(
        one("function F(a: int) -> (r: int) {\n  r = a\n}\nfunction G() {\n  x = F(b: 1, a: 1, a: 2)\n}\n"),
        ["8:9: `F` has no input `b`", "8:21: input `a` is given twice"]
    );
    assert_eq!(one("function F(a: int, b: int) -> (r: int) {\n  r = a\n}\nfunction G() {\n  x = F(a: 1)\n}\n"), ["8:7: `F` needs its input `b`"]);
    assert_eq!(one("function F() {\n}\nfunction G() {\n  x = F()\n}\n"), ["7:3: `F` gives 0 outputs, not 1"]);
}

// ---- statements -----------------------------------------------------------------------------------

#[test]
fn statements_assign_branch_loop_and_fail() {
    let text = format!(
        "{FHEAD}function F(n: int) -> (x: int) {{\n  reads t\n  x = 0; x += 2; x -= 1\n  x *= 3\n  x /= 1\n  for i in 1 .. n {{ x += i }}\n  if x > 3 {{ x = 3 }} else if x > 2 {{\n    x = 2\n  }}\n  else {{\n    fail \"never {{x}}\"\n  }}\n  wait 1s\n  wait t.y == 1 timeout 2s\n  r = await t where r.y == x timeout 1s\n  a, b = G()\n  fail\n}}\nfunction G() -> (a: int, b: int) {{\n  a = 1\n  b = 2\n}}\n"
    );
    let ff = check_functions(&text, "f.fn").unwrap();
    let body = &ff.functions[0].body;
    let kinds: Vec<&str> = body
        .iter()
        .map(|s| match &s.kind {
            StmtKind::Assign { op: AssignOp::Set, .. } => "=",
            StmtKind::Assign { op: AssignOp::Add, .. } => "+=",
            StmtKind::Assign { op: AssignOp::Sub, .. } => "-=",
            StmtKind::Assign { op: AssignOp::Mul, .. } => "*=",
            StmtKind::Assign { op: AssignOp::Div, .. } => "/=",
            StmtKind::For { .. } => "for",
            StmtKind::If { .. } => "if",
            StmtKind::Delay { .. } => "wait duration",
            StmtKind::Wait { .. } => "wait",
            StmtKind::Await { .. } => "await",
            StmtKind::Call { .. } => "call",
            StmtKind::Fail { .. } => "fail",
        })
        .collect();
    assert_eq!(kinds, ["=", "+=", "-=", "*=", "/=", "for", "if", "wait duration", "wait", "await", "call", "fail"]);
    let StmtKind::If { arms, otherwise } = &body[6].kind else { panic!() };
    assert_eq!(arms.len(), 2);
    assert!(matches!(&otherwise.as_ref().unwrap()[0].kind, StmtKind::Fail { reason: Some(_) }));
    assert!(matches!(&body[11].kind, StmtKind::Fail { reason: None }));
    // In a script box, a name assigned is the test's: read by the steps after it.
    let c = checked(&test_of("  let k = 2\n  step a script {\n    for i in 1 .. k { total = i }\n  }\n  step b check total == i\n"));
    assert_eq!(c.counts.steps, 2);
    let one = |body: &str| problems(&test_of(body));
    assert_eq!(one("  step a check ghost == 1\n"), ["6:16: `ghost` is read but nothing assigns it"]);
    assert_eq!(one("  let items = 1\n  step a script { items = 2 }\n"), ["7:19: `items` is a constant (`let`) and is never assigned"]);
    assert_eq!(one("  let items = 1\n  step a read t.x -> items\n"), ["7:22: `items` is a constant (`let`) and is never assigned"]);
    assert_eq!(one("  step a script { x = status.running }\n"), ["6:23: a script box reads no plant data: `status.running`"]);
    assert_eq!(one("  step a script { wait 1s }\n"), ["6:19: a script box does not wait: `wait`"]);
    assert_eq!(one("  step a script { wait x == 1 }\n"), ["6:19: a script box does not wait: `wait`"]);
    assert_eq!(one("  step a script { x = await t }\n"), ["6:19: a script box does not wait: `await`"]);
    assert_eq!(one("  step a script { x = F(a: 1) }\n"), ["6:23: a script box calls no function: `F` is called by a step (`call`)"]);
    assert_eq!(one("  step a script { write t.x := 1 }\n"), ["6:19: `write` is a step's action, not a statement"]);
    assert_eq!(one("  step a script { await t }\n"), ["6:19: an `await` is assigned: `<name> = await <table> ...`"]);
    assert_eq!(one("  step a script { x = 1 y = 2 }\n"), ["6:25: `y` where a line's end was expected"]);
    assert_eq!(one("  step a script { x = 1\n"), ["8:1: the file's end where `step` or the test's closing `}` was expected"], "the test's brace closes the box");
    assert_eq!(one("  step a script { for i in 1 to 3 { } }\n"), ["6:30: `to` where `..` was expected"]);
}

// ---- calls ------------------------------------------------------------------------------------------

#[test]
fn a_call_gives_every_input_once_and_takes_at_most_the_outputs() {
    let f = format!("{FHEAD}function F(a: int, b: int) -> (x: int, y: int) {{\n  x = a\n  y = b\n}}\n");
    let one = |call: &str| problems_with(&test_of(&format!("  use f.fn\n  step c call {call}\n")), &[("f.fn", &f)]);
    assert_eq!(one("F(a: 1, b: 2) -> x, y"), Vec::<String>::new());
    assert_eq!(one("F(b: 2, a: 1) -> x"), Vec::<String>::new(), "inputs by name, in any order; fewer names keep the first outputs");
    assert_eq!(one("F(a: 1, b: 2)"), Vec::<String>::new());
    assert_eq!(one("G(a: 1)"), ["7:15: `G` is no function of the files this test uses"]);
    assert_eq!(one("F(a: 1)"), ["7:15: `F` needs its input `b`"]);
    assert_eq!(one("F(a: 1, b: 2, a: 3)"), ["7:29: input `a` is given twice"]);
    assert_eq!(one("F(a: 1, b: 2, c: 3)"), ["7:29: `F` has no input `c`"]);
    assert_eq!(one("F(a: 1, b: 2) -> x, y, z"), ["7:38: `F` gives 2 outputs, not 3"]);
    assert_eq!(one("F(a: 1, b: ghost)"), ["7:26: `ghost` is read but nothing assigns it"]);
}

// ---- expressions ------------------------------------------------------------------------------------

#[test]
fn operators_bind_loosest_first_and_names_settle() {
    let c = checked(&test_of("  let a = 1\n  step s check a == 1 or a > 2 and not a == 3 + 4 * -5 % 2 - 1\n"));
    let ActionKind::Check { condition, .. } = &c.test.steps[0].actions[0].kind else { panic!() };
    fn shape(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(i) => i.to_string(),
            ExprKind::Name(n) => n.clone(),
            ExprKind::Unary { op: UnOp::Neg, expr } => format!("(-{})", shape(expr)),
            ExprKind::Unary { op: UnOp::Not, expr } => format!("(not {})", shape(expr)),
            ExprKind::Binary { op, left, right } => {
                let o = match op {
                    BinOp::Or => "or",
                    BinOp::And => "and",
                    BinOp::Cmp(c) => c.text(),
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Rem => "%",
                };
                format!("({} {o} {})", shape(left), shape(right))
            }
            other => format!("{other:?}"),
        }
    }
    assert_eq!(shape(condition), "((a == 1) or ((a > 2) and (not (a == ((3 + ((4 * (-5)) % 2)) - 1)))))");
    let one = |e: &str| problems(&test_of(&format!("  let a = 1\n  step s check {e}\n")));
    assert_eq!(one("(a + 1) * 2 == 4"), Vec::<String>::new());
    assert_eq!(one("1 < a < 3"), ["7:22: two comparisons in a row: join them with `and`"]);
    assert_eq!(one("t.c.d == 1"), ["7:16: `t.c.d`: a name reads one column"]);
    assert_eq!(one("step == 1"), ["7:16: `step` where a value was expected"]);
    assert_eq!(one("abs(a) + min(a, 2) + max(a) + round(a) + round(a, 2) + seconds(1s) + int(a) + float(a) + 1 == 2"), Vec::<String>::new());
    assert_eq!(one("text(a) == \"1\" and now() == now()"), Vec::<String>::new());
    assert_eq!(one("abs(1, 2) == 1"), ["7:16: `abs` takes 1 value, not 2"]);
    assert_eq!(one("round(1, 2, 3) == 1"), ["7:16: `round` takes 1 or 2 values, not 3"]);
    assert_eq!(one("now(1) == 1"), ["7:16: `now` takes no value, not 1"]);
    assert_eq!(one("min() == 1"), ["7:16: `min` takes one value or more, not 0"]);
    assert_eq!(one("foo(1) == 1"), ["7:16: `foo` is no built-in call (abs, min, max, round, seconds, now, int, float, text)"]);
    let f = format!("{FHEAD}function F() {{\n}}\n");
    assert_eq!(
        problems_with(&test_of("  use f.fn\n  step s check F() == 1\n"), &[("f.fn", &f)]),
        ["7:16: `F` is a function: a step calls it with `call`, a function by `<outputs> = F(...)`"]
    );
}

#[test]
fn a_name_that_is_no_variable_reads_plant_data_and_a_variable_s_reads_its_row() {
    let c = checked(&test_of("  step a await announce -> row\n  step b check row.lane == plc_main.lane\n"));
    let ActionKind::Check { condition, .. } = &c.test.steps[1].actions[0].kind else { panic!() };
    let ExprKind::Binary { left, right, .. } = &condition.kind else { panic!() };
    assert!(matches!(&left.kind, ExprKind::Field { base, .. } if base.value == "row"));
    assert!(matches!(&right.kind, ExprKind::Plant { table, column } if table.value == "plc_main" && column.value == "lane"));
}

#[test]
fn a_function_sees_its_own_names_and_not_the_test_s() {
    let f = format!("{FHEAD}function F() -> (r: int) {{\n  r = k\n}}\n");
    assert_eq!(
        problems_with(&test_of("  use f.fn\n  let k = 1\n  step a call F() -> r\n"), &[("f.fn", &f)]),
        ["f.fn:5:7: `k` is read but nothing assigns it"]
    );
}

// ---- refute round 1 (each finding with its neighbours) ----------------------------------------

#[test]
fn a_duration_is_read_exactly() {
    let one = |lit: &str| -> Result<i64, String> {
        match check_with(&test_of(&format!("  let d = {lit}\n  step x stop pass\n")), &[]) {
            Ok(c) => match c.test.lets()[0].1.kind {
                ExprKind::Duration(ms) => Ok(ms),
                ref other => Err(format!("{other:?}")),
            },
            Err(p) => Err(p[0].message.clone()),
        }
    };
    // Every millisecond of the first three seconds, written with three places.
    for n in 0..3 {
        for d in 0..1000 {
            assert_eq!(one(&format!("{n}.{d:03}s")), Ok(n * 1000 + d), "{n}.{d:03}s");
        }
    }
    assert_eq!(one("1.001s"), Ok(1001));
    assert_eq!(one("0.0011h"), Ok(3960));
    assert_eq!(one("1e-3s"), Ok(1));
    assert_eq!(one("2.5e-1min"), Ok(15_000));
    assert_eq!(one("1.5E1s"), Ok(15_000));
    assert_eq!(one("0.25ms").unwrap_err(), "`0.25ms` is not a whole number of milliseconds");
    assert_eq!(one("1.0005s").unwrap_err(), "`1.0005s` is not a whole number of milliseconds");
    assert_eq!(one("1e40h").unwrap_err(), "`1e40h` is past a duration's range");
    assert_eq!(one("5e-400s").unwrap_err(), "`5e-400s` is not a whole number of milliseconds");
    assert_eq!(one("0e-400s"), Ok(0));
}

#[test]
fn another_header_key_is_refused_by_its_name_at_its_line_whatever_its_value() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("format = \"veredas-test\"\nversion = 1\n{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    assert_eq!(one("owner = 99999999999999999999\n"), "3:1: the header holds `format` and `version` alone, not `owner`");
    assert_eq!(one("x = -9223372036854775809\n"), "3:1: the header holds `format` and `version` alone, not `x`");
    assert_eq!(one("\n# a note\n  [extra]\n"), "5:3: the header holds `format` and `version` alone, not `extra`");
    assert_eq!(one("[[many]]\n"), "3:1: the header holds `format` and `version` alone, not `many`");
    assert_eq!(one("meta.x = 1\n"), "3:1: the header holds `format` and `version` alone, not `meta`");
    assert_eq!(one("\"name\" = 1\n"), "3:1: the header holds `format` and `version` alone, not `name`");
    // The stamp's own keys, quoted or commented, are the stamp.
    assert!(parse_test(&format!("\"format\" = \"veredas-test\" # the stamp\nversion = 1 # one\n{body}"), "t").is_ok());
}

#[test]
fn a_let_holds_no_call() {
    let one = |e: &str| problems(&test_of(&format!("  let k = {e}\n  step x stop pass\n")));
    assert_eq!(one("now()"), ["6:11: a `let` is made of literals and the constants above it: `now(...)`"]);
    assert_eq!(one("max(1, 2)"), ["6:11: a `let` is made of literals and the constants above it: `max(...)`"]);
    assert_eq!(one("-(1 + 2) * 3 % 2"), Vec::<String>::new());
}

#[test]
fn a_call_in_a_statement_is_a_function_s_when_its_inputs_are_named() {
    let f = format!("{FHEAD}function F() -> (r: int) {{\n  r = 1\n}}\n");
    let script = |body: &str| problems_with(&test_of(&format!("  use f.fn\n  let z = 1\n  step s script {{ {body} }}\n")), &[("f.fn", &f)]);
    assert_eq!(script("y = rnd(z)"), ["8:23: `rnd` is no built-in call (abs, min, max, round, seconds, now, int, float, text)"]);
    assert_eq!(script("y = round(z)"), Vec::<String>::new());
    assert_eq!(script("y = 1 + F()"), ["8:27: `F` is a function: a step calls it with `call`, a function by `<outputs> = F(...)`"]);
    assert_eq!(script("y = F()"), ["8:23: a script box calls no function: `F` is called by a step (`call`)"]);
    let one = |body: &str| fproblems(&format!("{FHEAD}function G(y: int) -> (x: int) {{\n  {body}\n}}\nfunction H(a: int) -> (r: int) {{\n  r = a\n}}\n"));
    assert_eq!(one("x = rnd(y)"), ["5:7: `rnd` is no built-in call (abs, min, max, round, seconds, now, int, float, text)"]);
    assert_eq!(one("x = H(a: y)"), Vec::<String>::new());
    assert_eq!(one("x = H(y)"), ["5:7: `H` is a function: a step calls it with `call`, a function by `<outputs> = H(...)`"]);
    assert_eq!(one("x = H()"), ["5:7: `H` needs its input `a`"]);
}

#[test]
fn a_hash_inside_a_string_s_braces_is_no_comment() {
    let one = |e: &str| problems(&test_of(&format!("  let a = 1\n  step x report {e}\n")));
    assert_eq!(one("\"v {a # oops}\""), ["7:23: `#` is no part of the language"]);
    assert_eq!(one("\"v {a #}\""), ["7:23: `#` is no part of the language"]);
    assert_eq!(one("\"v # {a}\""), Vec::<String>::new());
    assert_eq!(one("\"v {a}\" # a comment"), Vec::<String>::new());
}

#[test]
fn a_crlf_file_names_every_problem_at_the_place_its_lf_twin_does() {
    let cases = [
        "  step a check 1 +\n    2 == 3\n",
        "  step a await announce where a.lane == 1\n",
        "  step a wait 1s\n         on timeout -> nowhere\n",
        "  step a stop pass\n  area late\n",
        "  step a script { x = 1 y = 2 }\n",
        "  step a branch 1 {\n  }\n",
    ];
    for c in cases {
        let lf = test_of(c);
        let crlf = lf.replace('\n', "\r\n");
        let (a, b) = (problems(&lf), problems(&crlf));
        assert!(!a.is_empty(), "{c:?}");
        assert_eq!(a, b, "{c:?}");
    }
    // A setting's rest of the line stops before its carriage return.
    let c = checked(&format!("{HEAD}test \"t\" {{\r\n  id T\r\n  area out\r\n  step a stop pass\r\n}}\r\n").replace("\n\n", "\r\n\r\n"));
    assert_eq!(c.test.area(), Some("out"));
}

#[test]
fn a_field_or_an_input_named_as_a_word_of_the_language_is_backquoted() {
    assert_eq!(
        problems(&test_of("  step x send s.m(timeout: 1)\n")),
        ["6:19: `timeout` is a word of the language, not a name (a name and `:`)"]
    );
    let c = checked(&test_of("  step x send sorter.announce(`read`: \"N\", item: 1)\n"));
    let ActionKind::Send { fields, .. } = &c.test.steps[0].actions[0].kind else { panic!() };
    assert_eq!(fields.iter().map(|f| f.name.value.as_str()).collect::<Vec<_>>(), ["read", "item"]);
    assert!(prose().contains("a table, a channel or a message's field whose name is one of them is written in backquotes"));
    assert!(prose().contains("A function's inputs are named by its author, never one of them."));
    assert_eq!(fproblems(&format!("{FHEAD}function G(`read`: int) {{\n}}\n")), ["4:12: ``read`` where a port's name was expected"]);
}

#[test]
fn nesting_past_its_limits_is_named_not_a_crash() {
    let one = |e: &str| problems(&test_of(&format!("  step x check {e}\n")));
    let parens = |n: usize| format!("{}1{} == 1", "(".repeat(n), ")".repeat(n));
    assert_eq!(one(&parens(64)), Vec::<String>::new(), "64 levels read");
    assert_eq!(one(&parens(65)), ["6:80: blocks and expressions nested deeper than 64, counted together"]);
    assert_eq!(one(&parens(3000)), ["6:80: blocks and expressions nested deeper than 64, counted together"]);
    assert_eq!(one(&format!("{}true", "not ".repeat(20_000))), ["6:272: blocks and expressions nested deeper than 64, counted together"]);
    assert_eq!(one(&format!("{}1", "-".repeat(1000))), ["6:80: blocks and expressions nested deeper than 64, counted together"]);
    assert_eq!(one(&format!("abs({}1{}) == 1", "abs(".repeat(70), ")".repeat(70))), ["6:275: blocks and expressions nested deeper than 64, counted together"]);
    let chain = |n: usize| format!("{} == 1", vec!["1"; n].join(" + "));
    assert_eq!(one(&chain(1000)), Vec::<String>::new(), "a chain of 999 additions and a comparison");
    assert_eq!(one(&chain(1001)), ["6:4018: an expression of more than 1000 operators"]);
    assert_eq!(one(&chain(100_000)), ["6:4018: an expression of more than 1000 operators"]);
    let blocks = |n: usize| format!("  step x script {{ {}x = 1{} }}\n", "if true { ".repeat(n), " }".repeat(n));
    assert_eq!(problems(&test_of(&blocks(63))), Vec::<String>::new());
    assert_eq!(problems(&test_of(&blocks(64))), ["6:657: blocks and expressions nested deeper than 64, counted together"]);
    assert_eq!(problems(&test_of(&blocks(5000))), ["6:657: blocks and expressions nested deeper than 64, counted together"]);
}

// ---- refute round 2 ------------------------------------------------------------------------

/// Run on a thread with a 2 MB stack, a spawned thread's default, as a designer's reader runs.
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(2 << 20).spawn(f).unwrap().join().unwrap()
}

/// The column of the `n`-th `needle` in `line`, counted in characters from 1.
fn column_of(line: &str, needle: char, n: usize) -> usize {
    line.chars().enumerate().filter(|(_, c)| *c == needle).nth(n - 1).map(|(i, _)| i + 1).unwrap()
}

#[test]
fn the_operator_limit_holds_in_patterns_ranges_parentheses_and_strings() {
    small_stack(|| {
        let terms = |n: usize, op: &str| vec!["(1)"; n].join(op);
        // A pattern of 1001 operators, and of 50 000: refused at the 1001st, the stack spared.
        for n in [1002usize, 50_000] {
            let line = format!("    == {} -> x", terms(n, "+"));
            let text = test_of(&format!("  step x branch 1 {{\n{line}\n  }}\n"));
            assert_eq!(problems(&text), [format!("7:{}: an expression of more than 1000 operators", column_of(&line, '+', 1001))]);
        }
        let line = format!("    == {} -> x", terms(1001, "+"));
        assert_eq!(problems(&test_of(&format!("  step x branch 1 {{\n{line}\n  }}\n"))), Vec::<String>::new(), "1000 operators read");
        // A range's end, and a `for`'s.
        let line = format!("  step x script {{ for i in {} .. 2 {{ }} }}", terms(50_000, "*"));
        assert_eq!(problems(&test_of(&format!("{line}\n"))), [format!("6:{}: an expression of more than 1000 operators", column_of(&line, '*', 1001))]);
        // Operators inside parentheses count with the expression around them.
        let line = format!("  step x check {} == 1", vec!["(1+1)"; 2000].join("+"));
        assert_eq!(problems(&test_of(&format!("{line}\n"))), [format!("6:{}: an expression of more than 1000 operators", column_of(&line, '+', 1001))]);
        // A string's `{ }` counts with the expression it sits in: operators and levels.
        let piece = format!("{{{}}}", vec!["1"; 251].join("+"));
        let line = format!("  step x report \"{}\"", piece.repeat(5));
        assert_eq!(problems(&test_of(&format!("{line}\n"))), [format!("6:{}: an expression of more than 1000 operators", column_of(&line, '+', 1001))]);
        let line = format!("  step x report {}\"{{{}1{}}}\"{}", "(".repeat(40), "(".repeat(40), ")".repeat(40), ")".repeat(40));
        assert_eq!(problems(&test_of(&format!("{line}\n"))), [format!("6:{}: blocks and expressions nested deeper than 64, counted together", column_of(&line, '(', 65))]);
    });
}

#[test]
fn a_duration_s_exponent_is_read_without_a_loop_over_it() {
    let one = |lit: &str| -> Result<i64, String> {
        match check_with(&test_of(&format!("  let d = {lit}\n  step x stop pass\n")), &[]) {
            Ok(c) => match c.test.lets()[0].1.kind {
                ExprKind::Duration(ms) => Ok(ms),
                ref other => Err(format!("{other:?}")),
            },
            Err(p) => Err(p[0].message.clone()),
        }
    };
    assert_eq!(one("0e-2147483648ms"), Ok(0));
    assert_eq!(one("0e99999999999999h"), Ok(0));
    assert_eq!(one("1.0000000000000000000000000000000000000000s"), Ok(1000), "trailing zeros are no digits");
    assert_eq!(one("0001.5000s"), Ok(1500));
    assert_eq!(one("1e-99999999999s").unwrap_err(), "`1e-99999999999s` is not a whole number of milliseconds");
    assert_eq!(one("1e99999999999s").unwrap_err(), "`1e99999999999s` is past a duration's range");
    assert_eq!(
        one("0.0000000000000000000000000000000000000001ms").unwrap_err(),
        "`0.0000000000000000000000000000000000000001ms` is not a whole number of milliseconds"
    );
    // An hour is 36 times a power of ten: the fractions that make whole milliseconds read.
    assert_eq!(one("1e-5h"), Ok(36));
    assert_eq!(one("5e-6h"), Ok(18));
    assert_eq!(one("25e-7h"), Ok(9));
    assert_eq!(one("36e-1h"), Ok(12_960_000));
    assert_eq!(one("1e-6h").unwrap_err(), "`1e-6h` is not a whole number of milliseconds");
    assert_eq!(one("1.5e-3s").unwrap_err(), "`1.5e-3s` is not a whole number of milliseconds");
    assert_eq!(one("5e-1min"), Ok(30_000));
    // The range's end: half the largest whole number of milliseconds.
    assert_eq!(one("4611686018427387903ms"), Ok(4_611_686_018_427_387_903));
    assert_eq!(one("4611686018427387904ms").unwrap_err(), "`4611686018427387904ms` is past a duration's range");

    // No loop over the exponent, against a baseline measured here, so a loaded machine slows both
    // alike: three literals of one length, read the three ways a duration is (zero, not whole,
    // past the range), with exponents of ten, then of a billion. Read 5 000 times each, the best of
    // three rounds on the thread's CPU, the second costs what the first does (the exponent's text
    // is as long; its value is never counted out), where a loop over a billion would cost
    // thousands of times more. A round stops once past the bound, so such a loop fails in seconds.
    use veredas_comissionador::lex::{Tok, lex};
    let small = "0e0000000010ms 1e-0000000010ms 1e0000000030ms";
    let large = "0e1000000000ms 1e-1000000000ms 1e1000000000ms";
    let read = |text: &str| lex(text, 0).into_iter().map(|t| t.tok).collect::<Vec<_>>();
    let read_as = |text: &str| {
        let w: Vec<&str> = text.split(' ').collect();
        vec![
            Tok::Duration(0),
            Tok::Bad(format!("`{}` is not a whole number of milliseconds", w[1])),
            Tok::Bad(format!("`{}` is past a duration's range", w[2])),
            Tok::Eof,
        ]
    };
    assert_eq!((read(small), read(large)), (read_as(small), read_as(large)));
    const TIMES: usize = 5_000;
    const BOUND: u32 = 10;
    let mut best: Option<(std::time::Duration, std::time::Duration)> = None;
    for _ in 0..3 {
        // Both sides look at the clock after every run, so the looking costs them alike.
        let base = cost(TIMES, std::time::Duration::MAX, &|| drop(read(small))).unwrap();
        let Some(huge) = cost(TIMES, base * BOUND, &|| drop(read(large))) else { continue };
        best = Some(best.map_or((base, huge), |(b, h)| (b.min(base), h.min(huge))));
    }
    let (base, huge) = best.unwrap_or_else(|| panic!("an exponent of a billion cost over {BOUND} times one of ten in every round"));
    assert!(huge <= base * BOUND, "an exponent of a billion: {huge:?}, of ten: {base:?}");
}

#[test]
fn a_call_statement_breaks_its_lines_inside_its_parentheses() {
    let f = |body: &str| fproblems(&format!("{FHEAD}function G(a: int) -> (r: int) {{\n  r = a\n}}\nfunction H() -> (y: int) {{\n  {body}\n}}\n"));
    assert_eq!(f("y = G(\n    a: 1)"), Vec::<String>::new());
    assert_eq!(f("y = G(\n    a: 1,\n  )"), ["10:3: `)` where a name and `:` was expected"]);
    assert_eq!(f("y, z = G(\n    a: 1)"), ["8:6: `G` gives 1 output, not 2"]);
    assert_eq!(f("y = G(\n  )"), ["8:7: `G` needs its input `a`"]);
}

#[test]
fn a_byte_order_mark_and_a_multi_line_stamp_are_the_stamp() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    assert!(parse_test(&format!("\u{feff}format = \"veredas-test\"\nversion = 1\n{body}"), "t").is_ok());
    assert!(parse_test(&format!("format = \"\"\"\nveredas-test\"\"\"\nversion = 1\n{body}"), "t").is_ok());
    let e = parse_test(&format!("\u{feff}format = \"veredas-test\"\nversion = 1\nname = 1\n{body}"), "t").unwrap_err();
    assert_eq!((e.line, e.column, e.message.as_str()), (3, 1, "the header holds `format` and `version` alone, not `name`"));
    let e = parse_test(&format!("format = \"veredas-test\"\nversion = 1\nnote = '''\nformat\n'''\n{body}"), "t").unwrap_err();
    assert_eq!((e.line, e.message.as_str()), (3, "the header holds `format` and `version` alone, not `note`"));
}

#[test]
fn a_comment_before_a_crlf_line_end_keeps_the_columns_of_its_lf_twin() {
    for c in ["  step a send # c\n", "  step a check 1 == # c\n", "  step a wait 1s # fine\n         on late -> a\n"] {
        let lf = test_of(c);
        let (a, b) = (problems(&lf), problems(&lf.replace('\n', "\r\n")));
        assert!(!a.is_empty(), "{c:?}");
        assert_eq!(a, b, "{c:?}");
    }
}

// ---- refute round 3 ------------------------------------------------------------------------

#[test]
fn a_quote_run_in_a_header_comment_hides_no_key() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    assert_eq!(one("format = \"veredas-test\"\nversion = 1 # it's ''' here\nextra = 2\n"), "3:1: the header holds `format` and `version` alone, not `extra`");
    assert_eq!(one("# a ''' in a comment\nformat = \"veredas-test\"\nversion = 1\nextra = 2\n"), "4:1: the header holds `format` and `version` alone, not `extra`");
    assert_eq!(one("format = \"veredas-test\" # \"\"\"\nversion = 1\n[meta]\nx = 1\n"), "3:1: the header holds `format` and `version` alone, not `meta`");
    assert_eq!(one("format = \"\"\"\nveredas-test\"\"\" # \"\"\"\nversion = 1\nextra = 2\n"), "4:1: the header holds `format` and `version` alone, not `extra`");
    // A quote run inside a single-line string is the string's; an escaped quote stays inside it.
    assert_eq!(one("format = \"veredas-test\"\nversion = 1\nnote = \"a ''' b \\\" c\"\nextra = 2\n"), "3:1: the header holds `format` and `version` alone, not `note`");
    // A multi-line literal string closed on its own line, then a key: named.
    assert_eq!(one("format = \"veredas-test\"\nversion = 1\nnote = '''\nx\n'''\nextra = 2\n"), "3:1: the header holds `format` and `version` alone, not `note`");
    assert!(parse_test(&format!("format = \"\"\"\nveredas-test\"\"\" # a comment\nversion = 1 # one\n{body}"), "t").is_ok());
}

#[test]
fn a_long_call_chain_and_a_long_cycle_are_checked_at_once() {
    let chain = |n: usize, back: bool| {
        let mut f = String::from(FHEAD);
        for i in 0..n {
            let next = if i + 1 < n { format!("F{}()", i + 1) } else if back { "F0()".to_string() } else { "1".to_string() };
            f.push_str(&format!("function F{i}() -> (x: int) {{\n  x = {next}\n}}\n"));
        }
        f
    };
    // A chain of calls and a file of a problem a function, each checked in a time linear in its
    // length (1 000 functions then 8 000, 2 000 then 16 000), where a quadratic check took seconds;
    // a cycle through every function in a time no worse than the square of its length (300, then
    // 2 400), where the cubic check took minutes.
    let calls = |n: usize| {
        let text = chain(n, false);
        let mut ok = false;
        let took = on_cpu(|| ok = check_functions(&text, "f.fn").is_ok());
        assert!(ok, "a chain of {n}");
        took
    };
    let cycle = |n: usize| {
        let text = chain(n, true);
        let mut e = Vec::new();
        let took = on_cpu(|| e = check_functions(&text, "f.fn").unwrap_err());
        assert_eq!(e.len(), n, "each function on the cycle named once");
        assert_eq!(e[0].message, format!("`F0` calls itself through `F1`, `F2`, `F3`, `F4`, `F5` and {} others", n - 6));
        assert_eq!(e[n - 1].message, format!("`F{}` calls itself through `F0`, `F1`, `F2`, `F3`, `F4` and {} others", n - 1, n - 6));
        took
    };
    let ghosts = |n: usize| {
        let mut f = String::from(FHEAD);
        for i in 0..n {
            f.push_str(&format!("function G{i}() -> (x: int) {{\n  x = ghost{i}\n}}\n"));
        }
        let mut e = Vec::new();
        let took = on_cpu(|| e = check_functions(&f, "f.fn").unwrap_err());
        assert_eq!(e.len(), n);
        let last = &e[n - 1];
        assert_eq!((last.line, last.column, last.message.clone()), (4 + 3 * (n - 1) + 1, 7, format!("`ghost{}` is read but nothing assigns it", n - 1)));
        took
    };
    linear(&[("a chain of calls", 1_000, &calls), ("a problem a function", 2_000, &ghosts)]);
    quadratic(&[("a cycle through every function", 300, &cycle)]);
}

#[test]
fn blocks_and_expressions_share_one_depth_on_a_small_stack() {
    small_stack(|| {
        // Blocks and levels together at the limit, an expression of 999 operators at the bottom.
        for blocks in [63usize, 32, 1] {
            let levels = 64 - blocks - 1;
            let sum = vec!["1"; 1000].join("+");
            let line = format!("  step x script {{ {}x = {}{sum}{}{} }}", "if 1 == 1 { ".repeat(blocks - 1), "(".repeat(levels), ")".repeat(levels), " }".repeat(blocks - 1));
            assert_eq!(problems(&test_of(&format!("{line}\n"))), Vec::<String>::new(), "{blocks} blocks, {levels} levels");
        }
        // One level past it, wherever it falls: named, not a crash.
        let line = format!("  step x script {{ {}x = {}1{} }}", "if 1 == 1 { ".repeat(40), "(".repeat(24), ")".repeat(24), );
        let line = format!("{line}{}", " }".repeat(40));
        assert_eq!(problems(&test_of(&format!("{line}\n"))), [format!("6:{}: blocks and expressions nested deeper than 64, counted together", column_of(&line, '(', 24))]);
        let line = format!("  step x script {{ {}x = 1{} }}", "if 1 == 1 { ".repeat(64), " }".repeat(64));
        assert_eq!(problems(&test_of(&format!("{line}\n"))), [format!("6:{}: blocks and expressions nested deeper than 64, counted together", column_of(&line, '{', 65))]);
    });
}

#[test]
fn an_unfinished_expression_in_a_string_s_braces_names_its_brace() {
    let one = |e: &str| problems(&test_of(&format!("  step x script {{ z = {e} }}\n")));
    assert_eq!(one("\"{1 +}\""), ["6:28: `}` where a value was expected"]);
    assert_eq!(one("\"{(1}\""), ["6:27: `}` where `)` was expected"]);
}

// ---- refute round 4 ------------------------------------------------------------------------

#[test]
fn a_character_past_ascii_in_the_header_is_read_whole() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("format = \"veredas-test\"\nversion = 1\n{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    for v in ["\"\"\"olá\"\"\"", "'''olá'''", "\"\"\"\nolá ç\n\"\"\"", "\"olá\"", "'olá'"] {
        assert_eq!(one(&format!("note = {v}\n")), "3:1: the header holds `format` and `version` alone, not `note`", "{v}");
    }
    assert!(parse_test(&format!("# olá, um comentário\nformat = \"veredas-test\" # ç\nversion = 1\n{body}"), "t").is_ok());
    // A used function file's header the same, through the test's check.
    let f = "format = \"veredas-functions\"\nversion = 1\nnote = \"\"\"olá\"\"\"\nfunction F() {\n}\n";
    assert_eq!(problems_with(&test_of("  use f.fn\n  step a stop pass\n"), &[("f.fn", f)]), ["f.fn:3:1: the header holds `format` and `version` alone, not `note`"]);
}

#[test]
fn a_quoted_header_key_is_read_to_its_closing_quote() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("format = \"veredas-test\"\nversion = 1\n{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    assert_eq!(one("\"format=x\" = 2\n"), "3:1: the header holds `format` and `version` alone, not `format=x`");
    assert_eq!(one("'version = 9' = 2\n"), "3:1: the header holds `format` and `version` alone, not `version = 9`");
    assert_eq!(one("\"a\\\"b\" = 2\n"), "3:1: the header holds `format` and `version` alone, not `a\"b`");
    // A dotted key is named by its first part, the table it makes; a quoted key holding a dot is
    // one key.
    assert_eq!(one("stamp.format = 2\n"), "3:1: the header holds `format` and `version` alone, not `stamp`");
    assert_eq!(one("\"version.x\" = 2\n"), "3:1: the header holds `format` and `version` alone, not `version.x`");
    // A dotted key under the stamp's own name is no TOML: core's reader refuses it first.
    assert!(one("format.x = 2\n").starts_with("3:1: not a test file: TOML parse error at line 3, column 1"));
    for stamp in ["\"format\" = \"veredas-test\"\n'version' = 1\n", "format=\"veredas-test\"\nversion=1\n", "\"format\"=\"veredas-test\"\nversion =1\n"] {
        assert!(parse_test(&format!("{stamp}{body}"), "t").is_ok(), "{stamp}");
    }
}

#[test]
fn a_function_named_twice_is_its_first_in_the_cycle_search() {
    // A real cycle through the first of two `F`s: named, beside the duplicate.
    let a = fproblems(&format!("{FHEAD}function F() -> (x: int) {{\n  x = G()\n}}\nfunction G() -> (x: int) {{\n  x = F()\n}}\nfunction F() -> (x: int) {{\n  x = 1\n}}\n"));
    assert_eq!(a, ["10:10: two functions are named `F`: the other is at f.fn:4:10", "4:10: `F` calls itself through `G`", "7:10: `G` calls itself through `F`"]);
    // No cycle where the second `F` would make one: the call to `F` reaches the first.
    let b = fproblems(&format!("{FHEAD}function F() -> (x: int) {{\n  x = 1\n}}\nfunction G() -> (x: int) {{\n  x = F()\n}}\nfunction F() -> (x: int) {{\n  x = G()\n}}\n"));
    assert_eq!(b, ["10:10: two functions are named `F`: the other is at f.fn:4:10"]);
}

#[test]
fn a_way_back_of_five_and_six_is_counted_right() {
    let file = |n: usize| {
        let mut f = String::from(FHEAD);
        for i in 0..n {
            f.push_str(&format!("function F{i}() -> (x: int) {{\n  x = F{}()\n}}\n", (i + 1) % n));
        }
        f
    };
    let cycle = |n: usize| fproblems(&file(n));
    assert_eq!(cycle(6)[0], "4:10: `F0` calls itself through `F1`, `F2`, `F3`, `F4`, `F5`");
    assert_eq!(cycle(7)[0], "4:10: `F0` calls itself through `F1`, `F2`, `F3`, `F4`, `F5` and 1 other");
    assert_eq!(cycle(8)[0], "4:10: `F0` calls itself through `F1`, `F2`, `F3`, `F4`, `F5` and 2 others");
    // Each way back counted in a time no worse than the square of the cycle's length: 300
    // functions, then 2 400.
    let long = |n: usize| {
        let f = file(n);
        let mut e = Vec::new();
        let took = on_cpu(|| e = fproblems(&f));
        assert_eq!(e.len(), n);
        took
    };
    quadratic(&[("a cycle through every function", 300, &long)]);
}

#[test]
fn a_test_using_a_large_function_file_is_checked_at_once() {
    // A used file of many functions, and one of many functions of one name: each checked in a time
    // linear in the file, 4 000 functions then 32 000, 2 000 then 16 000.
    let t = test_of("  use f.fn\n  step a stop pass\n");
    let used = |n: usize| {
        let mut f = String::from(FHEAD);
        for i in 0..n {
            f.push_str(&format!("function G{i}() -> (x: int) {{\n  x = {i}\n}}\n"));
        }
        let mut p = Vec::new();
        let took = on_cpu(|| p = problems_with(&t, &[("f.fn", &f)]));
        assert_eq!(p, Vec::<String>::new());
        took
    };
    let alike = |n: usize| {
        let mut d = String::from(FHEAD);
        for _ in 0..n {
            d.push_str("function F() -> (x: int) {\n  x = 1\n}\n");
        }
        let mut p = Vec::new();
        let took = on_cpu(|| p = problems_with(&t, &[("f.fn", &d)]));
        assert_eq!(p.len(), n - 1);
        assert_eq!(p[n - 2], format!("f.fn:{}:10: two functions are named `F`: the other is at f.fn:4:10", 4 + 3 * (n - 1)));
        took
    };
    linear(&[("a used file of many functions", 4_000, &used), ("a used file of functions of one name", 2_000, &alike)]);
}

// ---- refute round 5 ------------------------------------------------------------------------

#[test]
fn a_header_key_is_read_with_its_escapes_as_toml_reads_it() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("format = \"veredas-test\"\nversion = 1\n{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    // An escape that would make the stamp's name hides no key: the key is what TOML decodes, a
    // control character in it shown by its escape.
    let not = |name: &str| format!("3:1: the header holds `format` and `version` alone, not `{name}`");
    assert_eq!(one("\"\\format\" = 1\n"), not("\\u{c}ormat"));
    assert_eq!(one("\"versio\\n\" = 1\n"), not("versio\\n"));
    assert_eq!(one("\"forma\\t\" = { a = 1 }\n"), not("forma\\t"));
    assert_eq!(one("\"fo\\rmat\" = [1, 2]\n"), not("fo\\rmat"));
    assert_eq!(one("\"versio\\n\" = 18446744073709551616\n"), not("versio\\n"));
    assert_eq!(one("\"ve\\rsion\" = \"x\"\n"), not("ve\\rsion"));
    assert_eq!(one("\"a\\tb\" = 1\n"), not("a\\tb"));
    // An escape of an ordinary letter is that letter: the stamp written so is the stamp.
    for stamp in ["\"form\\u0061t\" = \"veredas-test\"\nversion = 1\n", "format = \"veredas-test\"\n\"\\u0076ersion\" = 1\n"] {
        assert!(parse_test(&format!("{stamp}{body}"), "t").is_ok(), "{stamp}");
    }
    // In a used function file the same, through the test's check.
    let f = "format = \"veredas-functions\"\nversion = 1\n\"\\format\" = 1\nfunction F() {\n}\n";
    assert_eq!(problems_with(&test_of("  use f.fn\n  step a stop pass\n"), &[("f.fn", f)]), ["f.fn:3:1: the header holds `format` and `version` alone, not `\\u{c}ormat`"]);
}

#[test]
fn a_dotted_or_table_key_with_quotes_is_named_as_toml_reads_it() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("format = \"veredas-test\"\nversion = 1\n{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    let not = |at: &str, name: &str| format!("{at}: the header holds `format` and `version` alone, not `{name}`");
    assert_eq!(one("'a=b'.c = 2\n"), not("3:1", "a=b"));
    assert_eq!(one("[\"a]b\"]\n"), not("3:1", "a]b"));
    assert_eq!(one("[[\"x]]y\".z]]\n"), not("3:1", "x]]y"));
    assert_eq!(one("  \"p.q\" . r = 2\n"), not("3:3", "p.q"));
}

#[test]
fn problems_sharing_one_line_are_placed_at_once() {
    // Positions for no step on one line, ASCII and past it (a comment at its end): each placed, the
    // last at its column, in a time linear in the line: 20 000 positions, then 160 000.
    let placed = |tail: &'static str| {
        move |n: usize| {
            let line = format!("layout {{{}}}{tail}", " a 0,0".repeat(n));
            let text = format!("{}{line}\n", test_of("  step b stop pass\n"));
            let mut e = Vec::new();
            let took = on_cpu(|| e = check_with(&text, &[]).unwrap_err());
            assert_eq!(e.len(), n, "{tail:?}");
            let last = e.iter().max_by_key(|p| p.column).unwrap();
            assert_eq!((last.line, last.column), (8, 8 + 6 * (n - 1) + 2), "{tail:?}: {}", last.message);
            took
        }
    };
    let (ascii, past) = (placed(""), placed(" # olá, ç"));
    linear(&[("a line of ASCII", 20_000, &ascii), ("a line past ASCII", 20_000, &past)]);
}

// ---- refute round 6 ------------------------------------------------------------------------

#[test]
fn another_header_key_is_named_whatever_value_core_s_reader_cannot_hold() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    let stamp = "format = \"veredas-test\"\nversion = 1\n";
    let not = |at: &str, name: &str| format!("{at}: the header holds `format` and `version` alone, not `{name}`");
    // A value past 128 bits, a float past range, inside an array or a table: the key named.
    assert_eq!(one(&format!("{stamp}x = 340282366920938463463374607431768211456\n")), not("3:1", "x"));
    assert_eq!(one(&format!("{stamp}x = 1e400\n")), not("3:1", "x"));
    assert_eq!(one(&format!("{stamp}x = [1e400]\n")), not("3:1", "x"));
    assert_eq!(one(&format!("{stamp}[x]\ny = 340282366920938463463374607431768211456\n")), not("3:1", "x"));
    assert_eq!(one("\"format\" = \"veredas-test\"\n'version' = 1\n  x = 1e400\n"), not("3:3", "x"));
    // The stamp decided first all the same: another format, no version, by core's own texts.
    let wrong = "format = \"veredas-functions\"\nversion = 1\n";
    let core = |h: &str| veredas_core::schema::check(h, "a test file", "veredas-test", 1).unwrap_err().to_string();
    assert_eq!(one(&format!("{wrong}x = 1e400\n")), format!("1:1: {}", core(wrong)));
    assert_eq!(one("format = \"veredas-test\"\nx = 1e400\n"), format!("1:1: {}", core("format = \"veredas-test\"\n")));
    // The stamp's own value past range is the stamp's fault, by core's text.
    let bad = "format = \"veredas-test\"\nversion = 340282366920938463463374607431768211456\nx = 1\n";
    assert_eq!(one(bad), format!("1:1: {}", core(bad)));
    // A header that is no TOML: named where TOML found it.
    let e = one(&format!("{stamp}x = \n"));
    assert!(e.starts_with("3:5: not a test file: TOML parse error at line 3, column 5"), "{e}");
}

#[test]
fn a_function_s_inputs_reads_and_calls_are_looked_up_at_once() {
    // A function's inputs, a call's, the tables it reads and the test's constants, each looked up
    // by name in a time linear in their number: 2 500 of each, then 20 000 (tables twice as many,
    // constants 5 000, then 40 000).
    let timed = |f: &dyn Fn() -> Vec<String>| {
        let mut ps = Vec::new();
        let took = on_cpu(|| ps = f());
        assert_eq!(ps.len(), 0, "{:?}", ps.first());
        took
    };
    // A function of n inputs, assigning its output and looping n times.
    let function = |n: usize| {
        let inputs: Vec<String> = (0..n).map(|i| format!("i{i}: int")).collect();
        let mut f = format!("{FHEAD}function F({}) -> (x: int) {{\n", inputs.join(", "));
        for _ in 0..n / 2 {
            f.push_str("  x = 1\n  for k in 1 .. 2 { x = k }\n");
        }
        f.push_str("}\n");
        f
    };
    let inputs = |n: usize| {
        let f = function(n);
        timed(&|| fproblems(&f))
    };
    // A step calling it with every input, from the last to the first.
    let call = |n: usize| {
        let f = function(n);
        let args: Vec<String> = (0..n).rev().map(|i| format!("i{i}: 1")).collect();
        let t = test_of(&format!("  use f.fn\n  step a call F({})\n    -> x\n  step b stop pass\n", args.join(", ")));
        timed(&|| problems_with(&t, &[("f.fn", &f)]))
    };
    // A function reading 2n tables, one by one.
    let tables = |n: usize| {
        let tables: Vec<String> = (0..2 * n).map(|i| format!("t{i}")).collect();
        let mut g = format!("{FHEAD}function G() -> (x: int) {{\n  reads {}\n", tables.join(", "));
        for i in (0..2 * n).rev() {
            g.push_str(&format!("  x = t{i}.c\n"));
        }
        g.push_str("}\n");
        timed(&|| fproblems(&g))
    };
    // n constants, each made of the one above it.
    let constants = |n: usize| {
        let mut lets = String::from("  let c0 = 1\n");
        for i in 1..n {
            lets.push_str(&format!("  let c{i} = c{} + 1\n", i - 1));
        }
        let t = test_of(&format!("{lets}  step a stop pass\n"));
        timed(&|| problems(&t))
    };
    linear(&[("n inputs, n statements", 2_500, &inputs), ("a call of n inputs", 2_500, &call), ("2n tables read", 2_500, &tables), ("n constants", 5_000, &constants)]);
}

// ---- refute round 7 ------------------------------------------------------------------------

#[test]
fn places_on_a_line_past_ascii_are_counted_in_any_order() {
    // A line of calls each naming an input the function lacks and leaving its input out: the
    // problems' places jump back and forth on one line past ASCII (a comment at its end), each
    // counted from the line's mark before it, in a time linear in the line: 20 000 calls, then
    // 160 000.
    let calls = |n: usize| {
        let f = format!("{FHEAD}function F(a: int) -> (x: int) {{\n  x = a\n}}\nfunction G() -> (y: int) {{\n  {}y = 0 # é\n}}\n", "y = F(b: 1); ".repeat(n));
        let mut ps = Vec::new();
        let took = on_cpu(|| ps = fproblems(&f));
        assert_eq!(ps.len(), 2 * n);
        // The last call: its function's name at 5 + 13 (n - 1) + 4, the input it names at 6 more.
        let at = 3 + 13 * (n - 1) + 4;
        let last: Vec<&String> = ps.iter().filter(|p| p.starts_with(&format!("8:{}:", at)) || p.starts_with(&format!("8:{}:", at + 2))).collect();
        assert_eq!(last, [&format!("8:{}: `F` has no input `b`", at + 2), &format!("8:{at}: `F` needs its input `a`")], "{:?}", &ps[ps.len() - 2..]);
        took
    };
    // Places on two lines past ASCII taken in turn, from their ends back to their starts, so each
    // returns to a line the one before left: each counted from its line's mark before it, 80 000
    // characters a line, then 640 000.
    let turns = |n: usize| {
        let text = format!("{}\n{}\n", "é".repeat(n), "€".repeat(n));
        let lines = veredas_comissionador::Lines::new(&text);
        let second = 2 * n + 1;
        let mut got = Vec::with_capacity(2 * n);
        let took = on_cpu(|| {
            for k in (0..n).rev() {
                got.push(lines.place(2 * k));
                got.push(lines.place(second + 3 * k));
            }
        });
        let at = |line: usize, column: usize| veredas_comissionador::Place { line, column };
        assert_eq!((got[0], got[1], got[2 * n - 2], got[2 * n - 1]), (at(1, n), at(2, n), at(1, 1), at(2, 1)));
        took
    };
    linear(&[("calls on one line past ASCII", 20_000, &calls), ("places on two lines in turn", 80_000, &turns)]);
    // The same places counted from the line's start, in every order, on lines of characters of one,
    // two, three and four bytes, a place inside a character taken as that character's.
    let text = "aé€𝄞b\r\n𝄞€éa\nlast é";
    let lines = veredas_comissionador::Lines::new(text);
    let mut at: Vec<usize> = (0..=text.len()).collect();
    at.reverse();
    at.rotate_left(7);
    for &i in &at {
        let mut b = i.min(text.len());
        while !text.is_char_boundary(b) {
            b -= 1;
        }
        let start = text[..b].rfind('\n').map_or(0, |n| n + 1);
        let want = veredas_comissionador::Place { line: text[..b].matches('\n').count() + 1, column: text[start..b].chars().count() + 1 };
        assert_eq!(lines.place(i), want, "byte {i}");
    }
}

#[test]
fn the_inputs_a_call_leaves_out_are_one_problem_named_by_the_first_five() {
    let one = |call: &str| fproblems(&format!("{FHEAD}function F(a: int, b: int, c: int, d: int, e: int, f: int, g: int) -> (r: int) {{\n  r = a\n}}\nfunction G() -> (y: int) {{\n  y = {call}\n}}\n"));
    assert_eq!(one("F(a: 1, b: 1, c: 1, d: 1, e: 1, f: 1)"), ["8:7: `F` needs its input `g`"]);
    assert_eq!(one("F(a: 1, b: 1, c: 1, d: 1, e: 1)"), ["8:7: `F` needs its inputs `f`, `g`"]);
    assert_eq!(one("F(a: 1, g: 1)"), ["8:7: `F` needs its inputs `b`, `c`, `d`, `e`, `f`"]);
    assert_eq!(one("F(d: 1)"), ["8:7: `F` needs its inputs `a`, `b`, `c`, `e`, `f` and 1 other"]);
    assert_eq!(one("F()"), ["8:7: `F` needs its inputs `a`, `b`, `c`, `d`, `e` and 2 others"]);
    // An input given twice or not the function's leaves the count of those left out as it is.
    assert_eq!(one("F(a: 1, a: 2, z: 1, b: 1, c: 1, d: 1, e: 1, f: 1)"), ["8:15: input `a` is given twice", "8:21: `F` has no input `z`", "8:7: `F` needs its input `g`"]);
    // A function naming one input twice is called with it once.
    let twice = fproblems(&format!("{FHEAD}function F(a: int, a: int, b: int) -> (r: int) {{\n  r = 1\n}}\nfunction G() -> (y: int) {{\n  y = F()\n}}\n"));
    assert_eq!(twice, ["4:20: `a` is named twice in `F`'s ports", "8:7: `F` needs its inputs `a`, `b`"]);
    // A step's call the same.
    let f = format!("{FHEAD}function F(a: int, b: int, c: int, d: int, e: int, f: int, g: int) -> (r: int) {{\n  r = a\n}}\n");
    assert_eq!(problems_with(&test_of("  use f.fn\n  step a call F(c: 1)\n    -> x\n  step b stop pass\n"), &[("f.fn", &f)]), ["7:15: `F` needs its inputs `a`, `b`, `d`, `e`, `f` and 1 other"]);
    // Many calls of a function of many inputs, each giving none: a problem a call, in a time linear
    // in the file, 1 000 inputs and calls, then 8 000.
    let empty = |n: usize| {
        let inputs: Vec<String> = (0..n).map(|i| format!("i{i}: int")).collect();
        let f = format!("{FHEAD}function F({}) -> (x: int) {{\n  x = 1\n}}\nfunction G() -> (y: int) {{\n{}}}\n", inputs.join(", "), "  y = F()\n".repeat(n));
        let mut ps = Vec::new();
        let took = on_cpu(|| ps = fproblems(&f));
        assert_eq!(ps.len(), n);
        assert_eq!(ps[n - 1], format!("{}:7: `F` needs its inputs `i0`, `i1`, `i2`, `i3`, `i4` and {} others", 7 + n, n - 5));
        took
    };
    linear(&[("calls giving none of many inputs", 1_000, &empty)]);
}

#[test]
fn a_stamp_s_own_fault_beside_another_key_is_core_s_words_for_the_stamp() {
    let body = "test \"t\" {\n  id T\n  step a stop pass\n}\n";
    let one = |head: &str| {
        let e = parse_test(&format!("{head}{body}"), "t").unwrap_err();
        format!("{}:{}: {}", e.line, e.column, e.message)
    };
    let core = |stamp: &str| format!("1:1: {}", veredas_core::schema::check(stamp, "a test file", "veredas-test", 1).unwrap_err());
    // Each stamp's fault in core's words for the stamp as written, whether another key stands
    // after it, before it (its line blanked, so a line core names stays where it is) or nowhere:
    // a value core cannot read inside the stamp's own table or array among them.
    let stamps = [
        "format = 1\nversion = 1\n",
        "format = \"veredas-test\"\nversion = \"1\"\n",
        "format = \"veredas-test\"\nversion = 4294967296\n",
        "format = \"veredas-test\"\nversion = -1\n",
        "format.a = 1\nversion = 1\n",
        "format = { a = 1 }\nversion = 1\n",
        "format = \"veredas-test\"\nversion = [\n  1,\n]\n",
        "format = \"veredas-functions\"\nversion = 1\n",
        "format.a = 1e400\nversion = 1\n",
        "format = [1e400]\nversion = 1\n",
        "format = { a = 1e400 }\nversion = 1\n",
        "format = \"veredas-test\"\nversion = { a = 340282366920938463463374607431768211456 }\n",
        "format = \"veredas-other\"\nversion = [1e400]\n",
        "format = \"veredas-test\"\n[version]\na = 1e400\n",
    ];
    for stamp in stamps {
        let want = core(stamp);
        assert_eq!(one(stamp), want, "{stamp:?}");
        assert_eq!(one(&format!("x = 1e400\n{stamp}")), core(&format!("\n{stamp}")), "{stamp:?}");
        assert_eq!(one(&format!("x = 2\n{stamp}")), core(&format!("\n{stamp}")), "{stamp:?}");
        // After it, unless the stamp ends in a table, which the key would join.
        if !stamp.contains("\n[") {
            assert_eq!(one(&format!("{stamp}x = 1e400\n")), want, "{stamp:?}");
            assert_eq!(one(&format!("{stamp}x = 2\n")), want, "{stamp:?}");
        }
    }
}

// ---- the architect's Q312 (e) ----------------------------------------------------------------

#[test]
fn the_roles_a_test_names_to_sign_are_read_and_checked() {
    let c = checked(&test_of("  sign tester, witness\n  step a stop pass\n"));
    assert_eq!(c.test.sign_roles(), ["tester", "witness"]);
    assert_eq!(checked(&test_of("  step a stop pass\n")).test.sign_roles(), Vec::<String>::new());
    assert_eq!(problems(&test_of("  sign boss\n  step a stop pass\n")), ["6:8: `boss` is no role (tester, witness)"]);
    assert_eq!(problems(&test_of("  sign tester, tester\n  step a stop pass\n")), ["6:16: `tester` is named twice in `sign`"]);
    assert_eq!(problems(&test_of("  sign tester\n  sign witness\n  step a stop pass\n")), ["7:3: `sign` is given twice"]);
    let e = problems(&test_of("  sign\n  step a stop pass\n"));
    assert!(e.len() == 1 && e[0].starts_with("6:7: ") && e[0].contains("a role (tester, witness)"), "{e:?}");
}

/// How long `f` keeps this thread on a processor: its CPU time as Linux's scheduler counts it, so
/// the machine's other work is left out; the wall clock where that count is not kept.
fn on_cpu(f: impl FnOnce()) -> std::time::Duration {
    let spent = Spent::start();
    f();
    spent.so_far()
}

/// The time the thread has run, from Linux's scheduler (it moves by a tick of the clock at most,
/// a millisecond here, so what it measures is made longer than that); the wall clock's where the
/// system keeps none.
struct Spent {
    cpu: Option<u64>,
    wall: std::time::Instant,
}

impl Spent {
    fn read() -> Option<u64> {
        std::fs::read_to_string("/proc/thread-self/schedstat").ok().and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok())
    }
    fn start() -> Spent {
        Spent { wall: std::time::Instant::now(), cpu: Spent::read() }
    }
    fn so_far(&self) -> std::time::Duration {
        match (self.cpu, Spent::read()) {
            (Some(a), Some(b)) => std::time::Duration::from_nanos(b.saturating_sub(a)),
            _ => self.wall.elapsed(),
        }
    }
}

/// What `times` runs of `f` cost on the thread's CPU, or `None` once that passes `limit`, looked
/// at after every run: work that would take hours is stopped at the limit, not waited for.
fn cost(times: usize, limit: std::time::Duration, f: &dyn Fn()) -> Option<std::time::Duration> {
    let spent = Spent::start();
    for _ in 0..times {
        f();
        if spent.so_far() > limit {
            return None;
        }
    }
    Some(spent.so_far())
}

/// How a check's time grows: the best of two runs at `n` and at eight times `n`. A check linear
/// in its file takes about eight times as long at the larger size (up to twelve with its maps'
/// logarithm and a loaded machine's caches); a quadratic one, up to sixty-four (the checks before
/// round 6: 36 and 44 at these sizes). Twenty is between.
fn growth(n: usize, check: &dyn Fn(usize) -> std::time::Duration) -> (std::time::Duration, std::time::Duration) {
    let best = |n: usize| (0..2).map(|_| check(n)).min().unwrap();
    (best(n), best(8 * n))
}

/// Cases each timed by `growth`, every one run before any is judged: each grows as a linear check
/// does, under twenty times at eight times the size.
fn linear(cases: &[Case]) {
    grows_under(20, cases);
}

/// Cases each timed by `growth`: each grows no faster than the square of its size, as the search of
/// every function's way back inside its own cycle does (C-201), up to sixty-four times at eight
/// times the size, where a cubic one grows up to 512. Two hundred is between.
fn quadratic(cases: &[Case]) {
    grows_under(200, cases);
}

/// A case timed by `growth`: what it is, its smaller size, and the check run at a size.
type Case<'a> = (&'a str, usize, &'a dyn Fn(usize) -> std::time::Duration);

/// Cases each timed by `growth`, every one run before any is judged: each takes under `bound` times
/// as long at eight times its size.
fn grows_under(bound: u32, cases: &[Case]) {
    let grown: Vec<(&str, usize, std::time::Duration, std::time::Duration)> = cases
        .iter()
        .map(|(what, n, check)| {
            let (small, large) = growth(*n, *check);
            (*what, *n, small, large)
        })
        .collect();
    let past: Vec<_> = grown.iter().filter(|(_, _, small, large)| *large >= bound * *small).collect();
    assert!(past.is_empty(), "grew {bound} times or more at eight times the size: {past:?} (all: {grown:?})");
}

#[test]
fn many_used_files_and_many_blocks_are_checked_in_linear_time() {
    // Used files, each read once: 5 000, then 40 000.
    let uses = |n: usize| {
        let files: Vec<(String, String)> = (0..n).map(|i| (format!("d/f{i}.fn"), format!("{FHEAD}function G{i}() -> (x: int) {{\n  x = {i}\n}}\n"))).collect();
        let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
        let lines: String = files.iter().map(|(p, _)| format!("  use {p}\n")).collect();
        let t = test_of(&format!("{lines}  step a stop pass\n"));
        let mut ps = Vec::new();
        let took = on_cpu(|| ps = problems_with(&t, &refs));
        assert_eq!(ps, Vec::<String>::new());
        took
    };
    let (used_small, used_large) = growth(5_000, &uses);
    // A file of n functions, one of them of n blocks, each calling a function wrongly: 10 000,
    // then 80 000.
    let blocks = |n: usize| {
        let mut f = String::from(FHEAD);
        for i in 0..n {
            f.push_str(&format!("function G{i}() {{\n}}\n"));
        }
        f.push_str("function H() {\n");
        for _ in 0..n {
            f.push_str("  if G1() { }\n");
        }
        f.push_str("}\n");
        let mut ps = Vec::new();
        let took = on_cpu(|| ps = fproblems(&f));
        assert_eq!(ps.len(), n);
        assert_eq!(ps[0], format!("{}:6: `G1` is a function: a step calls it with `call`, a function by `<outputs> = G1(...)`", 3 + 2 * n + 2));
        took
    };
    let (small, large) = growth(10_000, &blocks);
    assert!(
        used_large < 20 * used_small && large < 20 * small,
        "5 000 used files: {used_small:?}, 40 000: {used_large:?}; 10 000 functions and blocks: {small:?}, 80 000: {large:?}"
    );
}

// ---- the reference's text -------------------------------------------------------------------

/// A text's paragraphs, each on one line, the blocks the tests load (`test`, `function`) left out.
fn paragraphs(text: &str) -> Vec<String> {
    let mut text = text.replace("\r\n", "\n");
    for fence in ["```test\n", "```function\n"] {
        while let Some(start) = text.find(fence) {
            let end = start + fence.len() + text[start + fence.len()..].find("```").unwrap() + 3;
            text.replace_range(start..end, "");
        }
    }
    text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

#[test]
fn the_reference_is_the_text_this_test_holds() {
    let actual = paragraphs(&doc());
    let expected = paragraphs(REFERENCE);
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "paragraph {i} of docs/test-schema.md differs from the copy this test holds");
    }
    assert_eq!(actual.len(), expected.len(), "docs/test-schema.md has {} paragraphs, the copy {}", actual.len(), expected.len());
}

/// The reference, `docs/test-schema.md`, as this test holds it: its text, the two blocks it loads
/// left out. An edit to the reference is an edit here, beside the assertions that hold it.
const REFERENCE: &str = r##"
# The test file: `veredas-test`, version 1, and the function file: `veredas-functions`, version 1

A test is a check a person runs against a plant through portal (`docs/veredas-plan.md`,
"veredas-comissionador: the testing platform"): it drives the machine through portal's gated
writes, waits for what the plant does, measures it, branches on what it measured, and ends in an
acceptance report. One plain-text file holds one test, diff-able in git; Oficina's designer shows
it as text and as a flow chart, two views of the same file (`docs/design/comissionador/`), and the
flow view's positions live in the file's last block. Functions are written once, in function
files, and placed in a test as blocks, the way a function block is placed in ladder: their inputs
are the block's left ports, their outputs its right ports. `docs/comissionador-plan.md` says how
the engine reads, checks and runs these files; this file says what they hold.

The syntax is the assistant's, not the architect's: it refines the design's draft, and which
syntax the suite keeps is open (`docs/questions.md` Q310), as are the script box's language
(Q311) and how a test names plant data (Q317). Nothing in this file names a device: the worked
example runs against the bench's controller reference (`docs/controller-schema.md`), and a
plant's tests are files like these, kept outside the suite's code.

Every plain-text file of Veredas 2 states its schema (`docs/questions.md` Q10). A test file and a
function file are not TOML, because a test is a program, but each opens with the suite's stamp
in TOML: `format` and `version` on lines of their own, read by the one header reader every file
shares (`veredas_core::schema`), so a file of another format, an unknown version or a missing
stamp is refused by the same texts as a bench or a project file, before anything else is read.

## The worked example: a destination full, items come round

The machine of the controller reference announces an item every 100 ms and reports each item's
outcome 500 ms after it was answered; its destination 4 starts full, so an item sent there comes
back `FULL` and goes round again. A portal session reads the machine on a channel named `sorter`
and answers every announcement with destination 4 by a reply rule. The test starts the machine
by a command, a gated write, and waits until the machine's status says it runs; it then follows
20 outcomes for destination 4 through a function, computes the share that came round in a script
box, and branches on it: 0.95 or more passes, 0.85 to 0.95 runs the routing once more, anything
else releases the machine, notes the count and fails. Either way the machine is stopped by a
second command. It sits beside a folder `functions` holding the function file.


The function file, `functions/routing.fn`:


Read, the test has eight steps, one branch, one function and three gated writes, its three
`send`s. The engine is held to this: run live against the machine, every outcome for
destination 4 is `FULL`, so `delivered` is 0, `rerouted` 20 and `ratio` 1, and the run passes at
`summary`; its window replayed, the two sends it meets are noted not sent, the function meets the same 20
outcomes in the store, and the replay passes with the same values. The engine's grammar test,
`crates/veredas-comissionador/tests/grammar.rs`, loads these two blocks and holds each statement
of this file about reading and checking against the reader; it keeps a copy of this text, so an
edit here is an edit there.

What is built, 2026-11-22: the reading and the check (C-201), the engine run live against a portal
session (C-202) and replayed over a stored window of one (C-203); every statement below about a
run is held by their tests. What a run keeps on disk, its record and its acceptance report, is
`docs/report-schema.md` (C-204).

## The file, line by line

**Text.** UTF-8, lines ending in LF or CRLF. `#` begins a comment that runs to the end of its
line, outside a string. Blank lines and comments go anywhere.

**The header.** Every line before the first line that opens with `test` (a test file) or
`function` (a function file) is the header: TOML holding `format` and `version` and nothing else
(comments and blank lines aside). A test file's `format` is `"veredas-test"`, a function file's
`"veredas-functions"`; this reader knows `version` 1 of each. Another key in the header is
refused by its name as TOML reads it: a table's name, a dotted key's first part, a quoted key with
its escapes decoded (a control character in it shown by its escape).

**Names.** A name is ASCII letters, digits and `_`, not starting with a digit. These words are
the language's own and name nothing else: `and`, `as`, `await`, `branch`, `call`, `check`,
`else`, `error`, `fail`, `false`, `for`, `function`, `if`, `in`, `layout`, `not`, `on`,
`or`, `pass`, `read`, `reads`, `report`, `retry`, `script`, `send`, `step`, `stop`, `test`,
`timeout`, `true`, `wait`, `where`, `write`. After a `.` any name is a column's or a message's,
the language's own words included (`a.read`); a table, a channel or a message's field whose name
is one of them is written in backquotes (`` `check`.value ``, `` send sorter.announce(`read`: "N")
``). A function's inputs are named by its author, never one of them.

**Lines.** A line ends what it holds, except inside parentheses, where lines may break freely,
and a line whose first symbol is `->` continues the line before it.

**Literals.** A whole number (`20`, `9_000`), a number with a fraction or an exponent (`0.95`,
`1e3`), `true` and `false`, a duration (a number and its unit with no space between: `250ms`,
`5s`, `2min`, `1h`, read exactly, so `1.001s` is 1001 ms) and a string in double quotes. In a string, `\"`, `\\`, `\n`, `\t`, `\{` and
`\}` are the character, and `{` an expression `}` is the expression's value as text
(`"rerouted {rerouted}/{items}"`).

## The test file

```
test "<title>" {
  <settings>
  <steps>
}
layout { <positions> }
```

The title is the test's name as the library and the report show it. The settings come first,
each on its own line, then the steps; the `layout` block, when there is one, follows the test.

### Settings

Each states in parentheses that it is required, or the value it takes when left out. `id`,
`area`, `portal` and `use` take the rest of their line, trimmed, a comment and a surrounding
pair of quotes removed.

- `id` (required): the test's identifier, letters, digits, `-` and `_`. Its runs are kept under
  it (`runs/<id, lowercased>/`).
- `area` (none): the group the library shows the test in.
- `portal` (none): the name of the portal session the test is written for; a run against a
  session of another name is refused before anything is written.
- `modes` (`live, replay`): the modes the test runs in, `live`, `replay` or both; a run in
  another mode is refused.
- `timeout` (`10min`): the whole run's bound, on the run's clock (the plant's time, which a replay
  of the run's own window reproduces, never the machine's work), judged at each step and between
  its actions. A wait with no timeout of its own is bounded by what is left of it.
- `use` (none; once per file): a function file, its path relative to the test file's folder.
- `sign` (none): the roles that must sign each run's report, `tester`, `witness` or both
  (`docs/questions.md` Q312 (e)): the report awaits each until it has signed.
- `let` (none; once per name): `let <name> = <expression>`, a constant, the expression made of
  literals and the constants above it, with no call in it.

### Steps

```
step <name> <action>
            <action>
            on <outcome>, <outcome> -> <route>
```

A step is a name, unique in the test, and one or more actions, each on its own line, run in
order; `branch` and `stop` are only ever a step's last action. `on` lines follow the actions. A
step's lines run from its `step` line to the next `step` line or the test's closing brace.

Every action ends in one **outcome**: `pass`, `fail`, `timeout` or `error`. The first action
that does not pass ends the step with its outcome; a step whose actions all pass, passes.

**The actions.** Each names the block it is drawn as in the flow view.

- `write <table>.<column> := <expression>`, a **write**, gated: the column must be an OPC UA tag
  the session's project marks writable, and portal's write switch armed. The value is written
  as the tag's type takes it. Passes when the plant confirms it; ends in `error` with portal's
  words when the switch is off, the tag is not writable, the server is away, the write may or
  may not have reached the plant, or the plant answers a bad status.
- `send <channel>.<message>(<field>: <expression>, ...)`, a **write**, gated: one message of the
  channel's catalog, composed from the fields and sent to the channel's peer. Passes once sent;
  `error` with portal's words as a write's.
- `wait <condition> [timeout <duration>]`, a **wait**: passes at the first moment the condition
  holds over the plant's values, at once when it holds as the step starts; `timeout` when the
  bound passes first. A moment is a stamp with every row of it applied, so rows that share a stamp
  are judged together. `wait <duration>` waits that long and passes.
- `await <table> [where <condition>] [timeout <duration>] -> <name>`, a **wait** for an event:
  the first row of the table stamped after the step starts for which the condition holds,
  bound to the name, which the condition uses to read the row (`await outcome where o.dest == 4
  -> o`); when the clock stands at the stamp of the row the last `await` of that table took, the
  rows of that stamp after it too, so a loop of awaits takes every row however many share a
  stamp. Passes on that row; `timeout` when the bound passes first.
- `read <reference>, ... -> <name>, ...`, a **read**: each reference's value as the step starts,
  into the names in order. `error` for a reference with no value yet.
- `check <condition> [as "<label>"]`, a **check**: a criterion of the acceptance report, under
  its label or its own text. Passes when the condition holds, `fail` when it does not, `error`
  when it cannot be judged.
- `call <Function>(<input>: <expression>, ...) [-> <name>, ...]`, a **function** block: the
  function of that name from a `use`d file, every input given once, its outputs into the names
  in order (fewer names keep the first outputs). The step ends in the function's outcome.
- `script { <statements> }`, a **script** box: a calculation over the test's variables. It reads
  no plant data and does not wait, so it computes the same in a live run and in a replay.
  Passes; `fail` by a `fail` statement; `error` on a fault (a division by zero, a name never
  assigned, two values that do not compare).
- `branch <expression> { <arms> }`, a **branch**: each arm on its own line, `<pattern>, ... ->
  <route>`, or `else -> <route>` last. The first arm with a pattern that matches routes the run.
  A pattern is a comparison and a value (`>= 0.95`, `< 3`, `== "FULL"`, `!= 0`), a range of two
  values, both included (`0.85 .. 0.95`), or a value, matched by equality. No arm matching and
  no `else`, the step ends in `fail`.
- `report <item>, ...`, a **report**: each item recorded in the report, a value under its text
  (`ratio` as `ratio = 0.9`), a string as a note. An item that cannot be evaluated (a name not
  assigned on the path taken) is recorded with the reason; a report always passes.
- `stop pass ["<reason>"]` and `stop fail ["<reason>"]`, the **end**: the run ends, passed or
  failed.

**Routes.** A route, after `->`, is where the run goes next: a step's name; `retry <step> max
<n>`, the same as naming the step at most `n` times in the run, after which the step that took
the route ends in `fail` with the reason; or `stop pass` or `stop fail`.

**Where the run goes.** A step that passes goes to the next step in the file; past the last
step the run passes. A step that ends in `fail` fails the run; one that ends in `timeout` or
`error` aborts it. An `on` line routes the outcomes it names: `on timeout -> fail_log` sends a
wait that timed out to `fail_log`, `on pass -> report` skips what lies between. An outcome is
routed once per step. A branch routes its `pass` by its arms, so a branch takes no `on pass`.

A run ends **passed**, **failed** or **aborted**: aborted when the plant did not let the test
judge it (a timeout or an error no route takes), when the tester stops the run, or, in a replay,
when the stored window ends before a step's outcome.

### The layout

```
layout { <step> <column>,<row> ... }
```

The flow view's positions, on its grid, whole numbers from 0, entries apart by spaces or line
ends. The view writes this block; a run ignores it. A position for a name that is no step is
refused; a step with no position is placed by the view.

## The function file

```
function <Name>(<input>: <type>, ...) -> (<output>: <type>, ...) {
  reads   <table>, ...
  timeout <duration>
  <statements>
}
```

A function file holds one or more functions. A function's name is unique across the files a
test uses, and is no built-in call's (`abs`, `min`, ...). Its inputs are its block's left ports and its outputs its right ports; the arrow
and the outputs are left out for a function that returns nothing. The comment lines right
above `function` are its description, shown on its block.

- **Types**: `int`, `float`, `bool`, `text`, `time` and `duration`. A call's value for an input
  is checked against the input's type as the call starts; an `int` is taken where a `float` is
  asked for.
- `reads` (none): the tables the function reads. A reference to another table is refused.
- `timeout` (none, the run's bound): the bound of one call, never past its caller's (the calling
  function's bound, or the run's).
- **The body**: statements over the inputs, which are read only, and names of its own; a name of
  the test is not one of them. Every output is assigned somewhere in the body, or the check
  refuses the function; one that the path a call takes leaves unassigned ends the call in
  `error`.
- **A function only reads.** No `write`, `send`, `report`, `check` or `stop` is a statement, so
  a block that is a function can never touch the plant. A function may call another of its own
  file, never itself, through others or directly.

## Statements

The body of a script box, of a function, and of an `if` or a `for`:

- `<name> = <expression>`, and `+=`, `-=`, `*=`, `/=`: assign a name.
- `if <condition> { ... } else if <condition> { ... } else { ... }`: the first branch whose
  condition holds.
- `for <name> in <expression> .. <expression> { ... }`: whole numbers, both ends included; a
  range whose end is below its start runs nothing. A script box or a step's call runs at most
  10 000 000 statements, each turn of a loop counted as one, and then ends in `error`; the
  tester's stop ends it as it ends a wait, and the run's bound when a wait or a call inside it
  has moved the clock past it (a script box moves none).
- `fail ["<reason>"]`: ends the script or the call in `fail`.
- In a function only: `wait <condition> [timeout <duration>]` and `wait <duration>`;
  `<name> = await <table> [where <condition>] [timeout <duration>]`, the row the condition reads
  by that name (`o = await outcome where o.dest == dest`); and `<name>, ... = <Function>(<input>:
  <expression>, ...)`, a call whose inputs are named (or that has none), where a built-in's values
  are not (`x = round(y)`). A wait or an await that times out ends the call in `timeout`.

A block is `{`, statements one per line or apart by `;`, and `}`; it may sit on one line
(`{ delivered += 1 }`). In a script box a name assigned is a variable of the test, read by the
steps after it; a `let` constant is never assigned. In a function every name is the call's own.

## Expressions

**Values.** A whole number (64 bits), a number (64-bit floating point), a truth value, a text, a
moment (on the store's axis, below), a duration, and a row (an event an `await` took: its
columns by name, and `time`, `key`, `source`, `seq` and `arrival`, what portal stores with every
row).

**Names.** A name is a variable: a `let` constant, a name a script box, a `read`, an `await` or
a call assigned, an input, a loop's name. `<name>.<column>` reads a row's column when the name
is a variable; a name reads one column, so `a.b.c` is refused. **Any other `<table>.<column>`
is plant data**: the column's latest value in
portal's table of that name (a capture rule's, an OPC UA rule's, a message's, or a view with
portal's time column), stamped at or before the run's clock, a hole in a row read past.

**Operators**, loosest first: `or`; `and`; `not`; the comparisons `==`, `!=`, `<`, `<=`, `>`,
`>=`, one per comparison; `+`, `-`; `*`, `/`, `%`; a leading `-`; a call and `.`. Parentheses
group.

- Arithmetic: whole numbers give a whole number, except `/`, which gives a number; a number on
  either side gives a number; `%` takes whole numbers. A moment less a moment is a duration; a
  moment and a duration added or taken away give a moment; durations add, a duration times a
  number is a duration, and a duration divided by a duration is a number. A whole number, a
  duration or a moment past its 64 bits is an error, never wrapped or held at its end; so is a
  number `round` or `int` would make one.
- Comparisons: numbers with numbers, whatever their kind; a truth value with a truth value, or
  with a whole number, `true` being 1 and `false` 0; a text with a text; a text with a number,
  the text trimmed and read as a number (a text that is no number is unequal to every number,
  and ordering it against one is an error); a moment with a moment, a duration with a duration.
  Anything else is an error.
- Calls: `abs(x)`, `min(a, b, ...)`, `max(a, b, ...)`, `round(x)` and `round(x, places)`,
  `seconds(d)` (a duration as a number of seconds), `now()` (the run's clock), and the
  conversions `int(x)`, `float(x)`, `text(x)`.

**No value yet.** Plant data with no row stamped at or before the clock, or a name not assigned
yet, has no value. In a wait's condition, a comparison with no value does not hold yet; anywhere
else it is the action's `error`, naming what had no value.

**Shown.** A value is shown as the suite shows it: a whole number by its digits, a number by the
shortest text that reads back to it (the worked example's `margin` is `0.050000000000000044`; a
test that wants two places writes `round(margin, 2)`), a duration in the largest unit that divides
it (`90s`, `2min`, `250ms`), a moment as the plant's clock to the millisecond, a truth value as
`true` or `false`.

## Time

A run keeps a **clock**, a moment on the store's axis: portal's, the plant's local clock read as
UTC, the axis every row is indexed on. A live run starts it at the store's now; a replay at the
start of the stored window. A wait or an await that holds moves it to the stamp of the row it
held on; `wait <duration>` moves it by the duration; a timeout moves it to the bound. In a live
run a write or a send moves it to the moment the plant answered, so whatever the plant does
after a write is after the write; in a replay a write and a send are not sent and the clock does
not move. Every step reads the plant as of the clock. A live run reads a moment only once it is
250 ms past, so the rows stamped at or before it have reached the store; a replay of a live
run's own window therefore reads the rows the live run read. Plant data read right after a write
(a read, a check, a report, a wait, an await) is read earlier in such a replay than it was live,
by the time the live run took to send the write and have it answered (at least those 250 ms after
a step that read the plant): it finds the plant as it stood before the write, a wait can time out
where it held live and an await take an earlier row; and a write whose answer, live, passes the
run's bound ends the run at its next step where the replay goes on (`docs/questions.md` Q318). A replay's clock never passes its window: a step the window
ends inside is cut short and the run aborted with the window's end named, before the run's bound
or a call's.

## What a check refuses

Before a run, the engine reads the test and every function file it uses. Reading a file stops at
its first error of form, named with its file, line and column: a header that is not the stamp or
holds another key; something that cannot be read (a string, a backquote or a `{` not closed, an
unknown escape or character, a number run into letters, a duration that is not whole
milliseconds, a number past its range); a word where another is expected, a parenthesis or a
brace not closed; one of the language's words used as a name; a setting that is unknown, has no
value or comes after a step; a mode, an action, an outcome or a type that is none; an action after
a step's `branch` or `stop`, or after its `on` lines; a `read` whose values and names differ in
number; an `else` arm that is not the last, a branch with no arm; two comparisons in a row; a
name reading more than one column; blocks and expressions (parentheses, calls, `not`, `-`)
nested deeper than 64 levels, counted together, or an expression holding more than 1000 operators; a built-in call's name given to a function;
`reads` or `timeout` twice in a function; a write, a send or another step's action written as a
statement; an `await` not assigned.

The files read, it checks them whole and names every problem: `id` missing, or not letters,
digits, `-` and `_`; a setting given twice, a path `use`d twice, a constant defined twice; a `let`
made of anything but literals and the constants above it (a call among them); a used file that cannot be read; two
functions of one name; a step's name given twice; a route to no step; `retry ... max 0`; an
outcome routed twice by one step; `on pass` on a branch; a `send`'s field given twice; a call to
no function, inputs not given (one problem a call, a long list by its first five and counted), an
input given twice or not the function's, more names than outputs; a built-in call given the wrong
number of values, a call to no built-in, a function called inside
an expression; a name read that nothing assigns; a `let` constant or a
function's input assigned; a port named twice; an output nothing assigns; a function calling
itself (each function on the cycle named, with its way back, a long one by its first five); plant data in a script
box, or a table a function does not declare in `reads`; a wait, an await or a call in a script
box; a step placed twice, or a position for no step, in the `layout`.

Bound to a portal session as the run starts, the run is refused when the session's name is not
the test's `portal`, the mode is not among the test's `modes`, a table or a column the test reads
is not in the session, a column is read through a name no table it may hold a row of has (an
`await`'s table, and through every copy of the name, in any order: which await reached the read
is the routes' to say, so a name rebound to another table's rows is judged where it runs), a
`write`'s column is not an OPC UA tag marked writable, a `send`'s channel or message is not the
session's, or, live, the test writes, portal's switch is off and the run may not arm it (the person
has not confirmed the test's gated writes by their fingerprint, or nothing handed the run the way
to arm it): a run so confirmed arms the switch before its first step and disarms it at its end,
whichever way it ends, and a switch it finds armed it leaves as it found it (`docs/questions.md`
Q314, Q320).

## Grammar

```
test-file     = header "test" string "{" { setting } { step } "}" [ layout ]
function-file = header function { function }
header        = TOML lines: format, version
setting       = ( "id" | "area" | "portal" | "use" ) rest-of-line
              | "modes" mode { "," mode } | "timeout" duration | "sign" role { "," role }
              | "let" name "=" expr
role          = "tester" | "witness"
step          = "step" name action { action } { on }
action        = "write" ref ":=" expr
              | "send" name "." name "(" [ args ] ")"
              | "wait" ( expr [ "timeout" duration ] | duration )
              | "await" table [ "where" expr ] [ "timeout" duration ] "->" name
              | "read" ref { "," ref } "->" name { "," name }
              | "check" expr [ "as" string ]
              | "call" name "(" [ args ] ")" [ "->" name { "," name } ]
              | "script" block
              | "branch" expr "{" { arm } [ "else" "->" route ] "}"
              | "report" ( expr | string ) { "," ( expr | string ) }
              | "stop" ( "pass" | "fail" ) [ string ]
on            = "on" outcome { "," outcome } "->" route
arm           = pattern { "," pattern } "->" route
pattern       = ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) expr | expr ".." expr | expr
route         = name | "retry" name "max" integer | "stop" ( "pass" | "fail" )
outcome       = "pass" | "fail" | "timeout" | "error"
layout        = "layout" "{" { name integer "," integer } "}"
function      = "function" name "(" [ param { "," param } ] ")"
                [ "->" "(" param { "," param } ")" ] "{" [ "reads" table { "," table } ]
                [ "timeout" duration ] { statement } "}"
param         = name ":" type
type          = "int" | "float" | "bool" | "text" | "time" | "duration"
block         = "{" { statement } "}"
statement     = name ( "=" | "+=" | "-=" | "*=" | "/=" ) expr
              | "if" expr block { "else" "if" expr block } [ "else" block ]
              | "for" name "in" expr ".." expr block
              | "fail" [ string ]
              | "wait" ( expr [ "timeout" duration ] | duration )
              | name "=" "await" table [ "where" expr ] [ "timeout" duration ]
              | name { "," name } "=" name "(" [ args ] ")"
args          = name ":" expr { "," name ":" expr }
ref           = table "." name
table         = name | "`" any-name "`"
expr          = or-expr, by the operators above; a primary is a literal, a name, a name "." name,
                a call name "(" [ expr { "," expr } ] ")", or "(" expr ")"
```
"##;
