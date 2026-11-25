//! The engine, live (contract: C-202): a checked test bound to a portal session fed by the bench
//! (its OPC UA server and the controller reference's machine over a real socket) and run step by
//! step, every action, route, outcome and verdict as `docs/test-schema.md` says.

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::*;
use veredas_comissionador::eval::{Value, duration_text};
use veredas_comissionador::model::Outcome;
use veredas_comissionador::plant::{LivePlant, Plant};
use veredas_comissionador::run::{RunOptions, RunResult, RunVerdict, SETTLE_MS};

fn texts(r: &RunResult) -> Vec<String> {
    r.reported.iter().map(|x| x.text.clone()).collect()
}

fn writes(r: &RunResult) -> Vec<(String, bool, String, bool)> {
    r.writes.iter().map(|w| (w.text.clone(), w.sent, w.answer.clone(), w.ok)).collect()
}

/// Each step starts where the one before it ended: the clock never runs back.
fn monotone(r: &RunResult) {
    assert!(r.steps.windows(2).all(|w| w[1].start_ms == w[0].end_ms), "{:?}", r.steps);
    assert!(r.steps.iter().all(|s| s.end_ms >= s.start_ms));
    assert_eq!(r.steps.first().map(|s| s.start_ms), Some(r.start_ms));
    assert_eq!(r.steps.last().map(|s| s.end_ms), Some(r.end_ms));
}

#[test]
fn the_worked_example_passes_live_against_the_machine() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&blocks("test-schema.md", "test").remove(0));
    let (r, events) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    let st = steps(&r);
    assert_eq!(st.len(), 7, "{st:?}");
    assert_eq!(st[0], "start #1 pass: sorter.command(code: 1): sent");
    assert!(st[1].starts_with("running #1 pass: status.running == 1 held after "), "{}", st[1]);
    assert_eq!(st[2..], [
        "route #1 pass: delivered 0 · rerouted 20",
        "ratio #1 pass: margin = 0.050000000000000044, ratio = 1",
        "verdict #1 pass: 1 matched >= 0.95 -> release",
        "release #1 pass: sorter.command(code: 0): sent",
        "summary #1 pass: 3 reported",
    ]);
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Passed, Some("summary"), "`summary` passed the run"));
    assert_eq!(texts(&r), ["ratio = 1", "margin = 0.050000000000000044", "rerouted = 20"]);
    assert_eq!(r.criteria.len(), 1);
    assert_eq!(r.criteria[0].text, "ratio: >= 0.95 -> release; 0.85 .. 0.95 -> retry route max 1; else -> fail_log");
    assert_eq!((r.criteria[0].measured.clone(), r.criteria[0].result.as_str()), (vec!["ratio = 1".to_string()], ">= 0.95 -> release"));
    assert_eq!(writes(&r), [
        ("sorter.command(code: 1)".to_string(), true, "sent".to_string(), true),
        ("sorter.command(code: 0)".to_string(), true, "sent".to_string(), true),
    ]);
    monotone(&r);
    // The plant saw the stop the test sent last.
    assert!(wait(Duration::from_secs(5), || bench.channel("machine").unwrap().status().controller.unwrap().registers["running"] == 0));
    // Every step's start and end went to the listener, then the verdict.
    let started = events.iter().filter(|e| matches!(e, veredas_comissionador::run::Event::StepStarted { .. })).count();
    let ended = events.iter().filter(|e| matches!(e, veredas_comissionador::run::Event::StepEnded(_))).count();
    assert_eq!((started, ended), (7, 7));
    assert!(matches!(events.last(), Some(veredas_comissionador::run::Event::Ended { verdict: RunVerdict::Passed, .. })));
    // Every criterion, value reported and write went to the listener too, each inside its step.
    use veredas_comissionador::run::Event;
    let kinds: String = events
        .iter()
        .map(|e| match e {
            Event::StepStarted { .. } => '(',
            Event::StepEnded(_) => ')',
            Event::Criterion(_) => 'c',
            Event::Reported(_) => 'r',
            Event::Write(_) => 'w',
            Event::Ended { .. } => '.',
        })
        .collect();
    assert_eq!(kinds, "(w)()()()(c)(w)(rrr).");
    let crit: Vec<_> = events.iter().filter_map(|e| if let Event::Criterion(x) = e { Some(x.clone()) } else { None }).collect();
    let rep: Vec<_> = events.iter().filter_map(|e| if let Event::Reported(x) = e { Some(x.clone()) } else { None }).collect();
    let wr: Vec<_> = events.iter().filter_map(|e| if let Event::Write(x) = e { Some(x.clone()) } else { None }).collect();
    assert_eq!((crit, rep, wr), (r.criteria.clone(), r.reported.clone(), r.writes.clone()));
}

#[test]
fn the_worked_example_fails_where_the_destination_is_free() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 1));
    // The example's function follows the destination the replies name.
    let c = checked(&blocks("test-schema.md", "test").remove(0).replace("ItemRoute(dest: 4", "ItemRoute(dest: 1"));
    let (r, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    let names: Vec<&str> = r.steps.iter().map(|s| s.step.as_str()).collect();
    assert_eq!(names, ["start", "running", "route", "ratio", "verdict", "fail_log"]);
    assert!(r.steps[2].detail.ends_with(" · rerouted 0"), "{}", r.steps[2].detail);
    assert_eq!(r.criteria[0].result, "else -> fail_log");
    assert_eq!(texts(&r), ["rerouted 0/20"]);
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Failed, Some("fail_log"), "`fail_log` failed the run"));
    assert_eq!(writes(&r).len(), 2);
    monotone(&r);
}

