use crate::expressions::parser::ArrayNode;
use crate::expressions::types::CellReferenceIndex;
use crate::functions::math_and_trigonometry::array_size::check_array_size;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

// Veredas fork change 16: a host's seed, when it set one, comes first (`crate::seed_random`), each
// cell drawing its own sequence (change 40).
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn random(cell: &CellReferenceIndex) -> f64 {
    if let Some(v) = crate::model::next_seeded_random(cell) {
        return v;
    }
    rand::random()
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn random(cell: &CellReferenceIndex) -> f64 {
    use js_sys::Math;
    if let Some(v) = crate::model::next_seeded_random(cell) {
        return v;
    }
    Math::random()
}

impl<'a> Model<'a> {
    pub(crate) fn fn_rand(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !args.is_empty() {
            return CalcResult::new_args_number_error(cell);
        }
        CalcResult::Number(random(&cell))
    }

    // TODO: Add tests for RANDBETWEEN
    pub(crate) fn fn_randbetween(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let x = match self.get_number(&args[0], cell) {
            Ok(f) => f.floor(),
            Err(s) => return s,
        };
        let y = match self.get_number(&args[1], cell) {
            Ok(f) => f.ceil() + 1.0,
            Err(s) => return s,
        };
        if x > y {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: format!("{x}>{y}"),
            };
        }
        CalcResult::Number((x + random(&cell) * (y - x)).floor())
    }

    /// `=RANDARRAY([rows], [cols], [min], [max], [whole_number])`
    ///
    /// Returns a 2-D array of random numbers.
    ///   * rows         – number of rows (default 1)
    ///   * cols         – number of columns (default 1)
    ///   * min          – minimum value (default 0)
    ///   * max          – maximum value (default 1)
    ///   * whole_number – FALSE = decimal, TRUE = integer (default FALSE)
    pub(crate) fn fn_randarray(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() > 5 {
            return CalcResult::new_args_number_error(cell);
        }

        let rows = if !args.is_empty() {
            let v = self.evaluate_node_in_context(&args[0], cell);
            if matches!(v, CalcResult::EmptyArg) {
                1
            } else {
                match self.cast_to_number(v, cell) {
                    Ok(n) => {
                        let n = n.floor() as i64;
                        if n == 0 {
                            return CalcResult::new_error(
                                Error::CALC,
                                cell,
                                "rows must be >= 1".to_string(),
                            );
                        }
                        if n < 0 {
                            return CalcResult::new_error(
                                Error::VALUE,
                                cell,
                                "rows must be >= 1".to_string(),
                            );
                        }
                        n as usize
                    }
                    Err(e) => return e,
                }
            }
        } else {
            1
        };

        let columns = if args.len() >= 2 {
            let v = self.evaluate_node_in_context(&args[1], cell);
            if matches!(v, CalcResult::EmptyArg) {
                1
            } else {
                match self.cast_to_number(v, cell) {
                    Ok(n) => {
                        let n = n.floor() as i64;
                        if n == 0 {
                            return CalcResult::new_error(
                                Error::CALC,
                                cell,
                                "columns must be >= 1".to_string(),
                            );
                        }
                        if n < 0 {
                            return CalcResult::new_error(
                                Error::VALUE,
                                cell,
                                "columns must be >= 1".to_string(),
                            );
                        }
                        n as usize
                    }
                    Err(e) => return e,
                }
            }
        } else {
            1
        };

        if let Some((error, message)) = check_array_size(rows, columns) {
            return CalcResult::new_error(error, cell, message);
        }

        let min = if args.len() >= 3 {
            let v = self.evaluate_node_in_context(&args[2], cell);
            if matches!(v, CalcResult::EmptyArg) {
                0.0
            } else {
                match self.cast_to_number(v, cell) {
                    Ok(n) => n,
                    Err(e) => return e,
                }
            }
        } else {
            0.0
        };

        let max = if args.len() >= 4 {
            let v = self.evaluate_node_in_context(&args[3], cell);
            if matches!(v, CalcResult::EmptyArg) {
                1.0
            } else {
                match self.cast_to_number(v, cell) {
                    Ok(n) => n,
                    Err(e) => return e,
                }
            }
        } else {
            1.0
        };

        if min > max {
            return CalcResult::new_error(Error::VALUE, cell, "min must be <= max".to_string());
        }

        // get_boolean already maps EmptyArg → false, which is the correct default.
        let whole_number = if args.len() >= 5 {
            match self.get_boolean(&args[4], cell) {
                Ok(b) => b,
                Err(e) => return e,
            }
        } else {
            false
        };

        let mut result = Vec::with_capacity(rows);
        for _ in 0..rows {
            let mut row = Vec::with_capacity(columns);
            for _ in 0..columns {
                let val = if whole_number {
                    let min_int = min.floor();
                    let max_int = max.floor();
                    let span = max_int - min_int + 1.0;
                    min_int + (random(&cell) * span).floor()
                } else {
                    min + random(&cell) * (max - min)
                };
                row.push(ArrayNode::Number(val));
            }
            result.push(row);
        }

        CalcResult::Array(result)
    }
}

#[cfg(test)]
mod seeded_tests {
    use crate::test::util::new_empty_model;

    fn draws() -> Vec<String> {
        let mut model = new_empty_model();
        model._set("A1", "=RAND()");
        model._set("A2", "=RANDBETWEEN(1,1000000)");
        model._set("A3", "=RANDARRAY(2,2,0,100,TRUE)");
        model.evaluate();
        ["A1", "A2", "A3", "B3", "A4", "B4"].iter().map(|c| model._get_text(c)).collect()
    }

