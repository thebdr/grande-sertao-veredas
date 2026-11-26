//! A log's history read, as the session settles a flush's lines (contract: C-096, "a log whose
//! history a store error lost never says it read"). In the crate, since a flush that fails is the
//! store's own to provoke.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use super::{Stats, history_is_read, settle_lines, window_marks};

fn stats(pending: u64, stored: u64) -> Stats {
    Stats { count: pending + stored, last_arrival_ms: 0, last_ts: None, pending, stored, history_lost: false, pending_oldest_ms: None, oldest_stored_ms: None, said: false }
}

#[test]
fn a_failed_flush_loses_an_unread_history_for_good_and_never_a_read_one() {
    // A log whose worker sent its 100 lines of history: read once 100 are stored.
    let at = Arc::new(AtomicU64::new(100));
    let marks: HashMap<String, Arc<AtomicU64>> = HashMap::from([("log".to_string(), Arc::clone(&at)), ("old".to_string(), Arc::new(AtomicU64::new(10)))]);
    let mut all: BTreeMap<String, Stats> = BTreeMap::from([("log".to_string(), stats(60, 0)), ("old".to_string(), stats(5, 10)), ("plc".to_string(), stats(0, 0))]);
    // A flush that lands: its lines stored, the history not whole yet.
    settle_lines(&mut all, &marks, false);
    assert_eq!((all["log"].stored, all["log"].pending), (60, 0));
    assert!(!history_is_read(marks.get("log"), &all["log"]));
    // The next one fails while the history is unread: lost for good, whatever is stored after.
    all.get_mut("log").unwrap().pending = 40;
    all.get_mut("old").unwrap().pending = 3;
    settle_lines(&mut all, &marks, true);
    assert!(all["log"].history_lost);
    assert!(!history_is_read(marks.get("log"), &all["log"]), "100 counted, part of them lost");
    // A log whose history was read before the failure keeps it read: what is lost is new.
    assert!(!all["old"].history_lost && history_is_read(marks.get("old"), &all["old"]));
    // A source with no history to read reads as read; a log not read in yet does not.
    assert!(history_is_read(None, &all["plc"]));
    assert!(!history_is_read(Some(&Arc::new(AtomicU64::new(u64::MAX))), &stats(0, 500)));
}

/// A window pass lets go of a history only once its flag is published (C-096's eighth refute
/// round): a log whose lines are all stored, counted read by the pump but not yet said in the
/// health, loses its history to a pass that reaches its oldest row, and never says it read; one
/// already said keeps it, the window letting go of its rows like any others; one whose oldest row
/// is within the window loses nothing.
#[test]
fn a_window_lets_go_of_a_history_only_once_it_is_said() {
    let at = Arc::new(AtomicU64::new(100));
    let marks: HashMap<String, Arc<AtomicU64>> = ["counted", "said", "young"].iter().map(|id| (id.to_string(), Arc::clone(&at))).collect();
    let mut all: BTreeMap<String, Stats> = ["counted", "said", "young"].iter().map(|id| (id.to_string(), stats(100, 0))).collect();
    for st in all.values_mut() {
        st.pending_oldest_ms = Some(1_000);
    }
    all.get_mut("young").unwrap().pending_oldest_ms = Some(5_000);
    settle_lines(&mut all, &marks, false);
    assert!(all.iter().all(|(id, st)| history_is_read(marks.get(id), st)), "every history counted read");
    all.get_mut("said").unwrap().said = true;
    window_marks(&mut all, 2_000);
    assert!(!history_is_read(marks.get("counted"), &all["counted"]), "counted, not said: lost to the pass");
    assert!(history_is_read(marks.get("said"), &all["said"]), "said: kept");
    assert!(history_is_read(marks.get("young"), &all["young"]), "nothing of it let go");
}

/// The window compares a log's oldest stored arrival, not its first (C-096's ninth refute round):
/// arrivals are read from the wall clock, which may step back, so a later flush may hold rows older
/// than the first one's; a pass whose cutoff is past them loses the history unsaid.
#[test]
fn a_window_reads_a_log_s_oldest_arrival_whatever_the_order() {
    let at = Arc::new(AtomicU64::new(200));
    let marks: HashMap<String, Arc<AtomicU64>> = HashMap::from([("log".to_string(), Arc::clone(&at))]);
    let mut all: BTreeMap<String, Stats> = BTreeMap::from([("log".to_string(), stats(100, 0))]);
    all.get_mut("log").unwrap().pending_oldest_ms = Some(1_000);
    settle_lines(&mut all, &marks, false);
    // The clock stepped back: the next flush's lines arrived before the first's.
    let st = all.get_mut("log").unwrap();
    st.pending = 100;
    st.pending_oldest_ms = Some(500);
    settle_lines(&mut all, &marks, false);
    assert!(history_is_read(marks.get("log"), &all["log"]), "counted read, not said");
    window_marks(&mut all, 800);
    assert!(!history_is_read(marks.get("log"), &all["log"]), "its rows of 500 let go: lost");
}