#[test]
fn a_gated_write_lands_and_a_wait_meets_what_it_caused() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of(concat!(
        "  step now   wait plc_main.level == 0.9 timeout 1s\n",
        "  step go    write plc_main.run := 1\n",
        "  step fast  wait plc_main.speed >= 2.0 timeout 5s\n",
        "  step look  read plc_main.speed, plc_main.level -> s, l\n",
        "  step ok    check s == 2.0 and l > 0.5 as \"speed reached\"\n",
        "  step low   check s < 1\n",
    )));
    let (r, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    let st = steps(&r);
    assert_eq!(st[0], "now #1 pass: plc_main.level == 0.9 held after 0s", "a condition that holds as the step starts");
    assert_eq!(st[1], "go #1 pass: plc_main.run := 1: Good");
    // Every moment here is a stamp on the store's axis, the clock the rows carry and the plant's
    // `now` reads. The write's step ends at the plant's answer: the step before it read the plant,
    // so the write went out once that read's moment was 250 ms past.
    let (now, go, fast) = (&r.steps[0], &r.steps[1], &r.steps[2]);
    assert_eq!((now.end_ms, go.start_ms), (now.start_ms, now.start_ms), "a condition holding at once moves no clock");
    assert!(go.end_ms - go.start_ms >= SETTLE_MS, "the clock moved to the answer: {} ms", go.end_ms - go.start_ms);
    // The wait starts there and holds at the stamp of the first row of the store the reaction
    // set, the speed 1.5 before it: the bench's 100 ms is counted from the moment it took the
    // write, which the engine never sees, the answer coming back after it.
    let plant = LivePlant::new(s.live()).unwrap();
    let table = plant.tables().unwrap().into_iter().find(|t| t.name == "plc_main").unwrap();
    let rows = plant.rows(&table, 0, plant.now_ms()).unwrap();
    let reacted = rows.iter().position(|row| row.field("speed") == Some(Some(Value::Float(2.0)))).expect("the reaction stored");
    assert!(reacted > 0 && rows[..reacted].iter().all(|row| row.field("speed") == Some(Some(Value::Float(1.5)))), "{rows:?}");
    let reacted = rows[reacted].time_ms;
    assert!(reacted > go.start_ms, "the speed changed after the write went out");
    assert_eq!((fast.start_ms, fast.end_ms), (go.end_ms, reacted.max(go.end_ms)), "the wait holds at the reaction's row, or at once had it been stored before the answer");
    assert_eq!(st[2], format!("fast #1 pass: plc_main.speed >= 2.0 held after {}", duration_text(fast.end_ms - fast.start_ms)));
    assert_eq!(st[3..], ["look #1 pass: s = 2, l = 0.9", "ok #1 pass: speed reached: pass", "low #1 fail: s < 1: fail (s = 2)"]);
    assert_eq!(tag(&bench, "run"), "Int32(1)", "the write reached the plant");
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Failed, Some("low"), "`low` failed: s < 1: fail (s = 2)"));
    let crit: Vec<(String, Vec<String>, String)> = r.criteria.iter().map(|c| (c.text.clone(), c.measured.clone(), c.result.clone())).collect();
    assert_eq!(crit, [
        ("speed reached".to_string(), vec!["s = 2".to_string(), "l = 0.9".to_string()], "pass".to_string()),
        ("s < 1".to_string(), vec!["s = 2".to_string()], "fail".to_string()),
    ]);
    monotone(&r);
}

#[test]
fn the_binding_refuses_before_anything_runs() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    let c = checked(&format!(
        "{HEAD}test \"t\" {{\n  id T\n  portal another hall\n  modes replay\n  step a write plc_main.run := 1\n  step b write plc_main.speed := 1\n  step c send sorter.nothing()\n  step d send elsewhere.command(code: 1)\n  step e check ghost.x == 1 and plc_main.nothing == 1\n  step f await nowhere -> x\n}}\n"
    ));
    let e = run_with(&c, s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, [
        "the test is written for the portal session `another hall`, and this one is `the press hall`",
        "the test runs replay, not live",
        "`plc_main.speed` is not an OPC UA tag the session marks writable",
        "`sorter.nothing` is no channel and message of the session",
        "`elsewhere.command` is no channel and message of the session",
        "`nowhere` is no table of the session",
        "`ghost` is no table of the session",
        "`plc_main` has no column `nothing`",
        "the test writes, and portal's write switch is off",
    ]);
    // A test that writes is refused while the switch is off, and nothing reached the plant.
    let c = checked(&test_of("  step a write plc_main.run := 1\n"));
    let e = run_with(&c, s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, ["the test writes, and portal's write switch is off"]);
    assert_eq!(e.to_string(), "the run is refused: the test writes, and portal's write switch is off");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(tag(&bench, "run"), "Int32(0)");
    // A test that only reads runs with the switch off.
    let c = checked(&test_of("  step a check plc_main.level == 0.9\n"));
    assert_eq!(run_with(&c, s.live(), &RunOptions::live()).unwrap().0.verdict, RunVerdict::Passed);
    // A column read through a row an `await` binds, in a step, a script box and a function, against
    // its table; the fields every row has, and a column it has, bound.
    let c = checked(&test_of(concat!(
        "  step e await announce where a.nosuch == 2 timeout 2s -> a\n",
        "  step r report a.item, a.time, a.seq, a.other\n",
        "  step s script { b = a\n y = b.third }\n",
    )));
    assert_eq!(run_with(&c, s.live(), &RunOptions::live()).unwrap_err().0, [
        "`announce` has no column `nosuch`, read as `a.nosuch`",
        "`announce` has no column `other`, read as `a.other`",
        "`announce` has no column `third`, read as `b.third`",
    ]);
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction Rows() -> (o: int) {\n  reads outcome\n  r = await outcome where r.nosuch == 1\n  o = r.dest\n}\n";
    let c = checked_with(&test_of("  use f.fn\n  step a call Rows() -> o\n"), f);
    assert_eq!(run_with(&c, s.live(), &RunOptions::live()).unwrap_err().0, ["`outcome` has no column `nosuch`, read as `r.nosuch`"]);
}

