//! The telegram channel (contract: the channel clause), from Veredas 1's `veredas-telegram`
//! tests: two channels, or a channel and a raw socket, over real loopback TCP, configured by a
//! channel file (the segmented envelope layout, with sequence and keep-alive rules).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use veredas_core::telegram::channel::*;
use veredas_core::telegram::framing::{Frame, Framing, Segment};
use veredas_core::telegram::schema::{ChannelSpec, load};

/// The channel file every test runs with; `idle_ms` 0 never probes.
fn spec(idle_ms: u64) -> ChannelSpec {
    load(&format!(
        r#"
format = "veredas-channel"
version = 1

[framing]
start = "FD"
end = "FE"
envelope = [
  {{ name = "class", size = 1 }},
  {{ name = "source", size = 1, value = 1 }},
  {{ name = "destination", size = 1, value = 2 }},
  {{ name = "sequence", size = 1, role = "sequence" }},
  {{ name = "spare", size = 3 }},
  {{ name = "length", size = 2, role = "length" }},
  {{ name = "count", size = 2, role = "count" }},
]
segment = [ {{ name = "id", size = 2 }}, {{ name = "length", size = 2, role = "length" }} ]
tail = [ {{ name = "tail_spare", size = 2 }}, {{ name = "check", size = 1 }} ]

[sequence]
connect = 0
first = 1
last = 255

[keepalive]
on_connect = true
idle_ms = {idle_ms}
"#
    ))
    .unwrap()
}

/// The layout alone, for raw sockets.
fn layout() -> Framing {
    spec(0).framing
}

/// A frame as a raw peer would send it, with its sequence number.
fn raw_frame(prg: u8, body: &[u8]) -> Vec<u8> {
    let segments = if body.is_empty() { vec![] } else { vec![segment(7, body)] };
    let mut f = Frame { fields: BTreeMap::new(), segments };
    f.fields.insert("source".into(), 9);
    f.fields.insert("destination".into(), 1);
    layout().encode(&f, Some(u64::from(prg))).unwrap()
}

fn free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    port
}

fn listener(port: u16, keepalive_idle_ms: u64) -> Channel {
    Channel::start(ChannelConfig::new(
        "listener",
        Role::Listen { bind: "127.0.0.1".into(), port },
        spec(keepalive_idle_ms),
    ))
}

fn dialer(port: u16, keepalive_idle_ms: u64) -> Channel {
    let mut spec = spec(keepalive_idle_ms);
    for f in spec.framing.envelope.iter_mut() {
        match f.name.as_str() {
            "source" => f.value = 5,
            "destination" => f.value = 6,
            _ => {}
        }
    }
    Channel::start(ChannelConfig::new("dialer", Role::Connect { host: "127.0.0.1".into(), port }, spec))
}

