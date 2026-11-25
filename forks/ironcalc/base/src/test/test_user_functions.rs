#![allow(clippy::unwrap_used)]

//! `=MINHA(A1)`: names the engine does not know, answered by the host's
//! `UserFunctions` hook (fork change 7).

use std::cell::RefCell;
use std::rc::Rc;

use crate::expressions::token::Error;
use crate::test::util::new_empty_model;
use crate::user_functions::{UserFunctions, UserValue};

type Calls = Rc<RefCell<Vec<(String, Vec<UserValue>)>>>;

/// DOUBLE(x) = 2x; ECHO(..) returns its arguments as a one-row array;
/// FAIL() errors; calls are recorded.
struct Host {
    calls: Calls,
}

impl UserFunctions for Host {
    fn has(&self, name: &str) -> bool {
        matches!(
            name.to_ascii_uppercase().as_str(),
            "DOUBLE" | "ECHO" | "FAIL" | "COUNTCELLS"
        )
    }
    fn call(&mut self, name: &str, args: Vec<UserValue>) -> Result<UserValue, String> {
        self.calls.borrow_mut().push((name.to_string(), args.clone()));
        match name.to_ascii_uppercase().as_str() {
            "DOUBLE" => match args.first() {
                Some(UserValue::Number(n)) => Ok(UserValue::Number(n * 2.0)),
                Some(UserValue::Empty) => Ok(UserValue::Number(0.0)),
                other => Err(format!("DOUBLE wants a number, got {other:?}")),
            },
            "ECHO" => Ok(UserValue::Text(format!("{args:?}"))),
            "COUNTCELLS" => match args.first() {
                Some(UserValue::Array(rows)) => Ok(UserValue::Number(
                    rows.iter().map(|r| r.len()).sum::<usize>() as f64,
                )),
                _ => Ok(UserValue::Error(Error::NA)),
            },
            _ => Err("FAIL failed".to_string()),
        }
    }
}

fn host() -> (Box<Host>, Calls) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    (
        Box::new(Host {
            calls: calls.clone(),
        }),
        calls,
    )
}

#[test]
fn without_a_hook_an_unknown_name_is_name_error() {
    let mut model = new_empty_model();
    model._set("A1", "21");
    model._set("B1", "=DOUBLE(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#NAME?");
}

#[test]
fn the_hook_answers_its_names_and_only_its_names() {
    let mut model = new_empty_model();
    let (h, calls) = host();
    model.set_user_functions(Some(h));
    model._set("A1", "21");
    model._set("B1", "=DOUBLE(A1)");
    model._set("B2", "=double(A1)+1");
    model._set("B3", "=NOTMINE(A1)");
    model._set("B4", "=DOUBLE(\"x\")");
    model._set("B5", "=FAIL()");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");
    assert_eq!(model._get_text("B2"), "43");
    assert_eq!(model._get_text("B3"), "#NAME?");
    assert_eq!(model._get_text("B4"), "#VALUE!");
    assert_eq!(model._get_text("B5"), "#VALUE!");
    // the formula keeps the name as written
    assert_eq!(model._get_formula("B2"), "=double(A1)+1");
    assert!(calls
        .borrow()
        .iter()
        // (the name arrives in lower case, as the formula spells it; Veredas fork change 33)
        .any(|(n, a)| n == "double" && a == &vec![UserValue::Number(21.0)]));
}

#[test]
fn arguments_arrive_typed_and_ranges_as_arrays() {
    let mut model = new_empty_model();
    let (h, calls) = host();
    model.set_user_functions(Some(h));
    model._set("A1", "1");
    model._set("A2", "two");
    model._set("B1", "TRUE");
    model._set("C1", "=ECHO(A1:B2, \"t\", , 1/0)");
    model._set("C2", "=COUNTCELLS(A1:B2)");
    model._set("C3", "=COUNTCELLS(A:A)");
    model.evaluate();
    let calls = calls.borrow();
    let echo = calls.iter().rev().find(|(n, _)| n == "echo").unwrap();
    assert_eq!(
        echo.1,
        vec![
            UserValue::Array(vec![
                vec![UserValue::Number(1.0), UserValue::Boolean(true)],
                vec![UserValue::Text("two".to_string()), UserValue::Empty],
            ]),
            UserValue::Text("t".to_string()),
            UserValue::Empty,
            UserValue::Error(Error::DIV),
        ]
    );
    assert_eq!(model._get_text("C2"), "4");
    // a whole column is clipped to the cells that exist (rows 1..3 here)
    assert_eq!(model._get_text("C3"), "3");
}

#[test]
fn a_function_registered_after_the_formula_resolves_on_the_next_evaluate() {
    let mut model = new_empty_model();
    model._set("A1", "5");
    model._set("B1", "=DOUBLE(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#NAME?");
    let (h, _) = host();
    model.set_user_functions(Some(h));
    model.evaluate();
    assert_eq!(model._get_text("B1"), "10");
    model.set_user_functions(None);
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#NAME?");
}

#[test]
fn a_defined_name_lambda_wins_over_the_hook() {
    let mut model = new_empty_model();
    let (h, calls) = host();
    model.set_user_functions(Some(h));
    model
        .new_defined_name(crate::DefinedNameEntry::new(
            "DOUBLE",
            "=LAMBDA(x, x*3)",
        ))
        .unwrap();
    model._set("A1", "5");
    model._set("B1", "=DOUBLE(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "15");
    assert!(calls.borrow().iter().all(|(n, _)| n != "DOUBLE"));
}
