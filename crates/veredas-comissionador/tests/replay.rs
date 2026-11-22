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
    // An event the window holds, at its own stamp.
    let (r, _) = go(&test_of("  step e await plc_main -> p\n  step c check p.time <= now()\n"), from, to).unwrap();
    assert_eq!(r.verdict, RunVerdict::Passed);
    assert!(r.steps[0].end_ms > from && r.steps[0].end_ms <= to);
    assert_eq!(r.steps[0].outcome, Some(Outcome::Pass));
    // Refused before anything: a test that runs live only, a window that ends before it starts.
    assert_eq!(go(&format!("{HEAD}test \"t\" {{\n  id T\n  modes live\n  step a stop pass\n}}\n"), from, to).unwrap_err().0, ["the test runs live, not replay"]);
    assert_eq!(go(&test_of("  step a stop pass\n"), to, from).unwrap_err().0, ["the window ends before it starts"]);
}