#[test]
fn a_test_refused_for_a_row_s_column_writes_nothing() {
    // Armed: the write before the await would land, were the run not refused whole.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of("  step go write plc_main.run := 1\n  step e await announce where a.nosuch == 2 timeout 2s -> a\n"));
    assert_eq!(run_with(&c, s.live(), &RunOptions::live()).unwrap_err().0, ["`announce` has no column `nosuch`, read as `a.nosuch`"]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(tag(&bench, "run"), "Int32(0)");
}

#[test]
fn routes_retries_and_ends_take_the_run_where_the_file_says() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    let go = |body: &str| run_with(&checked(&test_of(body)), s.live(), &RunOptions::live()).unwrap().0;
    // A retry taken twice, then the else.
    let r = go("  step init script { n = 0 }\n  step bump script { n += 1 }\n  step v branch n {\n    1 .. 2 -> retry bump max 2\n    else -> done\n  }\n  step done report n, \"n is {n}\"\n");
    assert_eq!(steps(&r), [
        "init #1 pass: n = 0",
        "bump #1 pass: n = 1",
        "v #1 pass: 1 matched 1 .. 2 -> retry bump max 2",
        "bump #2 pass: n = 2",
        "v #2 pass: 2 matched 1 .. 2 -> retry bump max 2",
        "bump #3 pass: n = 3",
        "v #3 pass: 3 matched else -> done",
        "done #1 pass: 2 reported",
    ]);
    assert_eq!(texts(&r), ["n = 3", "n is 3"]);
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Passed, "the run passed its last step"));
    assert_eq!(r.criteria.iter().map(|c| c.result.as_str()).collect::<Vec<_>>(), ["1 .. 2 -> retry bump max 2", "1 .. 2 -> retry bump max 2", "else -> done"]);
    // A retry spent: the step fails with the reason, routed by its `on fail` or failing the run.
    let body = "  step init script { n = 0 }\n  step bump script { n += 1 }\n  step v branch n {\n    1 .. 5 -> retry bump max 1\n  }\n";
    let r = go(body);
    assert_eq!(steps(&r)[4], "v #2 pass: 2 matched 1 .. 5 -> retry bump max 1");
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Failed, Some("v"), "`v`: the retry of `bump` is spent after 1"));
    let r = go(&format!("{body}         on fail -> rescue\n  step skipped stop fail\n  step rescue stop pass \"rescued after {{n}}\"\n"));
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Passed, Some("rescue"), "rescued after 2"));
    // No arm matched: a failure; `on fail` routes it, `stop` ends with its verdict.
    let r = go("  step v branch 7 {\n    1 -> v\n  }\n");
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Failed, "`v` failed: 7 matched no arm"));
    let r = go("  step v branch 7 {\n    1 -> v\n  }\n         on fail -> stop pass\n");
    assert_eq!(r.verdict, RunVerdict::Passed);
    // `on pass` skips what lies between; past the last step the run passes.
    let r = go("  step a script { x = 1 }\n         on pass -> c\n  step b stop fail\n  step c report x\n");
    assert_eq!(r.steps.iter().map(|s| s.step.as_str()).collect::<Vec<_>>(), ["a", "c"]);
    assert_eq!(r.verdict, RunVerdict::Passed);
    // A script's `fail` fails the run; a fault aborts it, unless routed.
    let r = go("  step s script {\n    if 2 > 1 { fail \"too slow: {2 * 3}\" }\n  }\n");
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Failed, "`s` failed: too slow: 6"));
    let r = go("  step s script { x = 1 / 0 }\n");
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "`s`: a division by zero"));
    assert_eq!(r.steps[0].outcome, Some(Outcome::Error));
    let r = go("  step s script { x = 1 / 0 }\n         on error -> e\n  step e report \"handled\"\n");
    assert_eq!((r.verdict, texts(&r)), (RunVerdict::Passed, vec!["handled".to_string()]));
    // Several actions: the first that does not pass ends the step; a report always passes.
    let r = go("  step s report later\n         check 1 == 2\n         report \"never\"\n  step t script { later = 1 }\n");
    assert_eq!(texts(&r), ["later: `later` has no value yet"]);
    assert_eq!(steps(&r), ["s #1 fail: 1 reported · 1 == 2: fail ()"]);
    // A run with no end in sight is aborted, not hung.
    let r = go("  step a script { x = 1 }\n         on pass -> a\n");
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "more than 100000 steps without an end"));
}

#[test]
fn timeouts_the_run_s_bound_and_the_tester_s_stop() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    let go = |text: &str, opts: &RunOptions| run_with(&checked(text), s.live(), opts).unwrap().0;
    let r = go(&test_of("  step slow wait plc_main.speed >= 3.0 timeout 300ms\n"), &RunOptions::live());
    assert_eq!(steps(&r), ["slow #1 timeout: plc_main.speed >= 3.0 did not hold within 300ms"]);
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "`slow` timed out: plc_main.speed >= 3.0 did not hold within 300ms"));
    assert_eq!(r.end_ms - r.start_ms, 300, "the clock moves to the bound");
    let r = go(&test_of("  step slow wait plc_main.speed >= 3.0 timeout 300ms\n             on timeout -> next\n  step next report \"routed\"\n"), &RunOptions::live());
    assert_eq!((r.verdict, texts(&r)), (RunVerdict::Passed, vec!["routed".to_string()]));
    let r = go(&format!("{HEAD}test \"t\" {{\n  id T\n  timeout 500ms\n  step w wait plc_main.speed >= 3.0\n}}\n"), &RunOptions::live());
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Aborted, Some("w"), "the run's bound of 500ms passed"));
    assert_eq!(steps(&r), ["w #1 cut short: "]);
    // The bound passing inside the wait moves the clock to it, and the run's own window replayed
    // ends the same way.
    assert_eq!((r.steps[0].end_ms - r.steps[0].start_ms, r.end_ms - r.start_ms), (500, 500));
    let text = format!("{HEAD}test \"t\" {{\n  id T\n  timeout 500ms\n  modes live, replay\n  step w wait plc_main.speed >= 3.0\n}}\n");
    let live = go(&text, &RunOptions::live());
    let replayed = go(&text, &RunOptions::replay(live.start_ms, live.end_ms));
    assert_eq!((replayed.reason.as_str(), replayed.steps[0].end_ms), ("the run's bound of 500ms passed", live.steps[0].end_ms));
    let opts = RunOptions::live();
    let stop = opts.stop.clone();
    let text = test_of("  step a report \"before\"\n  step w wait plc_main.speed >= 3.0\n");
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(400));
            stop.store(true, Ordering::SeqCst);
        });
        go(&text, &opts)
    });
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.reason.as_str()), (RunVerdict::Aborted, Some("w"), "stopped by the tester"));
    assert_eq!(steps(&r), ["a #1 pass: 1 reported", "w #1 cut short: "]);
    // A loop of no statement, past any bound: the tester's stop ends it, and its budget of
    // statements, never the run's bound, which is the run's clock's and a script box moves no clock.
    let spin = format!("{HEAD}test \"t\" {{\n  id T\n  timeout 1s\n  step s script {{ for i in 1 .. 100000000000 {{ }} }}\n}}\n");
    let opts = RunOptions::live();
    let stop = opts.stop.clone();
    let t0 = std::time::Instant::now();
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            stop.store(true, Ordering::SeqCst);
        });
        go(&spin, &opts)
    });
    assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    assert_eq!((r.verdict, r.reason.as_str(), steps(&r)), (RunVerdict::Aborted, "stopped by the tester", vec!["s #1 cut short: ".to_string()]));
    let r = go(&spin, &RunOptions::live());
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "`s`: more than 10000000 statements"));
    assert_eq!(steps(&r), ["s #1 error: more than 10000000 statements"]);
    // A wait for a duration.
    let t0 = std::time::Instant::now();
    let r = go(&test_of("  step d wait 300ms\n"), &RunOptions::live());
    assert!(t0.elapsed() >= Duration::from_millis(300));
    assert_eq!((steps(&r), r.end_ms - r.start_ms), (vec!["d #1 pass: waited 300ms".to_string()], 300));
}

