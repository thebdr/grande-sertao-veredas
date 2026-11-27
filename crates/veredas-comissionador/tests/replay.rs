//! The engine, replayed (contract: C-203): a checked test run over a stored window of a portal
//! session fed by the bench, on the rows' own stamps, writes noted and never sent, a live run's own
//! window replayed to the same verdict and values.

mod common;

use std::time::{Duration, Instant};

use common::*;
use veredas_comissionador::model::Outcome;
use veredas_comissionador::run::{RunOptions, RunResult, RunVerdict};

fn texts(r: &RunResult) -> Vec<String> {
    r.reported.iter().map(|x| x.text.clone()).collect()
}

/// A step's text with the length of its wait taken out: live, a write moves the clock to the
/// plant's answer; replayed, the clock does not move, so a wait after a write is longer by that.
fn shape(r: &RunResult) -> Vec<String> {
    steps(r)
        .into_iter()
        .map(|s| match s.find(" held after ") {
            Some(i) => format!("{} held", &s[..i]),
            None => s.replace(": sent", "").replace(": not sent: replay", "").replace(": Good", ""),
        })
        .collect()
}

#[test]
fn a_live_run_s_own_window_replays_to_the_same_verdict_and_values() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&blocks("test-schema.md", "test").remove(0));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!(live.verdict, RunVerdict::Passed);
    let t0 = Instant::now();
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert!(t0.elapsed() < Duration::from_secs(3), "a replay does not wait for the clock: {:?}", t0.elapsed());
    assert_eq!((r.verdict, r.decided_at.clone(), r.reason.clone()), (live.verdict, live.decided_at.clone(), live.reason.clone()));
    assert_eq!(shape(&r), shape(&live));
    assert_eq!(texts(&r), texts(&live));
    assert_eq!(r.criteria, live.criteria);
    // The sends are noted, never sent.
    let w: Vec<(String, bool, String, bool)> = r.writes.iter().map(|w| (w.text.clone(), w.sent, w.answer.clone(), w.ok)).collect();
    assert_eq!(w, [
        ("sorter.command(code: 1)".to_string(), false, "not sent: replay".to_string(), true),
        ("sorter.command(code: 0)".to_string(), false, "not sent: replay".to_string(), true),
    ]);
    assert_eq!(steps(&r)[0], "start #1 pass: sorter.command(code: 1): not sent: replay");
    // The function met the same rows: the same twenty outcomes, ending at the same stamp.
    assert_eq!(r.steps[2].end_ms, live.steps[2].end_ms);
    assert_eq!((r.start_ms, r.end_ms <= live.end_ms), (live.start_ms, true));
    // Replayed again, the same window gives the same run, to the millisecond.
    let (again, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!(again, r);
}

#[test]
fn a_replay_never_writes_and_reads_the_stamps_a_write_caused() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of(concat!(
        "  step go    write plc_main.run := 1\n",
        "  step fast  wait plc_main.speed >= 2.0 timeout 5s\n",
        "  step look  read plc_main.speed -> sp\n",
        "  step done  report sp\n",
    )));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!(live.verdict, RunVerdict::Passed);
    // Put the plant back, turn the switch off: a replay needs neither.
    bench.server("plc").unwrap().set("run", 0i32);
    bench.server("plc").unwrap().set("speed", 1.5f64);
    s.set_writes_armed(false);
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!(r.verdict, RunVerdict::Passed);
    assert_eq!(steps(&r)[0], "go #1 pass: plc_main.run := 1: not sent: replay");
    assert_eq!(texts(&r), ["sp = 2"]);
    // The stamp the reaction's row carries is where both runs met it.
    assert_eq!(r.steps[1].end_ms, live.steps[1].end_ms);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(tag(&bench, "run"), "Int32(0)", "nothing reached the plant");
}

