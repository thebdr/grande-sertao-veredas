//! The decision path's HTTP client and its lifecycle queue (contract: the decision-client
//! clause), from Veredas 1's `veredas-telegram` tests. Every exchange runs over a real
//! loopback socket against a stand-in service that answers raw bytes, so what the client
//! reads is what a real service would send.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use veredas_core::telegram::decision_client::*;

fn event(id: u32) -> LifecycleEvent {
    LifecycleEvent {
        at_ms: 1_700_000_000_000 + id as i64,
        object_id: Some(id),
        direction: "msc->eu".into(),
        channel: "CHM".into(),
        kind: "scanner request".into(),
        msg_id: 51004,
        fields: Default::default(),
        raw_hex: LifecycleEvent::hex(&[0xFD, id as u8, 0xFE]),
    }
}

/// Read one HTTP request whole, as a real service does: the head up to its blank line, then
/// exactly the body its Content-Length announces (a client that lied about it would leave this
/// waiting, or cut the body short).
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..split]).to_string();
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("Content-Length: "))
                .map(|v| v.trim().parse().unwrap())
                .unwrap_or(0);
            if buf.len() >= split + 4 + length {
                assert_eq!(buf.len(), split + 4 + length, "nothing after the announced body");
                return String::from_utf8_lossy(&buf).to_string();
            }
        }
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0, "the client closed before its request was whole");
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// A one-shot service on an ephemeral port: reads the request, answers `reply` verbatim,
/// closes, and hands back what it was asked.
async fn serve_once(reply: &'static [u8]) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        socket.write_all(reply).await.unwrap();
        socket.shutdown().await.unwrap();
        request
    });
    (port, server)
}

fn client(port: u16) -> DecisionClient {
    DecisionClient::new(format!("http://127.0.0.1:{port}"), Duration::from_secs(5))
}

#[tokio::test]
async fn addresses_that_are_not_http_host_port_are_refused_before_dialling() {
    let refused = |url: &str| {
        let c = DecisionClient::new(url, Duration::from_secs(1));
        async move {
            c.decide(&DecideRequest::default())
                .await
                .unwrap_err()
                .to_string()
        }
    };
    assert_eq!(
        refused("127.0.0.1:8930").await,
        "`127.0.0.1:8930` is not an http:// address"
    );
    assert_eq!(
        refused("https://sorter:1").await,
        "`https://sorter:1` is not an http:// address"
    );
    assert_eq!(
        refused("http://host:notaport").await,
        "`notaport` is not a port number"
    );
    assert_eq!(
        refused("http://host:70000").await,
        "`70000` is not a port number"
    );
    // A trailing slash is trimmed; a path after the authority is ignored for the dial.
    assert_eq!(
        DecisionClient::new("http://a:1/", Duration::ZERO).base_url(),
        "http://a:1"
    );
}

#[tokio::test]
async fn a_path_after_the_port_still_dials_the_port() {
    let (port, server) = serve_once(b"HTTP/1.1 200 OK\r\n\r\n{\"destination\":5}").await;
    let c = DecisionClient::new(
        format!("http://127.0.0.1:{port}/ignored"),
        Duration::from_secs(5),
    );
    assert_eq!(
        c.decide(&DecideRequest::default())
            .await
            .unwrap()
            .destination,
        Some(5)
    );
    // The request line names the endpoint, not the base path.
    assert!(
        server
            .await
            .unwrap()
            .starts_with("POST /decide HTTP/1.1\r\n")
    );
}

#[tokio::test]
async fn a_success_yields_its_body_and_a_refusal_is_never_an_answer() {
    let (port, _s) =
        serve_once(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"destination\":7}")
            .await;
    let answer = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap();
    assert_eq!(
        answer,
        DecideResponse {
            destination: Some(7),
            ..Default::default()
        }
    );

    // 204 with no body: a success, but nothing to decode for /decide...
    let (port, _s) = serve_once(b"HTTP/1.1 204 No Content\r\n\r\n").await;
    let err = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Decode(_)), "{err:?}");
    // ...and exactly what posting events expects.
    let (port, _s) = serve_once(b"HTTP/1.1 204 No Content\r\n\r\n").await;
    client(port).post_events(&[event(1)]).await.unwrap();

    let (port, _s) = serve_once(b"HTTP/1.1 500 Internal Server Error\r\n\r\nboom").await;
    let err = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Refused(500)));
    assert_eq!(err.to_string(), "the decision service answered 500");

    let (port, _s) = serve_once(b"HTTP/1.1 302 Found\r\nLocation: /x\r\n\r\n").await;
    let err = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, DecisionError::Refused(302)),
        "a redirect is not an answer either"
    );

    let (port, _s) = serve_once(b"garbage with no header break").await;
    let err = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "the answer had no headers");

    let (port, _s) = serve_once(b"HTTP/1.1 OK\r\n\r\n{}").await;
    let err = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "unreadable status line: `HTTP/1.1 OK`");

    let (port, _s) = serve_once(b"HTTP/1.1 200 OK\r\n\r\n{\"destination\":\"seven\"}").await;
    let err = client(port)
        .decide(&DecideRequest::default())
        .await
        .unwrap_err();
    assert!(
        err.to_string().starts_with("decoding the answer: "),
        "{err}"
    );
}