#[test]
fn an_event_moves_the_clock_to_its_row_and_a_read_without_a_value_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of(concat!(
        "  step ev   await announce where a.lane == 2 timeout 5s -> a\n",
        "  step lane check a.lane == 2 and a.source == \"sorter\" and a.time <= now()\n",
        "  step none await announce where a.lane == 7 timeout 400ms -> b\n",
        "            on timeout -> ask\n",
        "  step ask  read answer.dest -> d\n",
    )));
    let (r, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    let st = steps(&r);
    assert!(st[0].starts_with("ev #1 pass: a row of `announce` after "), "{}", st[0]);
    assert_eq!(st[1], "lane #1 pass: a.lane == 2 and a.source == \"sorter\" and a.time <= now(): pass");
    assert_eq!(st[2], "none #1 timeout: no row of `announce` within 400ms");
    assert_eq!(st[3], "ask #1 error: `answer.dest` has no value yet", "portal stores no row of what it sends");
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "`ask`: `answer.dest` has no value yet"));
    // The event's step ends at the row's own stamp.
    assert_eq!(r.steps[0].end_ms, r.steps[1].start_ms);
    monotone(&r);
}

#[test]
fn a_write_the_switch_or_the_plant_refuses_mid_run_ends_its_step_in_error() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of("  step d wait 400ms\n  step w write plc_main.run := 1\n"));
    let l = s.live();
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            s.set_writes_armed(false);
        });
        run_with(&c, l, &RunOptions::live()).unwrap().0
    });
    assert_eq!(steps(&r)[1], "w #1 error: plc_main.run := 1: writes are turned off for this project");
    assert_eq!(writes(&r), [("plc_main.run := 1".to_string(), true, "writes are turned off for this project".to_string(), false)]);
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "`w`: plc_main.run := 1: writes are turned off for this project"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(tag(&bench, "run"), "Int32(0)");
    // The plant away: the server stopped, the write refused in portal's words.
    s.set_writes_armed(true);
    bench.stop();
    std::thread::sleep(Duration::from_millis(500));
    let (r, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    let w = &r.writes[0];
    assert!(w.sent && !w.ok, "{w:?}");
    assert_eq!(r.steps[1].outcome, Some(Outcome::Error));
    assert_eq!(r.verdict, RunVerdict::Aborted);
}

#[test]
fn functions_take_typed_inputs_give_their_outputs_and_keep_their_bound() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let f = concat!(
        "format = \"veredas-functions\"\nversion = 1\n\n",
        "function Twice(x: float) -> (y: float) {\n  y = x * 2\n}\n",
        "function Half(x: int) -> (y: int) {\n  if x > 5 { y = x - 4 }\n}\n",
        "function Never() -> (o: int) {\n  reads outcome\n  timeout 300ms\n  r = await outcome where r.dest == 99\n  o = 1\n}\n",
        "function Wait() {\n  reads plc_main\n  wait plc_main.speed >= 9.0 timeout 200ms\n}\n",
        "function Sum(n: int) -> (s: int, t: int) {\n  s = 0\n  for i in 1 .. n { s += i }\n  t, u = Pair()\n  for j in 3 .. 1 { s = 1000 }\n}\n",
        "function Pair() -> (a: int, b: int) {\n  a = 7\n  b = 8\n}\n",
        "function Outer() -> (o: int) {\n  timeout 300ms\n  o = Inner()\n}\n",
        "function Inner() -> (o: int) {\n  reads outcome\n  r = await outcome where r.dest == 99\n  o = 1\n}\n",
    );
    let go = |body: &str| run_with(&checked_with(&test_of(&format!("  use f.fn\n{body}")), f), s.live(), &RunOptions::live()).unwrap().0;
    let r = go("  step a call Twice(x: 2) -> y\n  step b call Sum(n: 4) -> s, t\n  step c report y, s, t\n");
    assert_eq!(steps(&r)[..2], ["a #1 pass: y 4".to_string(), "b #1 pass: s 10 · t 7".to_string()]);
    assert_eq!(texts(&r), ["y = 4", "s = 10", "t = 7"]);
    let r = go("  step a call Twice(x: \"2\") -> y\n");
    assert_eq!(steps(&r), ["a #1 error: Twice: input `x` takes float, not a text"]);
    let r = go("  step a call Half(x: 1) -> y\n");
    assert_eq!(steps(&r), ["a #1 error: Half: output `y` was never assigned"]);
    let r = go("  step a call Half(x: 8) -> y\n  step b report y\n");
    assert_eq!(texts(&r), ["y = 4"]);
    let r = go("  step a call Never() -> o\n");
    assert_eq!(steps(&r), ["a #1 timeout: Never: no row of `outcome` in time"]);
    assert_eq!((r.verdict, r.end_ms - r.start_ms), (RunVerdict::Aborted, 300));
    let r = go("  step a call Wait()\n");
    assert_eq!(steps(&r), ["a #1 timeout: Wait: plc_main.speed >= 9.0 did not hold in time"]);
    // A call inside a call keeps its caller's bound: 300 ms, not the run's ten minutes.
    let t0 = std::time::Instant::now();
    let r = go("  step a call Outer() -> o\n");
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    assert_eq!(steps(&r), ["a #1 timeout: Outer: Inner: no row of `outcome` in time"]);
    assert_eq!((r.verdict, r.end_ms - r.start_ms), (RunVerdict::Aborted, 300));
}