fn until(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

/// Wait for a connection and return the peer id the channel gave it.
fn wait_connected(ch: &Channel) -> PeerId {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        for event in ch.poll() {
            if let ChannelEvent::Connected { peer, .. } = event {
                return peer;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no connection");
}

fn segment(id: u64, body: &[u8]) -> Segment {
    Segment { fields: BTreeMap::from([("id".to_string(), id)]), body: body.to_vec() }
}

fn payload(msg_id: u64, body: &[u8]) -> Frame {
    Frame { fields: BTreeMap::new(), segments: vec![segment(msg_id, body)] }
}

/// The sequence number a received frame carried.
fn seq(f: &Frame) -> u8 {
    f.fields["sequence"] as u8
}

/// Collect received frames for a while.
fn collect(ch: &Channel, timeout: Duration) -> Vec<Frame> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    while Instant::now() < deadline {
        for event in ch.poll() {
            if let ChannelEvent::Received { frame, .. } = event {
                out.push(frame);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    out
}

#[test]
fn two_channels_exchange_telegrams_in_both_directions() {
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });
    let client = dialer(port, 0);

    let server_peer = wait_connected(&server);
    let client_peer = wait_connected(&client);

    server.send(server_peer, payload(21, b"SCANNER"));
    client.send(client_peer, payload(22, b"DESTINATION"));

    let at_client = collect(&client, Duration::from_secs(3));
    let at_server = collect(&server, Duration::from_secs(1));
    assert_eq!(at_client.len(), 1, "{at_client:?}");
    assert_eq!(at_client[0].segments[0].fields["id"], 21);
    assert_eq!(at_client[0].segments[0].body, b"SCANNER");
    assert_eq!(at_server.len(), 1, "{at_server:?}");
    assert_eq!(at_server[0].segments[0].fields["id"], 22);

    // The envelope carries the sender's configured node ids.
    assert_eq!(at_server[0].fields["source"], 5);
    assert_eq!(at_server[0].fields["destination"], 6);
}

#[test]
fn the_progressive_number_starts_at_zero_then_counts_from_one() {
    // Sequence 0 on the connect keep-alive; 1..255 after.
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });
    let client = dialer(port, 0);
    let client_peer = wait_connected(&client);
    let _ = wait_connected(&server);

    for i in 0..4 {
        client.send(client_peer, payload(1, format!("{i}").as_bytes()));
    }
    let frames = collect(&server, Duration::from_secs(3));
    assert_eq!(frames.len(), 4, "{frames:?}");
    let prgs: Vec<u8> = frames.iter().map(seq).collect();
    assert_eq!(prgs, vec![1, 2, 3, 4], "stamped at send, monotonic");
    // The connect keep-alive carried 0 and was counted as such.
    assert!(server.status().keepalives_received >= 1);
}

#[test]
fn a_repeated_progressive_number_is_skipped() {
    // The receiver checks the sequence number to detect a repeated
    // frame and skip it.
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });

    // A raw client, so the PRG can be controlled exactly.
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let _ = wait_connected(&server);
    use std::io::Write;
    let frame = raw_frame;
    sock.write_all(&frame(1, b"first")).unwrap();
    sock.write_all(&frame(1, b"repeat")).unwrap();
    sock.write_all(&frame(2, b"second")).unwrap();

    let frames = collect(&server, Duration::from_secs(3));
    let bodies: Vec<Vec<u8>> = frames.iter().map(|f| f.segments[0].body.clone()).collect();
    assert_eq!(
        bodies,
        vec![b"first".to_vec(), b"second".to_vec()],
        "the duplicate PRG was skipped"
    );
    assert_eq!(server.status().duplicates_skipped, 1);
}


#[test]
fn a_reply_for_a_departed_peer_is_dropped_not_misdelivered() {
    // The index-vs-id bug: with ids, a reply queued for a peer that
    // has gone cannot reach whoever connected after it.
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });

    let first = std::net::TcpStream::connect(("127.0.0.1", port)).expect("first");
    let first_peer = wait_connected(&server);
    drop(first);
    until("the disconnect", Duration::from_secs(5), || {
        server.status().peers == 0
    });

    let second = std::net::TcpStream::connect(("127.0.0.1", port)).expect("second");
    let second_peer = wait_connected(&server);
    assert_ne!(first_peer, second_peer, "ids are never reused");

    // Sending to the departed peer must not reach the new one.
    server.send(first_peer, payload(22, b"STALE"));
    std::thread::sleep(Duration::from_millis(300));
    second
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let mut buf = [0u8; 64];
    use std::io::Read;
    let mut received = Vec::new();
    while let Ok(n) = (&second).read(&mut buf) {
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
    }
    // Only the connect keep-alive (envelope+tail) may have arrived.
    assert_eq!(
        received.len(),
        16,
        "the stale reply must not reach the new peer: {received:?}"
    );
    assert!(
        server
            .status()
            .last_error
            .unwrap_or_default()
            .contains("is gone")
    );
}