#[tokio::test]
async fn events_and_bin_states_go_to_their_endpoints_as_json() {
    // An empty batch never dials (no service is listening on port 1).
    DecisionClient::new("http://127.0.0.1:1", Duration::from_secs(1))
        .post_events(&[])
        .await
        .unwrap();

    let (port, server) = serve_once(b"HTTP/1.1 200 OK\r\n\r\n").await;
    client(port).post_events(&[event(3)]).await.unwrap();
    let request = server.await.unwrap();
    assert!(
        request.starts_with("POST /events HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(request.contains("Connection: close\r\n"), "{request}");
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let sent: Vec<LifecycleEvent> = serde_json::from_str(body).unwrap();
    assert_eq!(sent, vec![event(3)]);

    let (port, server) = serve_once(b"HTTP/1.1 200 OK\r\n\r\n").await;
    client(port).post_bin_state(12, 0x4, true).await.unwrap();
    let request = server.await.unwrap();
    assert!(
        request.starts_with("POST /binstate HTTP/1.1\r\n"),
        "{request}"
    );
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"destination": 12, "state": 4, "blocked": true})
    );
}

#[test]
fn a_raw_frame_survives_the_round_trip_through_hex() {
    // The store's promise: every recorded event decodes again.
    let raw = vec![0xFD, 0x00, 0x01, 0x02, 0x7F, 0xFE];
    let event = LifecycleEvent {
        raw_hex: LifecycleEvent::hex(&raw),
        ..event(1)
    };
    assert_eq!(event.raw_hex, "FD000102 7FFE".replace(' ', ""));
    assert_eq!(event.bytes(), Some(raw));
}

#[test]
fn the_event_queue_drops_the_oldest_rather_than_blocking() {
    // A stalled store must never slow a sort decision.
    let queue = EventQueue::new(3);
    for i in 0..5 {
        queue.push(event(i));
    }
    let stats = queue.stats();
    assert_eq!(stats.queued, 3);
    assert_eq!(stats.dropped, 2, "the oldest two");

    let batch = queue.take(10);
    assert_eq!(batch.len(), 3);
    assert_eq!(
        batch
            .iter()
            .map(|e| e.object_id.unwrap())
            .collect::<Vec<_>>(),
        vec![2, 3, 4],
        "the newest survived, in order"
    );
    queue.note_sent(batch.len());
    assert_eq!(queue.stats().sent, 3);
    assert!(queue.stats().last_error.is_none());
}

#[test]
fn a_failed_batch_goes_back_in_order() {
    let queue = EventQueue::new(10);
    for i in 0..3 {
        queue.push(event(i));
    }
    let batch = queue.take(2);
    queue.push(event(99));
    queue.requeue(batch, "trajetorias is down".into());

    let all = queue.take(10);
    assert_eq!(
        all.iter().map(|e| e.object_id.unwrap()).collect::<Vec<_>>(),
        vec![0, 1, 2, 99],
        "requeued events keep their place at the front"
    );
    assert_eq!(
        queue.stats().last_error.as_deref(),
        Some("trajetorias is down")
    );
}

#[tokio::test]
async fn an_unreachable_service_fails_fast_instead_of_hanging() {
    // The property the wire fallback depends on: a decision call
    // must not outlive the controller's patience.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let client = DecisionClient::new(
        format!("http://127.0.0.1:{port}"),
        Duration::from_millis(300),
    );
    let started = std::time::Instant::now();
    let result = client.decide(&DecideRequest::default()).await;
    // Refused at once on Linux; on Windows a closed loopback port is retried, so the
    // bound answers first. Either way an error, and never late.
    assert!(
        matches!(
            result,
            Err(DecisionError::Connect { .. } | DecisionError::Timeout(_))
        ),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_service_that_never_answers_times_out() {
    // Accept the connection and say nothing — the shape of a hung
    // decision service.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let client = DecisionClient::new(
        format!("http://127.0.0.1:{port}"),
        Duration::from_millis(250),
    );
    let started = std::time::Instant::now();
    let err = client
        .decide(&DecideRequest::default())
        .await
        .expect_err("must time out");
    assert!(matches!(err, DecisionError::Timeout(t) if t == Duration::from_millis(250)));
    assert_eq!(err.to_string(), "no answer within 250ms");
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(250) && took < Duration::from_millis(900), "{took:?}");
}

