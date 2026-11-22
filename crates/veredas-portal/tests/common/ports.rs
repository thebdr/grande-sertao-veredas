//! Ports a test holds, never picks and hopes for. A port read off a listener that was then closed
//! (bind to 0, take the number, drop the socket) is anybody's until the test binds it again: on a
//! machine where other tests bind and dial all the time, another socket took one between the two,
//! the bench's listener failed to bind while the bench reported a start, and the session waited
//! for a peer that was never there (the server test's first wait, about one run in five hundred
//! under load, `docs/portal-plan.md`, "Traps"). So a listener binds port 0 itself and the test
//! reads the port it holds, and a port nobody may answer on is held bound without listening.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use veredas_maquinar::bench::Bench;

/// The port in a channel's state text once it listens: `listening on 127.0.0.1:<port>`, the
/// text core's channel sets when its socket is bound.
pub fn listening_port(state: &str) -> Option<u16> {
    state.strip_prefix("listening on ")?.rsplit_once(':')?.1.parse().ok()
}

/// The port a bench channel listens on, waited for: the bench binds on its own thread after its
/// start returns, so a dial sent at once may find nothing yet. A bind that failed is named.
pub fn bench_listening_port(bench: &Bench, channel: &str) -> u16 {
    let started = Instant::now();
    loop {
        let wire = bench.channel(channel).unwrap_or_else(|| panic!("the bench has no channel `{channel}`")).status().wire;
        if let Some(port) = listening_port(&wire.state) {
            return port;
        }
        assert!(wire.state != "failed" && started.elapsed() < Duration::from_secs(10), "the bench's `{channel}` does not listen: {wire:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A port held bound on the loopback and not listening, for as long as this value lives: a dial
/// to it is refused at once, and no port the system hands out (a bind to 0, a dial's own end) is
/// this one. On Unix it carries `SO_REUSEADDR`, so a listener that binds this very port later
/// with the same option (tokio's listener sets it there) shares it, and nothing can take the port
/// between the test's look at it and that listener's bind.
pub struct HeldPort {
    socket: Option<tokio::net::TcpSocket>,
    pub port: u16,
}

pub fn held_port() -> HeldPort {
    let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
    #[cfg(unix)]
    socket.set_reuseaddr(true).expect("SO_REUSEADDR");
    socket.bind("127.0.0.1:0".parse().unwrap()).expect("a port on the loopback");
    let port = socket.local_addr().expect("its address").port();
    HeldPort { socket: Some(socket), port }
}

impl HeldPort {
    /// Ready for a listener of the test's own to bind this port: on Unix the hold stays (shared);
    /// on Windows, where a listener does not share, it is let go now, right before the bind.
    pub fn yield_to_listener(&mut self) {
        #[cfg(windows)]
        {
            self.socket = None;
        }
        #[cfg(not(windows))]
        let _ = &self.socket;
    }
}