#[test]
fn rows_sharing_a_stamp_are_judged_together_and_each_awaited() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &with_rules(project_text(&bench, false, 4)));
    log_together(dir.path(), 100, &["x=0", "y=1"]);
    std::thread::sleep(Duration::from_millis(400));
    // `x` turns 1 as `y` turns 0, at one stamp: never both 1, so the wait times out.
    let c = checked(&test_of("  step w wait a.x == 1 and b.y == 1 timeout 1500ms\n"));
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            log_together(dir.path(), -500, &["x=1", "y=0"]);
        });
        run_with(&c, s.live(), &RunOptions::live()).unwrap().0
    });
    assert_eq!(steps(&r), ["w #1 timeout: a.x == 1 and b.y == 1 did not hold within 1500ms"]);
    // Two rows of one stamp and one after: three awaits take all three, in order.
    let c = checked(&test_of(concat!(
        "  step e1 await ev timeout 3s -> r1\n",
        "  step e2 await ev timeout 3s -> r2\n",
        "  step e3 await ev timeout 3s -> r3\n",
        "  step r  report r1.n, r2.n, r3.n\n",
    )));
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            log_together(dir.path(), -500, &["n=1", "n=2"]);
            log_text(dir.path(), -600, "n=3");
        });
        run_with(&c, s.live(), &RunOptions::live()).unwrap().0
    });
    assert_eq!(texts(&r), ["r1.n = 1", "r2.n = 2", "r3.n = 3"], "{:?}", steps(&r));
    assert_eq!(r.steps[0].end_ms, r.steps[1].end_ms, "the second row at the first's stamp");
    // A function's loop of awaits, as the worked example's counts: every row, however many share a
    // stamp.
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction Count(k: int) -> (s: int) {\n  reads ev\n  timeout 3s\n  s = 0\n  for i in 1 .. k {\n    e = await ev where e.n > 0\n    s += e.n\n  }\n}\n";
    let c = checked_with(&test_of("  use f.fn\n  step c call Count(k: 4) -> s\n  step r report s\n"), f);
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            log_together(dir.path(), -500, &["n=1", "n=2", "n=3"]);
            log_text(dir.path(), -600, "n=4");
        });
        run_with(&c, s.live(), &RunOptions::live()).unwrap().0
    });
    assert_eq!(texts(&r), ["s = 10"], "{:?}", steps(&r));
}

#[test]
fn values_compute_and_compare_as_the_reference_says() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    let go = |body: &str| run_with(&checked(&test_of(body)), s.live(), &RunOptions::live()).unwrap().0;
    let r = go(concat!(
        "  step e script {\n",
        "    a = 7 / 2; b = 7 % 3; c = 2 + 0.5; d = 90s - 30s; e = 2s * 3; f = 3s / 1500ms\n",
        "    g = \"4\" == 4; h = \" 4 \" == 4.0; i = \"x\" == 4; j = \"x\" != 4; k = true == 1; l = false < true\n",
        "    m = round(1.25, 1); n = seconds(1500ms); o = min(3, 1, 2); p = max(2.5, 1); q = int(3.9)\n",
        "    r = float(\"2.5\"); s = text(12); t = abs(-3); u = \"{s}{t}\"; v = round(2.5); w = 1 - 2 * 3\n",
        "    x = now() - now(); y = not (1 > 2 or 2 > 3) and 1 < 2; z = -(-1)\n",
        "  }\n",
    ));
    assert_eq!(
        steps(&r),
        ["e #1 pass: a = 3.5, b = 1, c = 2.5, d = 1min, e = 6s, f = 2, g = true, h = true, i = false, j = true, k = true, l = true, m = 1.3, n = 1.5, o = 1, p = 2.5, q = 3, r = 2.5, s = 12, t = 3, u = 123, v = 3, w = -5, x = 0s, y = true, z = 1"]
    );
    let one = |e: &str| steps(&go(&format!("  step e script {{ x = {e} }}\n"))).remove(0);
    assert_eq!(one("\"x\" < 4"), "e #1 error: \"x\" is no number to order against 4");
    assert_eq!(one("1 + \"a\""), "e #1 error: `+` does not take a whole number and a text");
    assert_eq!(one("true and 1"), "e #1 error: `and` takes truth values, not a whole number");
    assert_eq!(one("1s == 1"), "e #1 error: a duration and a whole number do not compare");
    assert_eq!(one("9223372036854775807 + 1"), "e #1 error: a whole number past its range");
    // Durations and moments past their range refused, never wrapped, saturated or a panic.
    let past = "e #1 error: a duration or a moment past its range";
    assert_eq!(one("9223372036854775807 * 1ms + 1ms"), past);
    assert_eq!(one("-9223372036854775807 * 1ms - 2ms"), past);
    assert_eq!(one("now() + 9223372036854775807 * 1ms"), past);
    assert_eq!(one("now() - (-9223372036854775807 * 1ms)"), past);
    assert_eq!(one("1s * 1e300"), past);
    assert_eq!(one("1s * 9223372036854775807"), past);
    assert_eq!(one("-(-9223372036854775807 * 1ms - 1ms)"), "e #1 error: a duration past its range");
    assert_eq!(one("abs(-9223372036854775807 * 1ms - 1ms)"), "e #1 error: a duration past its range");
    assert_eq!(one("round(1e300)"), "e #1 error: `round` gives a whole number past its range");
    assert_eq!(one("round(-1e19)"), "e #1 error: `round` gives a whole number past its range");
    assert_eq!(one("round(9.2e18)"), "e #1 pass: x = 9200000000000000000");
    assert_eq!(one("2s * 1.5"), "e #1 pass: x = 3s");
    // A whole number and a duration never pass through a float's 53 bits.
    assert_eq!(one("round(9223372036854775807)"), "e #1 pass: x = 9223372036854775807");
    assert_eq!(one("round(9007199254740993)"), "e #1 pass: x = 9007199254740993");
    assert_eq!(one("int(9.21e18)"), "e #1 pass: x = 9210000000000000000");
    assert_eq!(one("int(-9.223372036854775808e18)"), "e #1 pass: x = -9223372036854775808");
    assert_eq!(one("int(1e19)"), "e #1 error: `int` gives a whole number past its range");
    assert_eq!(one("9007199254740993ms * 1.0"), "e #1 pass: x = 9007199254740993ms");
    assert_eq!(one("9007199254740993ms * 2.0"), "e #1 pass: x = 18014398509481986ms");
    assert_eq!(one("3ms * 0.5"), "e #1 pass: x = 2ms");
    assert_eq!(one("-3ms * 0.5"), "e #1 pass: x = -2ms");
    assert_eq!(one("1ms * 0.1"), "e #1 pass: x = 0s");
    assert_eq!(one("0.25 * 6ms"), "e #1 pass: x = 2ms");
    // A whole number compared with a float, or with a text holding one, exactly.
    assert_eq!(one("9007199254740993 == 9007199254740992.0"), "e #1 pass: x = false");
    assert_eq!(one("9007199254740993 > 9007199254740992.0"), "e #1 pass: x = true");
    assert_eq!(one("9223372036854775807 == 9223372036854775806.0"), "e #1 pass: x = false");
    assert_eq!(one("9223372036854775807 < 9223372036854775806.0"), "e #1 pass: x = true");
    assert_eq!(one("\"9007199254740993\" == 9007199254740992"), "e #1 pass: x = false");
    assert_eq!(one("9007199254740992.0 == \"9007199254740993\""), "e #1 pass: x = false");
    assert_eq!(one("-1 > -1.5"), "e #1 pass: x = true");
    assert_eq!(one("2 == 2.0"), "e #1 pass: x = true");
    // A loop to the largest whole number ends there.
    let r = go("  step f script { for i in 9223372036854775806 .. 9223372036854775807 { x = i } }\n");
    assert_eq!(steps(&r), ["f #1 pass: i = 9223372036854775807, x = 9223372036854775807"]);
    assert_eq!(one("int(\"x\")"), "e #1 error: \"x\" is no whole number");
    // Plant data has its type as portal stores it, a text read as a number where compared with one.
    let r = go("  step c check plc_main.level > 0.5 and plc_main.run == 0 and plc_main.run == false\n");
    assert_eq!(r.verdict, RunVerdict::Passed);
}