#[tokio::test]
async fn a_real_exchange_carries_the_decision_back() {
    // A stand-in trajetorias: read the request, answer JSON.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let body =
            br#"{"destination":42,"alternatives":[43,44],"tier":"routing rule","decision_id":7}"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        socket.write_all(head.as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        socket.shutdown().await.unwrap();
        request
    });

    let client = DecisionClient::new(format!("http://127.0.0.1:{port}"), Duration::from_secs(5));
    let mut fields = std::collections::HashMap::new();
    fields.insert("infeed".to_string(), "2".to_string());
    let answer = client
        .decide(&DecideRequest {
            object_id: 4242,
            cell: 100,
            fields,
            weight_g: Some(2500),
            ..Default::default()
        })
        .await
        .expect("a decision");

    assert_eq!(answer.destination, Some(42));
    assert_eq!(answer.alternatives, vec![43, 44]);
    assert_eq!(answer.tier, "routing rule");
    assert_eq!(answer.decision_id, Some(7));

    // And the request carried what the plan needs to decide.
    let request = server.await.unwrap();
    assert!(request.starts_with("POST /decide HTTP/1.1"), "{request}");
    assert!(request.contains("Content-Type: application/json"));
    assert!(request.contains("\"object_id\":4242"), "{request}");
    assert!(request.contains("\"infeed\":\"2\""), "{request}");
    assert!(request.contains("\"weight_g\":2500"), "{request}");
}

#[tokio::test]
async fn a_service_with_no_answer_is_distinguishable_from_a_chute() {
    // trajetorias may legitimately have no destination; that is an
    // ANSWER (send FORCE_REASSIGNEMENT), not an error.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let body = br#"{"destination":null,"tier":"reject (nothing eligible)"}"#;
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        socket.write_all(head.as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        let _ = socket.shutdown().await;
    });

    let client = DecisionClient::new(format!("http://127.0.0.1:{port}"), Duration::from_secs(5));
    let answer = client.decide(&DecideRequest::default()).await.expect("ok");
    assert_eq!(answer.destination, None);
    assert!(answer.tier.contains("reject"));
}

#[tokio::test]
async fn the_bound_covers_a_reply_that_drips_in() {
    // A valid answer, one byte every 40 ms: each read makes progress, the whole call does not.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut socket).await;
        for byte in b"HTTP/1.1 200 OK\r\n\r\n{\"destination\":5}" {
            if socket.write_all(&[*byte]).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    });
    let c = DecisionClient::new(format!("http://127.0.0.1:{port}"), Duration::from_millis(300));
    let started = std::time::Instant::now();
    let err = c.decide(&DecideRequest::default()).await.unwrap_err();
    let took = started.elapsed();
    assert!(matches!(err, DecisionError::Timeout(t) if t == Duration::from_millis(300)), "{err:?}");
    assert!(took >= Duration::from_millis(300) && took < Duration::from_millis(900), "{took:?}");
}

#[tokio::test]
async fn every_request_announces_its_exact_length() {
    for path in ["/decide", "/events", "/binstate"] {
        let (port, server) = serve_once(b"HTTP/1.1 200 OK\r\n\r\n{}").await;
        let c = client(port);
        match path {
            "/decide" => drop(c.decide(&DecideRequest { object_id: 9, ..Default::default() }).await.unwrap()),
            "/events" => c.post_events(&[event(1), event(2)]).await.unwrap(),
            _ => c.post_bin_state(3, 1, false).await.unwrap(),
        }
        // `read_request` read exactly Content-Length bytes after the head and asserted nothing
        // followed; here the announced length is the body's.
        let request = server.await.unwrap();
        let (head, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with(&format!("POST {path} HTTP/1.1\r\n")), "{head}");
        assert!(head.contains(&format!("\r\nContent-Length: {}\r\n", body.len())), "{head}");
        assert!(head.contains("\r\nContent-Type: application/json\r\n"), "{head}");
    }
}

#[test]
fn a_requeue_onto_a_full_queue_keeps_the_bound_and_drops_the_oldest() {
    let ids = |q: &EventQueue| q.take(10).iter().map(|e| e.object_id.unwrap()).collect::<Vec<_>>();

    let q = EventQueue::new(3);
    for i in 0..3 {
        q.push(event(i));
    }
    let batch = q.take(3);
    q.push(event(3));
    q.requeue(batch, "down".into());
    assert_eq!((q.stats().queued, q.stats().dropped), (3, 1));
    assert_eq!(ids(&q), [1, 2, 3], "the oldest of the batch went, the newer event stayed");

    let q = EventQueue::new(3);
    for i in 0..3 {
        q.push(event(i));
    }
    let batch = q.take(2);
    q.push(event(3));
    q.push(event(4));
    q.requeue(batch, "down".into());
    assert_eq!((q.stats().queued, q.stats().dropped), (3, 2));
    assert_eq!(ids(&q), [2, 3, 4]);
}
