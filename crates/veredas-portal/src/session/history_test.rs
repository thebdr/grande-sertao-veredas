//! A log's history read, as the session settles a flush's lines (contract: C-096, "a log whose
//! history a store error lost never says it read"). In the crate, since a flush that fails is the
//! store's own to provoke.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use super::{Stats, history_is_read, settle_lines};

fn stats(pending: u64, stored: u64) -> Stats {
    Stats { count: pending + stored, last_arrival_ms: 0, last_ts: None, pending, stored, history_lost: false }
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