#[test]
fn a_peer_that_stops_reading_is_dropped_without_wedging_the_channel() {
    // The wedge bug: a full send buffer used to block the select arm
    // forever — no other peer served, no keep-alives, and shutdown
    // itself hung.
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });

    // A peer that connects and never reads a byte.
    let deaf = std::net::TcpStream::connect(("127.0.0.1", port)).expect("deaf peer");
    let deaf_peer = wait_connected(&server);

    // Feed it until its socket stops taking bytes and the write timeout judges it: however
    // large the socket buffers (Linux grows loopback ones to megabytes), the peer goes.
    let big = payload(23, &[b'x'; 480]);
    let started = Instant::now();
    while server.status().dropped_peers == 0 {
        assert!(started.elapsed() < Duration::from_secs(60), "the deaf peer was never dropped");
        for _ in 0..50 {
            server.send(deaf_peer, big.clone());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(server.status().peers, 0);

    // And the channel is still alive: a new peer works normally.
    let fresh = dialer(port, 0);
    let fresh_peer = wait_connected(&fresh);
    let server_peer = wait_connected(&server);
    server.send(server_peer, payload(24, b"ALIVE"));
    let frames = collect(&fresh, Duration::from_secs(3));
    assert!(
        frames.iter().any(|f| f.segments[0].body == b"ALIVE"),
        "the channel survived the deaf peer: {frames:?}"
    );
    drop(deaf);
    let _ = fresh_peer;

    // Stopping must not hang either.
    let t0 = Instant::now();
    server.stop();
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
}

#[test]
fn a_dialer_retries_until_the_listener_appears() {
    let port = free_port();
    let client = dialer(port, 0);
    until("a failed attempt", Duration::from_secs(10), || {
        client.status().last_error.is_some()
    });

    let server = listener(port, 0);
    until("the late connection", Duration::from_secs(30), || {
        server.status().peers == 1
    });
    let server_peer = wait_connected(&server);
    server.send(server_peer, payload(21, b"LATE"));
    let frames = collect(&client, Duration::from_secs(5));
    assert!(frames.iter().any(|f| f.segments[0].body == b"LATE"));
}

#[test]
fn garbage_between_frames_is_resynced_and_counted() {
    // A climbing resync count is how a wrong endian setting shows
    // itself, so it has to be visible rather than silent.
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let _ = wait_connected(&server);

    use std::io::Write;
    let good = raw_frame(4, b"REAL");
    sock.write_all(&[0x11, 0x22, 0x33]).unwrap();
    sock.write_all(&good).unwrap();

    let frames = collect(&server, Duration::from_secs(3));
    assert_eq!(frames.len(), 1, "the real frame was recovered");
    assert_eq!(frames[0].segments[0].body, b"REAL");
    assert!(server.status().resyncs >= 1);
}

#[test]
fn a_bind_failure_is_reported_and_the_channel_stops_cleanly() {
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").expect("squat");
    let port = squatter.local_addr().unwrap().port();
    let ch = listener(port, 0);
    until("the failure", Duration::from_secs(5), || {
        ch.status().last_error.is_some()
    });
    assert_eq!(ch.status().state, "failed");
    assert!(ch.status().last_error.unwrap().contains(&port.to_string()));
    let t0 = Instant::now();
    ch.stop();
    assert!(t0.elapsed() < Duration::from_secs(3));
}

// ---------------------------------------------------------------------------
// Refute round 1 (C-012): a reading peer is never mistaken for a deaf one, the PRG rules
// pinned exactly, the idle probe bounded.
// ---------------------------------------------------------------------------

/// The PRG of every frame read off a raw socket for `wait`, keep-alives included, in order.
fn raw_prgs(sock: &mut std::net::TcpStream, wait: Duration) -> Vec<(u8, bool)> {
    use std::io::Read;
    sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let deadline = Instant::now() + wait;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while Instant::now() < deadline {
        match sock.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    while let Ok((t, used)) = layout().decode(&buf[at..]) {
        out.push((seq(&t), t.is_keepalive()));
        at += used;
    }
    assert_eq!(at, buf.len(), "every byte read was a whole frame");
    out
}


#[test]
fn the_connect_keepalive_carries_prg_zero_in_both_roles() {
    // The listener, met by a raw client.
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let mut raw = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    assert_eq!(raw_prgs(&mut raw, Duration::from_millis(500)), [(0, true)]);

    // The dialer, met by a raw listener.
    let raw_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = raw_listener.local_addr().unwrap().port();
    let client = dialer(port, 600_000);
    let (mut raw, _) = raw_listener.accept().unwrap();
    assert_eq!(raw_prgs(&mut raw, Duration::from_millis(500)), [(0, true)]);
    drop(client);
}

#[test]
fn the_progressive_number_wraps_from_255_to_1() {
    let raw_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = raw_listener.local_addr().unwrap().port();
    let client = dialer(port, 600_000);
    let (mut raw, _) = raw_listener.accept().unwrap();
    let peer = wait_connected(&client);
    for i in 0..300u32 {
        client.send(peer, payload(1, &i.to_le_bytes()));
    }
    let prgs: Vec<u8> = raw_prgs(&mut raw, Duration::from_secs(3)).into_iter().map(|(p, _)| p).collect();
    let expected: Vec<u8> = std::iter::once(0).chain(1..=255).chain(1..=45).collect();
    assert_eq!(prgs, expected);
}

#[test]
fn a_keepalive_between_two_frames_makes_a_repeated_prg_new() {
    // The receiver's last PRG is the last frame's, keep-alives included: a sender whose idle
    // probes take PRGs from its counter can come back round to the PRG of its last data frame.
    // The connect keep-alive's 0 is not a PRG of the sequence.
    use std::io::Write;
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let _ = wait_connected(&server);
    for frame in [
        raw_frame(5, b"A"),
        raw_frame(6, b""),  // a probe
        raw_frame(5, b"B"), // the counter came back round: new
        raw_frame(5, b"B"), // a true repeat
        raw_frame(0, b""),  // a connect keep-alive
        raw_frame(5, b"C"), // still a repeat of B
    ] {
        sock.write_all(&frame).unwrap();
    }
    let bodies: Vec<Vec<u8>> =
        collect(&server, Duration::from_secs(2)).iter().map(|t| t.segments[0].body.clone()).collect();
    assert_eq!(bodies, [b"A".to_vec(), b"B".to_vec()]);
    assert_eq!(server.status().duplicates_skipped, 2);
}


#[test]
fn a_reading_peer_takes_a_burst_past_the_outbox_limit() {
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let client = dialer(port, 600_000);
    let _ = wait_connected(&client);
    let peer = wait_connected(&server);
    let n = 300usize;
    for i in 0..n as u32 {
        server.send(peer, payload(21, &i.to_le_bytes()));
    }
    let got = collect(&client, Duration::from_secs(5));
    let order: Vec<u32> =
        got.iter().map(|t| u32::from_le_bytes(t.segments[0].body[..4].try_into().unwrap())).collect();
    assert_eq!(order, (0..n as u32).collect::<Vec<_>>(), "every frame, in order");
    assert_eq!(server.status().dropped_peers, 0);
}

#[test]
fn a_steady_stream_neither_drops_a_reading_peer_nor_stops_the_channel_reading() {
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let client = dialer(port, 600_000);
    let client_peer = wait_connected(&client);
    let server_peer = wait_connected(&server);

    let started = Instant::now();
    let mut sent = 0u32;
    let mut ping_at = None;
    let mut ping_seen = None;
    let mut received = Vec::new();
    while started.elapsed() < Duration::from_secs(3) {
        // A frame every 10 ms, busy-paced so no tick slips in between by luck.
        let due = started + Duration::from_millis(10 * u64::from(sent));
        while Instant::now() < due {}
        server.send(server_peer, payload(21, &sent.to_le_bytes()));
        sent += 1;
        if ping_at.is_none() && started.elapsed() >= Duration::from_millis(500) {
            client.send(client_peer, payload(22, b"PING"));
            ping_at = Some(Instant::now());
        }
        for e in server.poll() {
            if matches!(e, ChannelEvent::Received { .. }) && ping_seen.is_none() {
                ping_seen = Some(Instant::now());
            }
        }
        received.extend(client.poll().into_iter().filter_map(|e| match e {
            ChannelEvent::Received { frame: telegram, .. } => Some(telegram),
            _ => None,
        }));
    }
    received.extend(collect(&client, Duration::from_secs(2)));
    assert_eq!(server.status().dropped_peers, 0, "{:?}", server.status().last_error);
    assert_eq!(received.len() as u32, sent, "every frame delivered");
    let lag = ping_seen.expect("the peer's frame was read") - ping_at.unwrap();
    assert!(lag < Duration::from_millis(300), "read {lag:?} after it was sent");
}

#[test]
fn a_deaf_peer_is_dropped_by_the_write_timeout() {
    // Paced below the outbox limit, so it is the write timeout that judges the peer, not the
    // outbox overflowing.
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let deaf = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let peer = wait_connected(&server);
    let big = payload(23, &[b'x'; 480]);
    let started = Instant::now();
    let mut why = None;
    'outer: while started.elapsed() < Duration::from_secs(60) {
        server.send(peer, big.clone());
        std::thread::sleep(Duration::from_millis(5));
        for e in server.poll() {
            if let ChannelEvent::Disconnected { peer: gone, why: w } = e {
                assert_eq!(gone, peer);
                why = Some(w);
                break 'outer;
            }
        }
    }
    assert_eq!(why.as_deref(), Some("write timed out — the peer is not reading"));
    drop(deaf);
}

#[test]
fn an_idle_link_is_probed_once_per_window_and_never_while_the_peer_talks() {
    // The listener never probes (its window is long); the dialer's is 300 ms.
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let client = dialer(port, 300);
    let _ = wait_connected(&client);
    let server_peer = wait_connected(&server);
    let connected = Instant::now();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(client.status().keepalives_sent, 0, "no probe before the window");
    std::thread::sleep(Duration::from_millis(1_750));
    // About one probe per 300 ms window (plus up to a 20 ms tick) over 2 s.
    let probes = client.status().keepalives_sent;
    assert!((4..=7).contains(&probes), "{probes} probes in {:?}", connected.elapsed());
    // Each is a liveness event at the other end (plus the connect keep-alive).
    until("the probes to arrive", Duration::from_secs(2), || {
        server.status().keepalives_received > probes
    });
    let keepalive_events = server.poll().iter().filter(|e| matches!(e, ChannelEvent::KeepAlive { .. })).count();
    // Every probe is an event (the connect keep-alive's may have been drained by `wait_connected`).
    assert!(keepalive_events as u64 >= probes, "{keepalive_events} events for {probes} probes");

    // While the listener talks every 100 ms, the dialer never probes.
    let before = client.status().keepalives_sent;
    for i in 0..15u8 {
        server.send(server_peer, payload(21, &[i]));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(client.status().keepalives_sent, before);
}

// ---------------------------------------------------------------------------
// Refute round 2 (C-012): several peers at once, a deaf peer beside a healthy one, a backlog
// at shutdown.
// ---------------------------------------------------------------------------

/// A raw client that reads and decodes everything for `wait`, returning the bodies received.
fn raw_reader(port: u16, wait: Duration) -> std::thread::JoinHandle<Vec<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        raw_prgs_and_bodies(&mut sock, wait).into_iter().filter_map(|(_, body)| body).collect()
    })
}