#[test]
fn a_replay_ends_where_its_window_ends_and_times_out_on_its_own_clock() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    std::thread::sleep(Duration::from_millis(1500));
    let now = veredas_portal::serve::store_now_ms();
    let (from, to) = (now - 1200, now - 300);
    let go = |text: &str, from: i64, to: i64| run_with(&checked(text), s.live(), &RunOptions::replay(from, to));
    // A condition true at the window's start holds at once: values stored before it count.
    let (r, _) = go(&test_of("  step a wait plc_main.level == 0.9 timeout 100ms\n"), from, to).unwrap();
    assert_eq!(steps(&r), ["a #1 pass: plc_main.level == 0.9 held after 0s"]);
    assert_eq!(r.end_ms, from);
    // A timeout inside the window: the replay's clock, not the machine's.
    let t0 = Instant::now();
    let (r, _) = go(&test_of("  step a wait plc_main.speed >= 3.0 timeout 500ms\n"), from, to).unwrap();
    assert!(t0.elapsed() < Duration::from_millis(400), "{:?}", t0.elapsed());
    assert_eq!(steps(&r), ["a #1 timeout: plc_main.speed >= 3.0 did not hold within 500ms"]);
    assert_eq!((r.verdict, r.end_ms), (RunVerdict::Aborted, from + 500));
    // A wait the window ends before: aborted, the step cut short, the window's end named.
    let (r, _) = go(&test_of("  step w report \"before\"\n  step a wait plc_main.speed >= 3.0 timeout 5s\n"), from, to).unwrap();
    assert_eq!(steps(&r), ["w #1 pass: 1 reported", "a #1 cut short: "]);
    assert_eq!(r.steps[1].outcome, None);
    assert_eq!((r.verdict, r.decided_at.as_deref()), (RunVerdict::Aborted, Some("a")));
    assert_eq!(r.reason, format!("the window ended at {}, before the step decided", veredas_comissionador::eval::time_text(to)));
    assert_eq!(r.end_ms, to);
    // A wait for a duration moves the clock, inside the window or to its end.
    let (r, _) = go(&test_of("  step d wait 600ms\n"), from, to).unwrap();
    assert_eq!((r.verdict, r.end_ms), (RunVerdict::Passed, from + 600));
    let (r, _) = go(&test_of("  step d wait 2s\n"), from, to).unwrap();
    assert_eq!((r.verdict, r.end_ms, r.steps[0].outcome), (RunVerdict::Aborted, to, None));
    // The window ends before the run's bound and before the wait: the window's end named, the
    // clock at it, never past; a function's wait under a call bound past the window the same.
    let window = format!("the window ended at {}, before the step decided", veredas_comissionador::eval::time_text(to));
    let (r, _) = go(&format!("{HEAD}test \"t\" {{\n  id T\n  timeout 2s\n  step d wait 5s\n}}\n"), from, to).unwrap();
    assert_eq!((r.verdict, r.reason.as_str(), r.end_ms, r.steps[0].outcome), (RunVerdict::Aborted, window.as_str(), to, None));
    let f = "format = \"veredas-functions\"\nversion = 1\n\nfunction Pause() {\n  timeout 2s\n  wait 5s\n}\n";
    let c = checked_with(&test_of("  use f.fn\n  step c call Pause()\n"), f);
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(from, to)).unwrap();
    assert_eq!((r.verdict, r.reason.as_str(), r.end_ms, r.steps[0].outcome), (RunVerdict::Aborted, window.as_str(), to, None));
    // An event the window holds, at its own stamp.
    let (r, _) = go(&test_of("  step e await plc_main -> p\n  step c check p.time <= now()\n"), from, to).unwrap();
    assert_eq!(r.verdict, RunVerdict::Passed);
    assert!(r.steps[0].end_ms > from && r.steps[0].end_ms <= to);
    assert_eq!(r.steps[0].outcome, Some(Outcome::Pass));
    // Refused before anything: a test that runs live only, a window that ends before it starts.
    assert_eq!(go(&format!("{HEAD}test \"t\" {{\n  id T\n  modes live\n  step a stop pass\n}}\n"), from, to).unwrap_err().0, ["the test runs live, not replay"]);
    assert_eq!(go(&test_of("  step a stop pass\n"), to, from).unwrap_err().0, ["the window ends before it starts"]);
}