#[test]
fn a_live_read_waits_until_its_moment_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    let c = checked(&test_of("  step c check readings.temp == 21.5\n"));
    // A line stamped about 40 ms before the run starts, written 60 ms after it: read, because the
    // run reads its start only once it is 250 ms past.
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(60));
            log_line(dir.path(), 100, "21.5");
        });
        run_with(&c, s.live(), &RunOptions::live()).unwrap().0
    });
    assert_eq!(steps(&r), ["c #1 pass: readings.temp == 21.5: pass"]);
    // A line stamped after the run's moment is not that moment's: the check reads the value before it.
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(60));
            log_line(dir.path(), -2000, "30.5");
        });
        run_with(&checked(&test_of("  step c check readings.temp == 21.5\n")), s.live(), &RunOptions::live()).unwrap().0
    });
    assert_eq!(steps(&r), ["c #1 pass: readings.temp == 21.5: pass"]);
}

// ---- refute round 2 ------------------------------------------------------------------------

#[test]
fn an_awaited_row_s_copies_are_followed_in_any_order_and_a_name_s_tables_judged_together() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    // A copy that stands before its await in the text, reached by the routes after it: refused,
    // nothing written.
    let c = checked(&test_of(concat!(
        "  step go   write plc_main.run := 1\n",
        "            on pass -> take\n",
        "  step copy script { r = o }\n",
        "  step use  check r.nosuch == 1\n",
        "  step take await plc_main timeout 3s -> o\n",
        "            on pass -> copy\n",
    )));
    let e = run_with(&c, s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, ["`plc_main` has no column `nosuch`, read as `r.nosuch`"]);
    assert_eq!(tag(&bench, "run"), "Int32(0)", "nothing written");
    // In a function, through two copies in a loop, each before the statement it follows.
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction F() -> (s: int) {\n  reads plc_main\n  timeout 1s\n  s = 0\n  for i in 1 .. 3 {\n    if i == 3 {\n      s = p.nosuch\n    }\n    if i >= 2 {\n      p = q\n    }\n    q = await plc_main\n  }\n}\n";
    let e = run_with(&checked_with(&test_of("  use f.fn\n  step c call F() -> s\n"), f), s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, ["`plc_main` has no column `nosuch`, read as `p.nosuch`"]);
    // A name rebound by an await of another table: each read judged against the tables the name
    // may hold, refused only when none of them has the column.
    log_line(dir.path(), -1500, "21.5");
    let rebound = |read: &str| {
        checked(&test_of(&format!(
            "  step a await plc_main timeout 2s -> r\n  step b check r.level > 0.5\n  step c await readings timeout 3s -> r\n  step d check {read}\n"
        )))
    };
    let (r, _) = run_with(&rebound("r.temp > 0"), s.live(), &RunOptions::live()).unwrap();
    assert_eq!((r.verdict, steps(&r)[3].as_str()), (RunVerdict::Passed, "d #1 pass: r.temp > 0: pass"), "{:?}", steps(&r));
    let e = run_with(&rebound("r.nosuch > 0"), s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, ["`plc_main` has no column `nosuch`, read as `r.nosuch`", "`readings` has no column `nosuch`, read as `r.nosuch`"]);
}

#[test]
fn awaits_over_a_view_take_each_row_of_a_stamp_once() {
    // A view over the events: its rows carry no source or sequence, so a row is told by its place
    // among the rows of its stamp, in one order whatever the store's.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let view = "\n[[table]]\nname = \"evv\"\nsql = \"SELECT index_ms, n FROM ev\"\n";
    let s = session(dir.path(), &(with_rules(project_text(&bench, false, 4)) + view));
    let c = checked(&test_of(concat!(
        "  step e1 await evv timeout 3s -> r1\n",
        "  step e2 await evv timeout 3s -> r2\n",
        "  step e3 await evv timeout 3s -> r3\n",
        "  step r  report r1.n, r2.n, r3.n\n",
    )));
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            log_together(dir.path(), -500, &["n=2", "n=1"]);
            log_text(dir.path(), -600, "n=3");
        });
        run_with(&c, s.live(), &RunOptions::live()).unwrap().0
    });
    let mut first_two = texts(&r)[..2].to_vec();
    first_two.sort();
    assert_eq!((first_two, texts(&r)[2].as_str()), (vec!["r1.n = 1".to_string(), "r2.n = 2".to_string()], "r3.n = 3"), "{:?}", steps(&r));
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction Count(k: int) -> (s: int) {\n  reads evv\n  timeout 3s\n  s = 0\n  for i in 1 .. k {\n    e = await evv where e.n > 0\n    s += e.n\n  }\n}\n";
    let c = checked_with(&test_of("  use f.fn\n  step c call Count(k: 4) -> s\n  step r report s\n"), f);
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            log_together(dir.path(), -500, &["n=1", "n=2", "n=3"]);
            log_text(dir.path(), -600, "n=4");
        });
        run_with(&c, s.live(), &RunOptions::live()).unwrap().0
    });
    assert_eq!(texts(&r), ["s = 10"], "{:?}", steps(&r));
}

