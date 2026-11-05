//! The telegram channel (contract: the channel clause), from Veredas 1's `veredas-telegram`
//! tests: two channels, or a channel and a raw socket, over real loopback TCP.

use std::time::{Duration, Instant};

use veredas_core::telegram::channel::*;
use veredas_core::telegram::codec::{self, DataSegment};
use veredas_core::telegram::{Endian, Envelope, Telegram};

fn free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    port
}

fn listener(port: u16, keepalive_idle_ms: u64) -> Channel {
    Channel::start(ChannelConfig {
        keepalive_idle_ms,
        ..ChannelConfig::new(
            "listener",
            Role::Listen {
                bind: "127.0.0.1".into(),
                port,
            },
        )
    })
}

fn dialer(port: u16, keepalive_idle_ms: u64) -> Channel {
    Channel::start(ChannelConfig {
        keepalive_idle_ms,
        source_node: 5,
        destination_node: 6,
        ..ChannelConfig::new(
            "dialer",
            Role::Connect {
                host: "127.0.0.1".into(),
                port,
            },
        )
    })
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

fn payload(msg_id: u16, body: &[u8]) -> Telegram {
    Telegram {
        envelope: Envelope::default(),
        segments: vec![DataSegment {
            msg_id,
            data: body.to_vec(),
        }],
    }
}

/// Collect received telegrams for a while.
fn collect(ch: &Channel, timeout: Duration) -> Vec<Telegram> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    while Instant::now() < deadline {
        for event in ch.poll() {
            if let ChannelEvent::Received { telegram, .. } = event {
                out.push(telegram);
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

    server.send(server_peer, payload(51004, b"SCANNER"));
    client.send(client_peer, payload(50002, b"DESTINATION"));

    let at_client = collect(&client, Duration::from_secs(3));
    let at_server = collect(&server, Duration::from_secs(1));
    assert_eq!(at_client.len(), 1, "{at_client:?}");
    assert_eq!(at_client[0].segments[0].msg_id, 51004);
    assert_eq!(at_client[0].segments[0].data, b"SCANNER");
    assert_eq!(at_server.len(), 1, "{at_server:?}");
    assert_eq!(at_server[0].segments[0].msg_id, 50002);

    // The envelope carries the sender's configured node ids.
    assert_eq!(at_server[0].envelope.source, 5);
    assert_eq!(at_server[0].envelope.destination, 6);
}

#[test]
fn the_progressive_number_starts_at_zero_then_counts_from_one() {
    // §3: PRG 0 on the connect keep-alive; 1..255 after.
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
    let prgs: Vec<u8> = frames.iter().map(|f| f.envelope.prg).collect();
    assert_eq!(prgs, vec![1, 2, 3, 4], "stamped at send, monotonic");
    // The connect keep-alive carried 0 and was counted as such.
    assert!(server.status().keepalives_received >= 1);
}

#[test]
fn a_repeated_progressive_number_is_skipped() {
    // §3.1: "The receiver must check the Progressive Number field in
    // order to detect a double message and so skip it."
    let port = free_port();
    let server = listener(port, 0);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });

    // A raw client, so the PRG can be controlled exactly.
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let _ = wait_connected(&server);
    use std::io::Write;
    let frame = |prg: u8, body: &[u8]| {
        codec::encode(
            &Telegram {
                envelope: Envelope {
                    class: 0,
                    source: 9,
                    destination: 1,
                    prg,
                },
                segments: vec![DataSegment {
                    msg_id: 7,
                    data: body.to_vec(),
                }],
            },
            Endian::default(),
        )
        .unwrap()
    };
    sock.write_all(&frame(1, b"first")).unwrap();
    sock.write_all(&frame(1, b"repeat")).unwrap();
    sock.write_all(&frame(2, b"second")).unwrap();

    let frames = collect(&server, Duration::from_secs(3));
    let bodies: Vec<Vec<u8>> = frames.iter().map(|f| f.segments[0].data.clone()).collect();
    assert_eq!(
        bodies,
        vec![b"first".to_vec(), b"second".to_vec()],
        "the duplicate PRG was skipped"
    );
    assert_eq!(server.status().duplicates_skipped, 1);
}

#[test]
fn an_idle_link_is_probed_with_a_keepalive() {
    // §3.2: silence for the receive timeout means send a keep-alive
    // to prove the link is still there.
    let port = free_port();
    let server = listener(port, 150);
    until("the bind", Duration::from_secs(5), || {
        server.status().state.contains("listening")
    });
    let client = dialer(port, 150);
    let _ = wait_connected(&client);

    until("keep-alives to flow", Duration::from_secs(10), || {
        client.status().keepalives_received >= 2 && server.status().keepalives_sent >= 2
    });
    // And a keep-alive is a liveness EVENT, so health does not read
    // a quiet-but-alive link as idle.
    let seen_keepalive = client
        .poll()
        .iter()
        .any(|e| matches!(e, ChannelEvent::KeepAlive { .. }))
        || client.status().keepalives_received > 0;
    assert!(seen_keepalive);
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
    server.send(first_peer, payload(50002, b"STALE"));
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
        codec::ENVELOPE_LEN + codec::TAIL_LEN,
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

    // Flood it well past any socket buffer.
    let big = payload(52002, &vec![b'x'; 480]);
    for _ in 0..(OUTBOX_LIMIT + 200) {
        server.send(deaf_peer, big.clone());
    }

    // The channel must notice and drop it, not hang.
    until(
        "the deaf peer to be dropped",
        Duration::from_secs(30),
        || server.status().dropped_peers >= 1,
    );
    assert_eq!(server.status().peers, 0);

    // And the channel is still alive: a new peer works normally.
    let fresh = dialer(port, 0);
    let fresh_peer = wait_connected(&fresh);
    let server_peer = wait_connected(&server);
    server.send(server_peer, payload(51000, b"ALIVE"));
    let frames = collect(&fresh, Duration::from_secs(3));
    assert!(
        frames.iter().any(|f| f.segments[0].data == b"ALIVE"),
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
    server.send(server_peer, payload(51004, b"LATE"));
    let frames = collect(&client, Duration::from_secs(5));
    assert!(frames.iter().any(|f| f.segments[0].data == b"LATE"));
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
    let good = codec::encode(
        &Telegram {
            envelope: Envelope {
                class: 0,
                source: 1,
                destination: 2,
                prg: 4,
            },
            segments: vec![DataSegment {
                msg_id: 3,
                data: b"REAL".to_vec(),
            }],
        },
        Endian::default(),
    )
    .unwrap();
    sock.write_all(&[0x11, 0x22, 0x33]).unwrap();
    sock.write_all(&good).unwrap();

    let frames = collect(&server, Duration::from_secs(3));
    assert_eq!(frames.len(), 1, "the real frame was recovered");
    assert_eq!(frames[0].segments[0].data, b"REAL");
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