#[test]
fn a_replay_s_write_moves_no_clock_where_the_live_one_moved_to_the_plant_s_answer() {
    // After a step that read the plant, the live clock sits the read's settle (250 ms) behind the
    // machine; a write moves it to the plant's answer. A replay's write moves nothing, so a wait
    // after it whose bound is tighter than that lag and the reaction together times out in the
    // replay where it held live (docs/questions.md Q318, the claim narrowed meanwhile).
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of(concat!(
        "  step a    wait plc_main.level == 0.9 timeout 1s\n",
        "  step go   write plc_main.run := 1\n",
        "  step fast wait plc_main.speed >= 2.0 timeout 300ms\n",
    )));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!((live.verdict, live.decided_at.as_deref()), (RunVerdict::Passed, Some("fast")), "{:?}", steps(&live));
    let go = &live.steps[1];
    assert!(go.end_ms - go.start_ms >= veredas_comissionador::run::SETTLE_MS, "the live write moved the clock: {go:?}");
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!(r.steps[1].end_ms, r.steps[1].start_ms, "the replay's write moved nothing");
    assert_eq!(steps(&r)[2], "fast #1 timeout: plc_main.speed >= 2.0 did not hold within 300ms");
    assert_eq!(r.verdict, RunVerdict::Aborted);
}

#[test]
fn a_run_s_bound_passed_by_a_write_s_answer_replays_on_past_it() {
    // A write's answer lands past the run's bound live and ends the run at the next step; replayed,
    // the write moves no clock and the run goes on (Q318, the claim narrowed: a write whose answer
    // passes the run's bound).
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&format!("{HEAD}test \"t\" {{\n  id T\n  timeout 100ms\n  step a check plc_main.level > 0.5\n         write plc_main.run := 1\n  step b report \"done\"\n}}\n"));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!((live.verdict, live.decided_at.as_deref(), live.reason.as_str()), (RunVerdict::Aborted, Some("b"), "the run's bound of 100ms passed"));
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!((r.verdict, r.decided_at.as_deref()), (RunVerdict::Passed, Some("b")));
}

#[test]
fn a_replay_s_clock_stays_the_write_s_lag_behind_until_a_wait_meets_a_row() {
    // Refute round 5 (C-203): the lag a replayed write leaves carries on into every later step
    // until a wait or an await meets a row (Q318, the claim narrowed so). A duration wait after the
    // write ends the run at its bound inside it live, its clock past the plant's answer, and passes
    // replayed;
    // a check after a duration wait reads the plant after the reaction live and before it replayed.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&format!(
        "{HEAD}test \"t\" {{\n  id T\n  timeout 1s\n  step a check plc_main.level > 0.5\n  step go write plc_main.run := 1\n  step d wait 800ms\n  step r report \"done\"\n}}\n"
    ));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!((live.verdict, live.decided_at.as_deref(), live.reason.as_str()), (RunVerdict::Aborted, Some("d"), "the run's bound of 1s passed"), "{:?}", steps(&live));
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!((r.verdict, r.decided_at.as_deref()), (RunVerdict::Passed, Some("r")), "{:?}", steps(&r));
    bench.server("plc").unwrap().set("run", 0);
    bench.server("plc").unwrap().set("speed", 1.5);
    std::thread::sleep(Duration::from_millis(300));
    let c = checked(&test_of("  step a check plc_main.level > 0.5\n  step go write plc_main.run := 1\n  step d wait 350ms\n  step c check plc_main.speed >= 2.0\n"));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!(live.verdict, RunVerdict::Passed, "{:?}", steps(&live));
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!((r.verdict, steps(&r)[3].as_str()), (RunVerdict::Failed, "c #1 fail: plc_main.speed >= 2.0: fail (plc_main.speed = 1.5)"), "{:?}", steps(&r));
}