#[test]
fn a_wait_and_a_read_agree_on_a_stamp_two_keys_share() {
    // Two keys of one table at one stamp, `x` 0 under one and 1 under the other: the wait judges
    // the stamp by the value a read there finds, so the check after it agrees with it.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let keyed = "\n[[rule]]\nname = \"kt\"\npattern = \" k=(?P<k>[a-z]) x=(?P<x>[0-9]+)\"\nscope = \"plant\"\nkey = \"k\"\ntypes = { x = \"int\" }\n";
    let s = session(dir.path(), &(project_text(&bench, false, 4) + keyed));
    let c = checked(&test_of("  step w wait kt.x == 1 timeout 1500ms\n          on timeout -> c\n  step c check kt.x == 1\n"));
    for lines in [["k=b x=0", "k=a x=1"], ["k=a x=1", "k=b x=0"]] {
        let r = std::thread::scope(|sc| {
            sc.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                log_together(dir.path(), -500, &lines);
            });
            run_with(&c, s.live(), &RunOptions::live()).unwrap().0
        });
        let st = steps(&r);
        assert_eq!(st[0].starts_with("w #1 pass"), st[1].starts_with("c #1 pass"), "{lines:?}: {st:?}");
        std::thread::sleep(Duration::from_millis(600));
    }
}

#[test]
fn a_live_wait_for_a_duration_waits_until_its_bound_has_passed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    // A function's wait past its own bound of 1 s: the call times out at the bound, once the plant
    // has reached it, so the write after it and what it causes come after.
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction Pause() {\n  timeout 1s\n  wait 5s\n}\n";
    let c = checked_with(&test_of(concat!(
        "  use f.fn\n",
        "  step p    call Pause()\n",
        "            on timeout -> go\n",
        "  step go   write plc_main.run := 1\n",
        "  step fast wait plc_main.speed >= 2.0 timeout 5s\n",
    )), f);
    let begun = std::time::Instant::now();
    let (r, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert!(begun.elapsed() >= Duration::from_secs(1), "{:?}", begun.elapsed());
    let (p, go) = (&r.steps[0], &r.steps[1]);
    assert_eq!((p.outcome, p.end_ms - p.start_ms, go.start_ms), (Some(Outcome::Timeout), 1000, p.end_ms), "{:?}", steps(&r));
    let plant = veredas_comissionador::plant::LivePlant::new(s.live()).unwrap();
    use veredas_comissionador::plant::Plant;
    let table = plant.tables().unwrap().into_iter().find(|t| t.name == "plc_main").unwrap();
    let rows = plant.rows(&table, 0, plant.now_ms()).unwrap();
    let reacted = rows.iter().find(|row| row.field("speed") == Some(Some(veredas_comissionador::eval::Value::Float(2.0)))).expect("the reaction stored").time_ms;
    assert!(reacted > go.start_ms, "the reaction after the write: {reacted} against {}", go.start_ms);
    // A step's wait past the run's bound: the run ends at the bound, once it has passed.
    let begun = std::time::Instant::now();
    let (r, _) = run_with(&checked(&format!("{HEAD}test \"t\" {{\n  id T\n  timeout 500ms\n  step w wait 5min\n}}\n")), s.live(), &RunOptions::live()).unwrap();
    assert!(begun.elapsed() >= Duration::from_millis(500), "{:?}", begun.elapsed());
    assert_eq!((r.reason.as_str(), r.end_ms - r.start_ms), ("the run's bound of 500ms passed", 500));
}

// ---- refute round 3 ------------------------------------------------------------------------

#[test]
fn the_run_s_bound_is_the_run_s_clock_s_live_and_replayed_alike() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let go = |text: &str, opts: &RunOptions| run_with(&checked(text), s.live(), opts).unwrap().0;
    let bounded = |timeout: &str, body: &str| format!("{HEAD}test \"t\" {{\n  id T\n  timeout {timeout}\n  modes live, replay\n{body}}}\n");
    // A read's settle puts the machine past the bound, not the run's clock: the run goes on, live
    // and replayed alike.
    for text in [
        bounded("1s", "  step d wait 800ms\n  step c check plc_main.level > 0.5\n  step s stop pass\n"),
        bounded("200ms", "  step r read plc_main.level -> l\n  step s report l\n"),
        bounded("300ms", "  step a script { for i in 1 .. 3000000 { } }\n  step s stop pass\n"),
    ] {
        let live = go(&text, &RunOptions::live());
        let replayed = go(&text, &RunOptions::replay(live.start_ms, live.end_ms));
        assert_eq!(live.verdict, RunVerdict::Passed, "{:?}", steps(&live));
        assert_eq!((replayed.verdict, texts(&replayed), steps(&replayed)), (live.verdict, texts(&live), steps(&live)), "{text}");
    }
    // The bound judged between a step's actions: past it, the next action is not run, and the
    // clock never goes back to it.
    let text = bounded("100ms", "  step a wait 150ms\n         write plc_main.run := 1\n");
    let r = go(&text, &RunOptions::live());
    assert_eq!((r.verdict, r.reason.as_str(), r.writes.len()), (RunVerdict::Aborted, "the run's bound of 100ms passed", 0), "{:?}", steps(&r));
    assert_eq!(tag(&bench, "run"), "Int32(0)");
    // A write before the bound on the run's clock goes out; its answer, past the bound, ends the
    // step before the wait after it, the clock left at the answer.
    let text = bounded("100ms", "  step a check plc_main.level > 0.5\n         write plc_main.run := 1\n         wait plc_main.speed >= 3.0\n");
    let r = go(&text, &RunOptions::live());
    assert_eq!((r.verdict, r.reason.as_str(), r.writes.len()), (RunVerdict::Aborted, "the run's bound of 100ms passed", 1), "{:?}", steps(&r));
    assert!(r.steps[0].end_ms - r.steps[0].start_ms >= veredas_comissionador::run::SETTLE_MS, "never back to the bound: {:?}", r.steps[0]);
    monotone(&r);
}

