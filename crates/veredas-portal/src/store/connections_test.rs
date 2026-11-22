//! The pump's read of the store never takes a reader's connection (contract: the session clause,
//! "a pass never waits on a reader"): with the server's query connection and a shell's snapshot
//! connection both held, the memory figure the pump reads comes back at once, as unheld. In the
//! crate, since the connections are private; the session's own test holds the query connection
//! from outside.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{DuckStore, Residency};

#[test]
fn the_memory_figure_is_read_at_once_with_both_readers_connections_held() {
    let store = std::sync::Arc::new(DuckStore::open(&Residency::Ram, &[], &[]).unwrap());
    let unheld = store.approx_bytes();
    let _api = store.api.lock().unwrap();
    let _ui = store.ui.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::sync::Arc::clone(&store);
    std::thread::spawn(move || {
        // Five reads, each timed: a read that tried a held connection first, for however short a
        // while, is slow every time; a loaded machine slows one read now and then, not all five.
        let mut fastest = Duration::MAX;
        let mut figure = 0;
        for _ in 0..5 {
            let started = Instant::now();
            figure = reader.approx_bytes();
            fastest = fastest.min(started.elapsed());
        }
        let _ = tx.send((figure, fastest));
    });
    let (figure, fastest) = rx.recv_timeout(Duration::from_secs(10)).expect("the figure came back with both readers' connections held");
    assert_eq!(figure, unheld, "the same figure, read while the readers hold theirs");
    assert!(fastest < Duration::from_millis(50), "read at once, never after a wait on a reader's connection: the fastest of five took {fastest:?}");
}