    #[test]
    fn a_seed_makes_the_draws_the_same_every_time_and_none_returns_to_chance() {
        crate::seed_random(Some(42));
        let first = draws();
        crate::seed_random(Some(42));
        assert_eq!(draws(), first);
        assert!(crate::random_is_seeded());
        // another seed, other draws
        crate::seed_random(Some(43));
        assert_ne!(draws(), first);
        // unseeded: back to chance (a RAND() equal to the seeded one is out of reach)
        crate::seed_random(None);
        assert!(!crate::random_is_seeded());
        assert_ne!(draws()[0], first[0]);
        // every draw lies in [0, 1)
        crate::seed_random(Some(7));
        let cell = crate::expressions::types::CellReferenceIndex { sheet: 0, row: 1, column: 1 };
        for _ in 0..10_000 {
            let x = crate::model::next_seeded_random(&cell).unwrap();
            assert!((0.0..1.0).contains(&x), "{x}");
        }
        crate::seed_random(None);
    }

    /// The cells' values in the order given, after an evaluation seeded with 5.
    fn values(model: &mut crate::Model, cells: &[&str]) -> Vec<String> {
        crate::seed_random(Some(5));
        model.evaluate();
        cells.iter().map(|c| model._get_text(c)).collect()
    }

    #[test]
    fn each_cell_draws_its_own_sequence_and_a_host_keys_a_moved_one() {
        // Veredas fork change 40: a draw removed before a cell, in the evaluation's order, leaves
        // the cell's own draws as they were
        let mut model = new_empty_model();
        for r in 1..=5 {
            model._set(&format!("A{r}"), "=RAND()");
        }
        model._set("B1", "=RANDARRAY(1,3)");
        let first = values(&mut model, &["A2", "A3", "A4", "A5", "B1", "C1", "D1"]);
        assert_eq!(values(&mut model, &["A2", "A3", "A4", "A5", "B1", "C1", "D1"]), first, "seeded alike, drawn alike");
        model._set("A1", "7");
        assert_eq!(values(&mut model, &["A2", "A3", "A4", "A5", "B1", "C1", "D1"]), first, "A1's draw removed");
        // two cells' draws differ, and a cell's second draw is not its first
        model._set("E1", "=RAND()-RAND()");
        let e1 = values(&mut model, &["E1"]);
        assert_ne!(e1[0], "0");
        assert_ne!(first[0], first[1]);
        // a host keys the cells it moved: A3 drawing as A2 stood draws A2's numbers, a place the
        // keys do not hold draws as no other cell's
        let mut keys = std::collections::HashMap::new();
        keys.insert((0, 3, 1), (0, 2, 1));
        crate::key_random_draws(Some(keys));
        let moved = values(&mut model, &["A3", "A2"]);
        crate::key_random_draws(None);
        assert_eq!(moved[0], first[0], "A3 draws as A2");
        assert_ne!(moved[1], first[0], "A2, not in the keys, draws as no other cell");
        assert_ne!(moved[1], first[1]);
        crate::seed_random(None);
    }

    #[test]
    fn a_run_the_evaluator_abandons_and_runs_again_draws_the_same_numbers() {
        // Veredas fork change 40, refined: too deep a recursion (64 formulas, or a stack whose
        // depth depends on the build) abandons the outer cells' runs and replays them; each run
        // draws from its first number, so the limits never change a draw
        let chain = |model: &mut crate::Model| {
            model._set("A1", "=RAND()+B1*0");
            model._set("C1", "=RAND()");
            for r in 1..100 {
                model._set(&format!("B{r}"), &format!("=B{}", r + 1));
            }
            model._set("B100", "1");
        };
        let mut deep = new_empty_model();
        chain(&mut deep);
        let default = values(&mut deep, &["A1", "C1"]);
        deep.set_max_evaluation_depth(1);
        assert_eq!(values(&mut deep, &["A1", "C1"]), default, "replayed at every step");
        deep.set_max_evaluation_depth(4);
        assert_eq!(values(&mut deep, &["A1", "C1"]), default, "replayed every fourth step");
        // and a cell reading nothing draws its first number, as A1 does at the end of the chain
        let mut shallow = new_empty_model();
        shallow._set("A1", "=RAND()");
        assert_eq!(values(&mut shallow, &["A1"])[0], default[0]);
        crate::seed_random(None);
    }

    #[test]
    fn a_read_of_an_abandoned_run_puts_nothing_on_record() {
        // Veredas fork change 42: a run the recursion abandons reads every cell empty, so it may
        // take a branch no real run takes and read a whole row; put on record for the anchor, its
        // own spill into that row was read as contradicting it, `#CIRC!`, as deep as the limits
        // (which turn on the build's stack) let the unwind come
        for reads in ["SUM(1:1)", "SUM(INDIRECT(\"1:1\"))", "SUMIFS(1:1,1:1,\">0\")"] {
            let anchor = format!("=RANDARRAY(1,3)+IF(Z1=1,0,{reads})*0");
            let mut plain = new_empty_model();
            plain._set("Z1", "1");
            plain._set("AA1", &anchor);
            let want = values(&mut plain, &["AA1", "AB1", "AC1"]);
            assert!(want.iter().all(|v| v.parse::<f64>().is_ok()), "{reads}: {want:?}");
            let mut deep = new_empty_model();
            for r in 1..40 {
                deep._set(&format!("Z{r}"), &format!("=Z{}", r + 1));
            }
            deep._set("Z40", "1");
            deep._set("AA1", &anchor);
            for depth in [64, 16, 4, 1] {
                deep.set_max_evaluation_depth(depth);
                assert_eq!(values(&mut deep, &["AA1", "AB1", "AC1"]), want, "{reads} at depth {depth}");
            }
        }
        crate::seed_random(None);
    }
}
