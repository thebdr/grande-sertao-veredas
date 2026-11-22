//! The pump's read of the store never takes a reader's connection (contract: the session clause,
//! "a pass never waits on a reader"): with the server's query connection and a shell's snapshot
//! connection both held, the memory figure the pump reads still comes back. In the crate, since
//! the connections are private; the session's own test holds the query connection from outside.

use std::sync::mpsc;
use std::time::Duration;

use super::{DuckStore, Residency};

#[test]
fn the_memory_figure_is_read_with_both_readers_connections_held() {
    let store = std::sync::Arc::new(DuckStore::open(&Residency::Ram, &[], &[]).unwrap());
    let unheld = store.approx_bytes();
    let _api = store.api.lock().unwrap();
    let _ui = store.ui.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::sync::Arc::clone(&store);
    std::thread::spawn(move || {
        let _ = tx.send(reader.approx_bytes());
    });
    let figure = rx.recv_timeout(Duration::from_secs(10)).expect("the figure came back with both readers' connections held");
    assert_eq!(figure, unheld, "the same figure, read while the readers hold theirs");
}