/// Every frame read off a raw socket for `wait`: its sequence number and its first segment.
fn raw_prgs_and_bodies(sock: &mut std::net::TcpStream, wait: Duration) -> Vec<(u8, Option<Vec<u8>>)> {
    use std::io::Read;
    sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let deadline = Instant::now() + wait;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    while Instant::now() < deadline {
        if let Ok(n) = sock.read(&mut chunk) {
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    while let Ok((t, used)) = layout().decode(&buf[at..]) {
        out.push((seq(&t), t.segments.first().map(|s| s.body.clone())));
        at += used;
    }
    out
}

#[test]
fn a_burst_reaches_a_reading_peer_quickly_with_other_peers_connected() {
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    // Three other readers connected beside the one the burst is for.
    let others: Vec<_> = (0..3).map(|_| raw_reader(port, Duration::from_secs(3))).collect();
    until("three peers", Duration::from_secs(5), || server.status().peers == 3);
    let client = dialer(port, 600_000);
    let _ = wait_connected(&client);
    until("four peers", Duration::from_secs(5), || server.status().peers == 4);
    let peer = loop {
        // The newest peer is the dialler.
        let ids: Vec<PeerId> = server
            .poll()
            .into_iter()
            .filter_map(|e| match e {
                ChannelEvent::Connected { peer, .. } => Some(peer),
                _ => None,
            })
            .collect();
        if let Some(id) = ids.into_iter().max() {
            break id;
        }
    };
    let started = Instant::now();
    for i in 0..300u32 {
        server.send(peer, payload(1, &i.to_le_bytes()));
    }
    let mut got = Vec::new();
    while got.len() < 300 && started.elapsed() < Duration::from_secs(5) {
        got.extend(collect(&client, Duration::from_millis(50)));
    }
    let took = started.elapsed();
    assert_eq!(got.len(), 300, "after {took:?}");
    assert!(took < Duration::from_millis(1_500), "300 frames took {took:?} with 4 peers");
    assert_eq!(server.status().dropped_peers, 0);
    for o in others {
        let _ = o.join();
    }
}

#[test]
fn a_deaf_peer_stalls_no_other_peer_while_it_is_timed_out() {
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let deaf = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let deaf_peer = wait_connected(&server);
    let client = dialer(port, 600_000);
    let client_peer = wait_connected(&client);
    let _ = wait_connected(&server);

    // The healthy dialler talks every 50 ms while the deaf peer is fed until it is dropped;
    // the longest silence between two of its frames at the server is the stall.
    let started = Instant::now();
    let mut last_seen: Option<Instant> = None;
    let mut worst = Duration::ZERO;
    let mut i = 0u32;
    while server.status().dropped_peers == 0 {
        assert!(started.elapsed() < Duration::from_secs(60), "the deaf peer was never dropped");
        for _ in 0..20 {
            server.send(deaf_peer, payload(2, &[b'x'; 480]));
        }
        client.send(client_peer, payload(3, &i.to_le_bytes()));
        i += 1;
        std::thread::sleep(Duration::from_millis(50));
        for e in server.poll() {
            if let ChannelEvent::Received { .. } = e {
                let now = Instant::now();
                if let Some(prev) = last_seen {
                    worst = worst.max(now - prev);
                }
                last_seen = Some(now);
            }
        }
    }
    assert!(i > 20, "the write timeout ran {:?}", started.elapsed());
    assert!(worst < Duration::from_millis(400), "the healthy peer went unread for {worst:?}");
    drop(deaf);
}

#[test]
fn stop_returns_promptly_behind_a_backlog() {
    let port = free_port();
    let server = listener(port, 600_000);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let deaf: Vec<_> = (0..4).map(|_| std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
    until("four peers", Duration::from_secs(5), || server.status().peers == 4);
    for _ in 0..450 {
        server.send_all(payload(2, &[b'x'; 480]));
    }
    let started = Instant::now();
    server.stop();
    assert!(started.elapsed() < Duration::from_secs(2), "stop took {:?}", started.elapsed());
    drop(deaf);
}

#[test]
fn a_frame_after_a_quiet_spell_of_254_probes_is_not_skipped() {
    // Two channels of this code. The dialler probes every tick; B is sent when the listener
    // has counted 254 probes since A, aiming B at A's own sequence number. The aim races the
    // next probe, so the attempt repeats until B carried A's number (the raw-socket test above
    // pins the rule itself deterministically).
    for attempt in 0..4 {
        let port = free_port();
        let server = listener(port, 600_000);
        until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
        let client = dialer(port, 1);
        let peer = wait_connected(&client);
        let _ = wait_connected(&server);
        client.send(peer, payload(1, b"A"));
        let mut prg_a = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while prg_a.is_none() && Instant::now() < deadline {
            for e in server.poll() {
                if let ChannelEvent::Received { frame: telegram, .. } = e {
                    prg_a = Some(telegram.fields["sequence"]);

                }
            }
        }
        let prg_a = prg_a.expect("A arrived");
        // The dialler's numbers are consumed by A and by its probes, one each, from 1: A took
        // the number after its first (pA - 1 mod 255) probes, so B takes pA again once probes
        // reach that count + 254 (mod 255). Watch its own probe counter and send B the moment
        // it gets there; the next probe is a tick away.
        let target = (prg_a + 253) % 255;
        let since = client.status().keepalives_sent;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let sent = client.status().keepalives_sent;
            if sent > since + 2 && sent % 255 == target {
                break;
            }
            assert!(Instant::now() < deadline, "the dialler stopped probing");
        }
        client.send(peer, payload(1, b"B"));
        let mut prg_b = None;
        let deadline = Instant::now() + Duration::from_secs(3);
        while prg_b.is_none() && Instant::now() < deadline {
            for e in server.poll() {
                if let ChannelEvent::Received { frame: telegram, .. } = e {
                    assert_eq!(telegram.segments[0].body, b"B");
                    prg_b = Some(telegram.fields["sequence"]);
                }
            }
        }
        let prg_b = prg_b.expect("B was received, not skipped");
        assert_eq!(server.status().duplicates_skipped, 0);
        if prg_b == prg_a {
            return;
        }
        eprintln!("attempt {attempt}: B carried {prg_b}, A {prg_a}; aiming again");
    }
    panic!("B never carried A's sequence number in four attempts");
}

#[test]
fn a_slow_but_reading_peer_is_never_judged_deaf() {
    // About 100 KB/s against an 8 MB burst: the socket keeps taking bytes as the peer reads, so
    // the write timeout must not fire, however the OS signals writability (Linux signals only
    // once a third of the send buffer is free). The burst is far more than the reader and the
    // socket buffers together can take in the time, so the outbox still holds most of it when
    // the peer is judged (refute round 4: a 2 MB burst could sit wholly in the buffers).
    use std::io::Read;
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let reader = std::thread::spawn(move || {
        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let mut total = 0usize;
        let mut chunk = [0u8; 1024];
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(6) {
            if let Ok(n) = sock.read(&mut chunk) {
                total += n;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        total
    });
    let peer = wait_connected(&server);
    for _ in 0..1_600 {
        server.send(peer, payload(23, &[b'x'; 5_000]));
    }
    // Judged while the reader still reads (it reads for 6 s; the write timeout is 2 s): when it
    // finishes it closes its socket, and a closed peer is rightly dropped.
    std::thread::sleep(Duration::from_millis(5_000));
    let status = server.status();
    assert_eq!(status.dropped_peers, 0, "{:?}", status.last_error);
    assert!(status.sent < 1_600, "the burst was still queued when judged: {} sent", status.sent);
    let read = reader.join().unwrap();
    assert!(read > 300_000, "the reader kept reading: {read} bytes");
}

// ---------------------------------------------------------------------------
// Refute round 4 (C-012): probes under `keepalive_counts = false`, a segmentless layout's
// keep-alive, and a slow reader judged while the outbox still holds most of its burst.
// ---------------------------------------------------------------------------

/// The test file's rules with keep-alives that take no sequence number.
fn uncounted(idle_ms: u64) -> ChannelSpec {
    let mut s = spec(idle_ms);
    s.sequence.as_mut().unwrap().keepalive_counts = false;
    s
}

#[test]
fn uncounted_probes_carry_the_last_number_and_take_none() {
    let raw_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = raw_listener.local_addr().unwrap().port();
    let client = Channel::start(ChannelConfig::new(
        "dialer",
        Role::Connect { host: "127.0.0.1".into(), port },
        uncounted(1),
    ));
    let (mut raw, _) = raw_listener.accept().unwrap();
    let peer = wait_connected(&client);
    // Probes before any data, then A, probes, B, probes: every tick of silence sends one.
    let probes = |n: u64| {
        let since = client.status().keepalives_sent;
        until("probes", Duration::from_secs(5), || client.status().keepalives_sent >= since + n);
    };
    probes(3);
    client.send(peer, payload(1, b"A"));
    probes(3);
    client.send(peer, payload(1, b"B"));
    probes(3);
    let frames = raw_prgs(&mut raw, Duration::from_millis(500));
    // The connect keep-alive's 0, probes carrying 0, A on 1, probes carrying 1, B on 2, probes
    // carrying 2: the data frames number 1, 2 as if no probe had been sent.
    let data: Vec<u8> = frames.iter().filter(|(_, ka)| !ka).map(|(p, _)| *p).collect();
    assert_eq!(data, [1, 2], "{frames:?}");
    let mut last = 0;
    for (p, ka) in &frames {
        if *ka {
            assert_eq!(*p, last, "a probe carries the last number stamped: {frames:?}");
        } else {
            last = *p;
        }
    }
    let probes_seen = frames.iter().filter(|(_, ka)| *ka).count();
    assert!(probes_seen >= 10, "the connect keep-alive and nine probes at least: {frames:?}");
}

#[test]
fn uncounted_probes_never_bring_a_data_frame_back_round_to_the_last() {
    // Two channels of this code, both uncounted: A, then more probes than a whole cycle, then B.
    // Had the probes taken numbers, B could land on A's and be skipped; they take none, so B
    // is A's number plus one and arrives.
    let port = free_port();
    let server = Channel::start(ChannelConfig::new(
        "listener",
        Role::Listen { bind: "127.0.0.1".into(), port },
        uncounted(600_000),
    ));
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let client = Channel::start(ChannelConfig::new(
        "dialer",
        Role::Connect { host: "127.0.0.1".into(), port },
        uncounted(1),
    ));
    let peer = wait_connected(&client);
    let _ = wait_connected(&server);
    client.send(peer, payload(1, b"A"));
    let since = client.status().keepalives_sent;
    until("a cycle of probes", Duration::from_secs(20), || client.status().keepalives_sent >= since + 256);
    client.send(peer, payload(1, b"B"));
    let frames = collect(&server, Duration::from_secs(1));
    let got: Vec<(Vec<u8>, u8)> = frames.iter().map(|f| (f.segments[0].body.clone(), seq(f))).collect();
    assert_eq!(got, [(b"A".to_vec(), 1), (b"B".to_vec(), 2)]);
    assert_eq!(server.status().duplicates_skipped, 0);
    assert!(server.status().keepalives_received >= 256);
}

#[test]
fn a_segmentless_empty_body_arrives_as_a_keepalive_with_its_fields() {
    use std::io::Write;
    // A bare layout: a kind byte, a two-byte length, the body; no sequence.
    let bare = load(
        r#"
format = "veredas-channel"
version = 1
[framing]
envelope = [ { name = "kind", size = 1 }, { name = "length", size = 2, role = "length" } ]
"#,
    )
    .unwrap();
    let port = free_port();
    let server = Channel::start(ChannelConfig::new("bare", Role::Listen { bind: "127.0.0.1".into(), port }, bare));
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    sock.write_all(&[0x07, 0, 0, 0x08, 2, 0, b'h', b'i']).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = Vec::new();
    while seen.len() < 2 && Instant::now() < deadline {
        for e in server.poll() {
            match e {
                ChannelEvent::KeepAlive { frame, .. } => {
                    seen.push(("keepalive", frame.fields["kind"], frame.segments.len()))
                }
                ChannelEvent::Received { frame, .. } => {
                    assert_eq!(frame.segments[0].body, b"hi");
                    seen.push(("received", frame.fields["kind"], frame.segments.len()))
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen, [("keepalive", 7, 0), ("received", 8, 1)]);
    assert_eq!((server.status().keepalives_received, server.status().received), (1, 1));
}

// ---------------------------------------------------------------------------
// Refute round 5 (C-012): a keep-alive the owner sends is numbered by the same rule as a probe,
// so under `keepalive_counts = false` no run of them brings a data frame back round.
// ---------------------------------------------------------------------------

#[test]
fn keepalives_the_owner_sends_are_numbered_like_probes() {
    // A, 254 empty frames sent by the owner, B; no probes (idle 0). Uncounted, the empty frames
    // take no number and B follows A; counted, they take 2..=255, are counted as seen, and B
    // wraps to 1. Either way nothing is skipped.
    for (counts, b_number) in [(false, 2u8), (true, 1u8)] {
        let rules = |idle| {
            let mut s = spec(idle);
            s.sequence.as_mut().unwrap().keepalive_counts = counts;
            s
        };
        let port = free_port();
        let server = Channel::start(ChannelConfig::new(
            "listener",
            Role::Listen { bind: "127.0.0.1".into(), port },
            rules(0),
        ));
        until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
        let client =
            Channel::start(ChannelConfig::new("dialer", Role::Connect { host: "127.0.0.1".into(), port }, rules(0)));
        let peer = wait_connected(&client);
        let _ = wait_connected(&server);
        client.send(peer, payload(1, b"A"));
        for _ in 0..254 {
            client.send(peer, Frame::default());
        }
        client.send(peer, payload(1, b"B"));
        until("all of it", Duration::from_secs(10), || {
            let s = server.status();
            s.received + s.duplicates_skipped >= 2 && s.keepalives_received >= 255
        });
        let got: Vec<(Vec<u8>, u8)> =
            collect(&server, Duration::from_millis(200)).iter().map(|f| (f.segments[0].body.clone(), seq(f))).collect();
        assert_eq!(got, [(b"A".to_vec(), 1), (b"B".to_vec(), b_number)], "keepalive_counts = {counts}");
        assert_eq!(server.status().duplicates_skipped, 0, "keepalive_counts = {counts}");
    }
}

#[test]
fn empty_messages_of_a_segmentless_layout_never_cost_a_data_message() {
    // A kind byte, a sequence number, a length, the body; keep-alives do not count. 254 empty
    // acknowledgements (kind 6) between two data messages arrive as keep-alives carrying their
    // kind, take no number, and the second data message arrives on the number after the first.
    let bare = load(
        r#"
format = "veredas-channel"
version = 1
[framing]
envelope = [
  { name = "kind", size = 1 },
  { name = "sequence", size = 1, role = "sequence" },
  { name = "length", size = 2, role = "length" },
]
[sequence]
keepalive_counts = false
[keepalive]
idle_ms = 0
"#,
    )
    .unwrap();
    let message = |kind: u64, body: &[u8]| Frame {
        fields: BTreeMap::from([("kind".to_string(), kind)]),
        segments: vec![Segment { fields: BTreeMap::new(), body: body.to_vec() }],
    };
    let port = free_port();
    let server = Channel::start(ChannelConfig::new("bare", Role::Listen { bind: "127.0.0.1".into(), port }, bare.clone()));
    until("the bind", Duration::from_secs(5), || server.status().state.contains("listening"));
    let client = Channel::start(ChannelConfig::new("dialer", Role::Connect { host: "127.0.0.1".into(), port }, bare));
    let peer = wait_connected(&client);
    let _ = wait_connected(&server);
    client.send(peer, message(8, b"A"));
    for _ in 0..254 {
        client.send(peer, message(6, b""));
    }
    client.send(peer, message(8, b"B"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut data, mut acks) = (Vec::new(), 0);
    while (data.len() < 2 || acks < 254) && Instant::now() < deadline {
        for e in server.poll() {
            match e {
                ChannelEvent::KeepAlive { frame, .. } if frame.fields["kind"] == 6 => acks += 1,
                ChannelEvent::Received { frame, .. } => {
                    data.push((frame.fields["kind"], frame.segments[0].body.clone(), frame.fields["sequence"]))
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(data, [(8, b"A".to_vec(), 1), (8, b"B".to_vec(), 2)]);
    assert_eq!(acks, 254);
    assert_eq!(server.status().duplicates_skipped, 0);
}