#[test]
fn a_replay_ends_where_the_tester_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    std::thread::sleep(Duration::from_millis(600));
    let now = veredas_portal::serve::store_now_ms();
    // A step routed back to itself, replayed: stopped before it starts, and 200 ms in.
    let text = test_of("  step a wait plc_main.level == 0.9\n         on pass -> a\n");
    let opts = RunOptions::replay(now - 500, now - 300);
    opts.stop.store(true, Ordering::SeqCst);
    let r = run_with(&checked(&text), s.live(), &opts).unwrap().0;
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "stopped by the tester"));
    let opts = RunOptions::replay(now - 500, now - 300);
    let stop = opts.stop.clone();
    let t0 = std::time::Instant::now();
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            stop.store(true, Ordering::SeqCst);
        });
        run_with(&checked(&text), s.live(), &opts).unwrap().0
    });
    assert_eq!((r.verdict, r.reason.as_str()), (RunVerdict::Aborted, "stopped by the tester"), "{:?}", t0.elapsed());
    // A retry of 1000 the same.
    let text = test_of("  step a wait plc_main.level == 0.9\n         on pass -> retry a max 1000\n");
    let opts = RunOptions::replay(now - 500, now - 300);
    opts.stop.store(true, Ordering::SeqCst);
    let r = run_with(&checked(&text), s.live(), &opts).unwrap().0;
    assert_eq!((r.verdict, r.reason.as_str(), r.steps.len()), (RunVerdict::Aborted, "stopped by the tester", 0));
}

#[test]
fn a_row_copied_through_min_or_max_is_bound_as_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of(concat!(
        "  step go   write plc_main.run := 1\n",
        "  step take await plc_main timeout 3s -> o\n",
        "  step copy script { r = min(o) }\n",
        "  step use  check r.nosuch == 1\n",
    )));
    let e = run_with(&c, s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, ["`plc_main` has no column `nosuch`, read as `r.nosuch`"]);
    assert_eq!(tag(&bench, "run"), "Int32(0)", "nothing written");
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction F() -> (s: int) {\n  reads plc_main\n  timeout 1s\n  q = await plc_main\n  p = max(q)\n  s = p.nosuch\n}\n";
    let e = run_with(&checked_with(&test_of("  use f.fn\n  step c call F() -> s\n"), f), s.live(), &RunOptions::live()).unwrap_err();
    assert_eq!(e.0, ["`plc_main` has no column `nosuch`, read as `p.nosuch`"]);
}

// ---- the architect's Q314 (b) ----------------------------------------------------------------

#[test]
fn a_run_arms_the_switch_for_the_writes_it_was_confirmed_for_and_disarms_it_after() {
    use veredas_comissionador::run::{Event, gated_writes, run, writes_fingerprint};
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = std::rc::Rc::new(session(dir.path(), &project_text(&bench, false, 4)));
    let arming = || {
        let s = s.clone();
        veredas_comissionador::plant::LivePlant::with_arming(s.live(), Box::new(move |on| s.set_writes_armed(on))).unwrap()
    };
    let c = checked(&test_of("  step go write plc_main.run := 1\n  step w wait plc_main.speed >= 2.0 timeout 5s\n"));
    let writes = gated_writes(&c);
    let fp = writes_fingerprint(&writes);
    assert_eq!((writes, fp.len()), (vec!["write plc_main.run := 1".to_string()], 16));
    // Not confirmed, or confirmed for another list: refused, the fingerprint named, nothing written.
    let p = arming();
    let e = run(&c, &p, &RunOptions::live(), &mut |_| {}).unwrap_err();
    assert_eq!(e.0, [format!("the test writes, and portal's write switch is off: its 1 gated write, confirmed as `{fp}`, arm it for the run")]);
    let e = run(&c, &p, &RunOptions::live().confirming("feedfacefeedface"), &mut |_| {}).unwrap_err();
    assert_eq!(e.0, [format!("the test writes, and portal's write switch is off: `feedfacefeedface` confirms no list of this test's; its 1 gated write, confirmed as `{fp}`, arm it for the run")]);
    assert_eq!((tag(&bench, "run"), s.live().writes_armed()), ("Int32(0)".to_string(), false));
    // Confirmed: armed for the run, the write lands, the switch off again after.
    let mut armed_at_write = None;
    let r = run(&c, &p, &RunOptions::live().confirming(fp.clone()), &mut |e| {
        if let Event::Write(_) = e {
            armed_at_write = Some(s.live().writes_armed());
        }
    })
    .unwrap();
    assert_eq!((r.verdict, armed_at_write, s.live().writes_armed()), (RunVerdict::Passed, Some(true), false), "{:?}", steps(&r));
    assert_eq!(tag(&bench, "run"), "Int32(1)");
    // A run that aborts disarms too.
    let aborting = checked(&test_of("  step go write plc_main.run := 1\n  step w wait plc_main.speed >= 9.0 timeout 300ms\n"));
    let r = run(&aborting, &p, &RunOptions::live().confirming(writes_fingerprint(&gated_writes(&aborting))), &mut |_| {}).unwrap();
    assert_eq!((r.verdict, s.live().writes_armed()), (RunVerdict::Aborted, false));
    // A switch the run found armed: left as found, confirmed or not.
    s.set_writes_armed(true);
    let r = run(&c, &p, &RunOptions::live(), &mut |_| {}).unwrap();
    assert_eq!((r.verdict, s.live().writes_armed()), (RunVerdict::Passed, true));
    s.set_writes_armed(false);
    // A plant its embedder handed no arming: refused as before, confirmed or not.
    drop(p);
    let bare = veredas_comissionador::plant::LivePlant::new(s.live()).unwrap();
    let e = run(&c, &bare, &RunOptions::live().confirming(fp), &mut |_| {}).unwrap_err();
    assert_eq!(e.0, ["the test writes, and portal's write switch is off"]);
}