#[test]
fn a_window_before_the_store_holds_a_table_the_test_reads_is_refused() {
    // C-205's refute round 6: the embedder tells the plant from which moment the store holds each
    // table's rows (`LivePlant::with_rows_from`); a replay whose window opens before it, for a table
    // the test reads, is refused with the table named, and a table it does not read refuses nothing.
    use veredas_comissionador::eval::time_text;
    use veredas_comissionador::plant::LivePlant;
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, false, 4));
    let now = veredas_portal::serve::store_now_ms();
    let held = [("plc_main".to_string(), now - 1_000), ("Readings".to_string(), now - 60_000)].into_iter().collect();
    let p = LivePlant::new(s.live()).unwrap().with_rows_from(held);
    let (from, to) = (now - 30_000, now - 20_000);
    let go = |text: &str| veredas_comissionador::run::run(&checked(&test_of(text)), &p, &RunOptions::replay(from, to), &mut |_| {});
    let e = go("  step c check plc_main.speed == 1.5\n  step r report readings.temp\n").unwrap_err();
    assert_eq!(e.0, [format!("the window opens at {}, before the session's store holds `plc_main.speed`, from {}", time_text(from), time_text(now - 1_000))]);
    assert!(go("  step r report readings.temp\n").is_ok(), "a table held from before the window refuses nothing");
}

#[test]
fn a_write_that_failed_live_replays_not_sent_and_passing() {
    // Refute round 6 (C-203): a replayed write is not sent and its step passes (Q315 (a)), so a live
    // run whose write the plant refused replays to another verdict: the claim is narrowed to live
    // runs whose every write and send was taken (docs/questions.md Q322 asks whether a replay should
    // take the live answer from portal's table of writes, Q318 (a)).
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let text = project_text(&bench, true, 4).replacen("series = \"level\", type = \"float\" }", "series = \"level\", type = \"float\", writable = true }", 1);
    let s = session(dir.path(), &text);
    let c = checked(&test_of("  step w write plc_main.level := 1.0\n  step r report \"done\"\n"));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!((live.verdict, live.decided_at.as_deref()), (RunVerdict::Aborted, Some("w")), "{:?}", steps(&live));
    assert!(live.writes[0].sent && !live.writes[0].ok, "{:?}", live.writes);
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!((r.verdict, r.decided_at.as_deref(), r.writes[0].answer.as_str()), (RunVerdict::Passed, Some("r"), "not sent: replay"), "{:?}", steps(&r));
}

#[test]
fn a_live_run_the_tester_stopped_replays_to_its_own_verdict() {
    // Refute round 7 (C-203): a live run the tester stopped ended on the stop, which its window does
    // not hold: replayed, it reaches the verdict the test gives. The claim is narrowed to live runs
    // that ended on their own verdict, not the tester's stop.
    use veredas_comissionador::run::Event;
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of("  step a check plc_main.level == 0.9\n  step b report \"done\"\n"));
    let p = veredas_comissionador::plant::LivePlant::new(s.live()).unwrap();
    let opts = RunOptions::live();
    let stop = opts.stop.clone();
    let live = veredas_comissionador::run::run(&c, &p, &opts, &mut |e| {
        if let Event::StepEnded(r) = e
            && r.step == "a"
        {
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    })
    .unwrap();
    assert_eq!((live.verdict, live.reason.as_str()), (RunVerdict::Aborted, "stopped by the tester"), "{:?}", steps(&live));
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!((r.verdict, r.decided_at.as_deref()), (RunVerdict::Passed, Some("b")), "{:?}", steps(&r));
}

#[test]
fn a_replay_reads_the_plant_right_after_a_write_as_it_stood_before_it() {
    // The same lag, with a loose bound: an await right after the write takes a row stamped before
    // the write in the replay, where the live one took one after the plant's answer, and a read
    // after it reads the plant before the write (docs/questions.md Q318, the claim narrowed: no
    // plant data read right after a write).
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let c = checked(&test_of(concat!(
        "  step a    wait plc_main.level == 0.9 timeout 1s\n",
        "  step go   write plc_main.run := 1\n",
        "  step next await plc_main timeout 5s -> p\n",
        "  step look read plc_main.run -> x\n",
        "  step r    report p.run, x\n",
    )));
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!(live.verdict, RunVerdict::Passed, "{:?}", steps(&live));
    let answered = live.steps[1].end_ms;
    assert!(live.steps[2].end_ms >= answered, "live, the row after the plant's answer");
    let (r, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    assert_eq!(r.verdict, RunVerdict::Passed, "{:?}", steps(&r));
    assert!(r.steps[2].end_ms < answered, "replayed, a row before the write: {} against {answered}", r.steps[2].end_ms);
    assert_eq!(texts(&r), ["p.run = 0", "x = 0"]);
}

