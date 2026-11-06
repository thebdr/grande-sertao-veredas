//! Every assumption the opc layer makes about `async-opcua`, pinned against a REAL server
//! (C-023), not reasoned about. Run this first after bumping the `async-opcua-*` pin: a failure
//! here names the assumption that broke before it resurfaces as a mystery in a worker.
//!
//! What is pinned:
//!   1. anonymous connect, and that the session reports itself connected
//!   2. username/password connect, and that a WRONG password is rejected
//!   3. batch read: one call, N nodes, results in request order, good ones with no bad status
//!   4. a read of an unknown node yields a bad STATUS, not a failed call
//!   5. a write lands in the server's address space (checked server-side); a read-only node
//!      refuses with a bad status
//!   6. browse lists a folder's children with what the browser needs (node id, display name,
//!      node class, type definition)
//!   7. a server refuses a Read past its per-call cap WHOLE, with its own status (the reason
//!      reads are chunked), and takes one at the cap; past the stack's array limit of 1 000 a
//!      request fails to decode whatever the cap (the reason the chunk stays under it)
//!   8. a browse returns at most the references asked for, and at most the server's own page,
//!      with a continuation point that BrowseNext follows to the rest (the reason the browser
//!      loops); a released point is gone
//!   9. a status code's severity is its top two bits, 0b11 reading as bad, a good code with
//!      sub-status bits still good, and it displays as its name (what `Quality` and the failure
//!      texts rest on)
//!  10. the wire types the demo plant exposes map as `opc::value` expects, and the attributes
//!      the browser reads come in the types its renderers match on
//!  11. the demo plant's slow tag takes its stated time per read

use std::sync::Arc;
use std::time::{Duration, Instant};

use opcua_client::{Client, ClientBuilder, IdentityToken, Session};
use opcua_types::{
    AttributeId, BrowseDescription, BrowseDirection, BrowseResultMask, DataValue, EndpointDescription,
    MessageSecurityMode, NodeClass, NodeClassMask, NodeId, ObjectId, QualifiedName, ReadValueId, ReferenceTypeId,
    StatusCode, TimestampsToReturn, UserTokenPolicy, Variant, WriteValue,
};
use veredas_core::opc::demo::{BULK_COUNT, DEMO_PASS, DEMO_USER, DemoConfig, DemoPlant, MAX_NODES_PER_READ, SLOW_READ, tags};
use veredas_core::opc::{bit_field, opc_to_value};
use veredas_core::{SeriesType, Value};

/// A client configured as the worker configures one: no keypair (SecurityPolicy None), a
/// short retry budget so a failing test fails fast rather than hanging.
fn client() -> Client {
    ClientBuilder::new()
        .application_name("veredas test")
        .application_uri("urn:veredas:test")
        .product_uri("urn:veredas:test")
        .create_sample_keypair(false)
        .trust_server_certs(true)
        .session_retry_limit(1)
        .client()
        .expect("client builds")
}

fn endpoint(url: &str) -> EndpointDescription {
    (url, "None", MessageSecurityMode::None, UserTokenPolicy::anonymous()).into()
}

/// Connect and wait until the session is usable, or fail with why. Races the event loop
/// against the connection wait, the library's own pattern and the reason a REJECTED credential
/// reports in milliseconds: `wait_for_connection` alone never resolves.
async fn connect(
    plant: &DemoPlant,
    token: IdentityToken,
) -> Result<(Arc<Session>, tokio::task::JoinHandle<StatusCode>), String> {
    let mut client = client();
    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint(&plant.endpoint_url), token)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let mut handle = event_loop.spawn();
    let settled = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::select! {
            status = &mut handle => Err(format!("event loop gave up: {status:?}")),
            ok = session.wait_for_connection() => {
                if ok { Ok(()) } else { Err("session never reached connected".to_string()) }
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for connection".to_string())?;
    settled.map(|()| (session, handle))
}

fn read_value_id(node: &NodeId) -> ReadValueId {
    ReadValueId { node_id: node.clone(), attribute_id: AttributeId::Value as u32, ..Default::default() }
}

fn describe(node: NodeId) -> BrowseDescription {
    BrowseDescription {
        node_id: node,
        browse_direction: BrowseDirection::Forward,
        reference_type_id: ReferenceTypeId::HierarchicalReferences.into(),
        include_subtypes: true,
        node_class_mask: NodeClassMask::all().bits(),
        result_mask: BrowseResultMask::All as u32,
    }
}

#[tokio::test]
async fn anonymous_connect_read_and_batch_order() {
    let plant = DemoPlant::start().await.expect("demo plant starts");
    plant.set(tags::TEMPERATURE, 42.5f64);
    plant.set(tags::STATUS_WORD, 0b0101_0011u16);
    plant.set(tags::BATCH_ID, "B-042");

    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("anonymous connect");

    // ONE read call carrying several nodes: the worker's shape.
    let nodes = [plant.node_id(tags::TEMPERATURE), plant.node_id(tags::STATUS_WORD), plant.node_id(tags::BATCH_ID)];
    let to_read: Vec<ReadValueId> = nodes.iter().map(read_value_id).collect();
    let values = session.read(&to_read, TimestampsToReturn::Both, 0.0).await.expect("read succeeds");

    // Results come back positionally, one per request, in request order: the worker zips them
    // against its tag list, so this is load-bearing.
    assert_eq!(values.len(), 3);
    assert_eq!(values[0].value, Some(Variant::Double(42.5)));
    assert_eq!(values[1].value, Some(Variant::UInt16(0b0101_0011)));
    assert_eq!(values[2].value, Some(Variant::from("B-042")));
    for v in &values {
        // A good read carries no status or a good one: the worker reads "none" as good.
        assert!(v.status.is_none_or(|s| s.is_good()), "{:?}", v.status);
        assert!(v.source_timestamp.is_some(), "source timestamp present");
    }

    // The value layer agrees with the wire types this server sends.
    let temp = values[0].value.clone().unwrap();
    assert_eq!(opc_to_value(&temp, SeriesType::Float).unwrap(), Value::Float(42.5));
    let status = values[1].value.clone().unwrap();
    assert_eq!(bit_field(&status, 0, 1).unwrap(), Value::Int(1)); // running
    assert_eq!(bit_field(&status, 2, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&status, 4, 3).unwrap(), Value::Int(5)); // mode field
    let batch = values[2].value.clone().unwrap();
    assert_eq!(opc_to_value(&batch, SeriesType::Text).unwrap(), Value::Text(Arc::from("B-042")));

    plant.stop().await;
}

#[tokio::test]
async fn an_unknown_node_is_a_bad_status_not_a_failed_call() {
    // A whole poll must not die because one tag id was mistyped.
    let plant = DemoPlant::start().await.expect("demo plant starts");
    plant.set(tags::TEMPERATURE, 1.0f64);
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");

    let to_read = vec![read_value_id(&plant.node_id(tags::TEMPERATURE)), read_value_id(&NodeId::new(9, "no.such.tag"))];
    let values = session.read(&to_read, TimestampsToReturn::Both, 0.0).await.expect("the read CALL still succeeds");

    assert_eq!(values.len(), 2);
    assert!(values[0].status.is_none_or(|s| s.is_good()));
    assert_eq!(values[1].status, Some(StatusCode::BadNodeIdUnknown));
    assert!(values[1].value.is_none(), "a bad read carries no value");

    plant.stop().await;
}

#[tokio::test]
async fn username_password_accepts_the_right_pair_and_rejects_the_wrong_one() {
    let plant = DemoPlant::start().await.expect("demo plant starts");

    let good = IdentityToken::UserName(DEMO_USER.into(), DEMO_PASS.into());
    let (session, _loop) = connect(&plant, good).await.expect("valid credentials");
    // The session works, not merely connect returned.
    let v = session
        .read(&[read_value_id(&plant.node_id(tags::TEMPERATURE))], TimestampsToReturn::Both, 0.0)
        .await
        .expect("authenticated read");
    assert_eq!(v.len(), 1);

    // A wrong password must NOT yield a working session.
    let bad = IdentityToken::UserName(DEMO_USER.into(), "wrong".into());
    let t0 = Instant::now();
    assert!(connect(&plant, bad).await.is_err(), "a wrong password must fail to establish a session");
    assert!(t0.elapsed() < Duration::from_secs(10), "rejected promptly: {:?}", t0.elapsed());
    // An unknown user likewise.
    let stranger = IdentityToken::UserName("nobody".into(), DEMO_PASS.into());
    let t0 = Instant::now();
    assert!(connect(&plant, stranger).await.is_err());
    assert!(t0.elapsed() < Duration::from_secs(10), "rejected promptly: {:?}", t0.elapsed());

    plant.stop().await;
}

#[tokio::test]
async fn write_lands_in_the_address_space_and_readonly_is_refused() {
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");

    let write = |node: NodeId, v: Variant| WriteValue {
        node_id: node,
        attribute_id: AttributeId::Value as u32,
        value: DataValue::value_only(v),
        ..Default::default()
    };
    // One call, several nodes: a multi-tag command.
    let writes = vec![
        write(plant.node_id(tags::CMD_START), Variant::Int32(1)),
        write(plant.node_id(tags::SETPOINT), Variant::Double(73.5)),
    ];
    let results = session.write(&writes).await.expect("write call succeeds");
    assert_eq!(results, vec![StatusCode::Good, StatusCode::Good], "one status per write, in request order");

    // Server-side truth: the value reached the address space. Good alone would not prove it.
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(1)));
    assert_eq!(plant.get(tags::SETPOINT), Some(Variant::Double(73.5)));

    // The other half of the pulse.
    assert_eq!(session.write(&[write(plant.node_id(tags::CMD_START), Variant::Int32(0))]).await.expect("write"), vec![StatusCode::Good]);
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(0)));

    // A non-writable node refuses with a bad status, in its place, and does not move.
    let before = plant.get(tags::TEMPERATURE);
    let refused = session
        .write(&[
            write(plant.node_id(tags::TEMPERATURE), Variant::Double(0.0)),
            write(plant.node_id(tags::CMD_START), Variant::Int32(1)),
        ])
        .await
        .expect("call succeeds");
    assert!(refused[0].is_bad(), "{refused:?}");
    assert_eq!(refused[1], StatusCode::Good);
    assert_eq!(plant.get(tags::TEMPERATURE), before);

    plant.stop().await;
}

#[tokio::test]
async fn browse_enumerates_children_with_browser_metadata() {
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");

    let results = session.browse(&[describe(plant.node_id("line1"))], 1_000, None).await.expect("browse succeeds");
    assert_eq!(results.len(), 1, "one result per browsed node");
    assert!(results[0].continuation_point.is_null_or_empty());
    let refs = results[0].references.clone().unwrap_or_default();
    assert_eq!(refs.len(), 5, "the demo folder holds five tags");

    let mut names: Vec<String> = Vec::new();
    for r in &refs {
        assert_eq!(r.node_class, NodeClass::Variable, "tags are variables");
        assert!(!r.node_id.node_id.is_null(), "a child carries its node id");
        assert!(!r.display_name.text.as_ref().is_empty(), "a child carries a display name");
        assert_ne!(r.type_definition.node_id, NodeId::null(), "a variable carries its type definition");
        names.push(r.browse_name.name.as_ref().to_string());
    }
    names.sort();
    assert_eq!(names, vec!["BatchId", "CmdStart", "Setpoint", "StatusWord", "Temperature"]);

    // And the root is reachable, so the tree has somewhere to start.
    let roots = session.browse(&[describe(ObjectId::ObjectsFolder.into())], 1_000, None).await.expect("browse root");
    let root_refs = roots[0].references.clone().unwrap_or_default();
    assert!(root_refs.iter().any(|r| r.browse_name == QualifiedName::from("Line1")));

    plant.stop().await;
}

#[tokio::test]
async fn a_read_past_the_servers_cap_is_refused_whole() {
    // Why the worker chunks: a server takes at most its cap per Read and refuses a bigger call
    // whole, as a failed call, not as per-node statuses.
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");
    let ids: Vec<ReadValueId> =
        (0..BULK_COUNT).map(|i| read_value_id(&plant.node_id(&format!("bulk.v{i:04}")))).collect();
    // The worker's chunk fits under this cap, which sits under the stack's array limit.
    const {
        assert!(
            BULK_COUNT > MAX_NODES_PER_READ
                && MAX_NODES_PER_READ > veredas_core::opc::worker::MAX_NODES_PER_READ
                && ARRAY_LIMIT > MAX_NODES_PER_READ
        )
    };

    // One past the cap: the server faults the call with its own status, which the stack keeps.
    let over = session.read(&ids[..MAX_NODES_PER_READ + 1], TimestampsToReturn::Both, 0.0).await;
    let err = over.expect_err("one past the cap is refused");
    assert_eq!(err.status(), StatusCode::BadTooManyOperations, "{err}");
    assert_eq!(err.to_string(), "BadTooManyOperations: Request returned ServiceFault");

    let at = session.read(&ids[..MAX_NODES_PER_READ], TimestampsToReturn::Both, 0.0).await.expect("at the cap");
    assert_eq!(at.len(), MAX_NODES_PER_READ);
    assert_eq!(at[MAX_NODES_PER_READ - 1].value, Some(Variant::Int32(MAX_NODES_PER_READ as i32 - 1)));

    // The session survives the refusal.
    let one = session.read(&ids[..1], TimestampsToReturn::Both, 0.0).await.expect("still usable");
    assert_eq!(one[0].value, Some(Variant::Int32(0)));

    plant.stop().await;
}

/// The stack's own array limit: a request of more than 1 000 elements fails to decode at the
/// server before any per-call cap is checked (what a 3,048-tag read met in the field, as
/// "BadDecodingError: Request returned ServiceFault"), and the client decodes no larger answer.
/// So the worker's chunk must stay at or under it, whatever a server's cap.
const ARRAY_LIMIT: usize = 1_000;

#[tokio::test]
async fn a_read_past_the_stacks_array_limit_fails_to_decode_whatever_the_cap() {
    assert_eq!(opcua_types::constants::MAX_ARRAY_LENGTH, ARRAY_LIMIT);
    const { assert!(veredas_core::opc::worker::MAX_NODES_PER_READ <= ARRAY_LIMIT) };
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");
    let ids: Vec<ReadValueId> =
        (0..=ARRAY_LIMIT).map(|i| read_value_id(&plant.node_id(&format!("bulk.v{i:04}")))).collect();
    let err = session.read(&ids, TimestampsToReturn::Both, 0.0).await.expect_err("refused");
    assert_eq!(err.status(), StatusCode::BadDecodingError, "{err}");
    assert_eq!(err.to_string(), "BadDecodingError: Request returned ServiceFault");
    let one = session.read(&ids[..1], TimestampsToReturn::Both, 0.0).await.expect("still usable");
    assert_eq!(one[0].value, Some(Variant::Int32(0)));
    plant.stop().await;
}

#[tokio::test]
async fn a_browse_pages_and_browse_next_follows_to_the_rest() {
    // Why the browser loops: the server returns at most what was asked per round, with a
    // continuation point for the rest.
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");

    let first = session.browse(&[describe(plant.node_id("bulk"))], 500, None).await.expect("browse");
    let refs = first[0].references.clone().unwrap_or_default();
    assert_eq!(refs.len(), 500);
    assert!(!first[0].continuation_point.is_null_or_empty(), "more to come");

    let mut all = refs.len();
    let mut cp = first[0].continuation_point.clone();
    let mut rounds = 0;
    while !cp.is_null_or_empty() {
        rounds += 1;
        let next = session.browse_next(false, &[cp]).await.expect("browse next");
        assert!(next[0].status_code.is_good(), "{:?}", next[0].status_code);
        all += next[0].references.as_ref().map_or(0, |r| r.len());
        cp = next[0].continuation_point.clone();
    }
    assert_eq!((all, rounds), (BULK_COUNT, 2));

    // Releasing a point mid-way is accepted, and the point is gone after it.
    let first = session.browse(&[describe(plant.node_id("bulk"))], 100, None).await.expect("browse");
    let cp = first[0].continuation_point.clone();
    let released = session.browse_next(true, std::slice::from_ref(&cp)).await.expect("release");
    assert_eq!(released[0].status_code, StatusCode::Good);
    assert!(released[0].references.as_ref().is_none_or(|r| r.is_empty()));
    let reused = session.browse_next(false, &[cp]).await.expect("call succeeds");
    assert_eq!(reused[0].status_code, StatusCode::BadContinuationPointInvalid);

    plant.stop().await;
}

#[tokio::test]
async fn a_server_pages_at_its_own_limit_whatever_is_asked() {
    // A server with a smaller page than the client asks for (the browser asks for 1 000).
    let plant = DemoPlant::start_with(DemoConfig { browse_page: Some(1), ..DemoConfig::default() })
        .await
        .expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");
    let first = session.browse(&[describe(plant.node_id("line1"))], 1_000, None).await.expect("browse");
    assert_eq!(first[0].references.as_ref().map(|r| r.len()), Some(1));
    assert!(!first[0].continuation_point.is_null_or_empty());
    plant.stop().await;
}

#[tokio::test]
async fn the_browser_s_requests_are_answered_as_it_reads_them() {
    // What browse.rs sends and expects, pinned where a version bump would show it first.
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");

    // A browse with no class filter (mask 0) lists every child.
    let mut unfiltered = describe(plant.node_id("line1"));
    unfiltered.node_class_mask = 0;
    let refs = session.browse(&[unfiltered], 1_000, None).await.expect("browse")[0].references.clone().unwrap_or_default();
    assert_eq!(refs.len(), 5);

    // A full page of the browser's size decodes: its page sits at the stack's array limit.
    const { assert!(veredas_core::opc::browse::BROWSE_PAGE as usize <= ARRAY_LIMIT) };
    let page = veredas_core::opc::browse::BROWSE_PAGE;
    let full = session.browse(&[describe(plant.node_id("bulk"))], page, None).await.expect("a full page");
    assert_eq!(full[0].references.as_ref().map(|r| r.len()), Some(page as usize));

    // The attributes come back in the types the renderers match on; one a node lacks is
    // BadAttributeIdInvalid.
    let attr = |node: NodeId, attribute: AttributeId| ReadValueId {
        node_id: node,
        attribute_id: attribute as u32,
        ..Default::default()
    };
    let temp = plant.node_id(tags::TEMPERATURE);
    let values = session
        .read(
            &[
                attr(temp.clone(), AttributeId::DataType),
                attr(temp.clone(), AttributeId::NodeClass),
                attr(temp.clone(), AttributeId::AccessLevel),
                attr(temp.clone(), AttributeId::ValueRank),
                attr(temp.clone(), AttributeId::BrowseName),
                attr(temp.clone(), AttributeId::EventNotifier),
                attr(temp.clone(), AttributeId::UserAccessLevel),
                attr(temp, AttributeId::MinimumSamplingInterval),
            ],
            TimestampsToReturn::Neither,
            0.0,
        )
        .await
        .expect("read");
    assert_eq!(values[0].value, Some(Variant::from(NodeId::new(0, 11u32))));
    assert_eq!(values[1].value, Some(Variant::Int32(2)));
    assert_eq!(values[2].value, Some(Variant::Byte(1)));
    assert_eq!(values[3].value, Some(Variant::Int32(-1)));
    assert_eq!(values[4].value, Some(Variant::from(QualifiedName::new(0, "Temperature"))));
    assert_eq!(values[5].status, Some(StatusCode::BadAttributeIdInvalid));
    assert!(values[5].value.is_none());
    assert_eq!(values[6].value, Some(Variant::Byte(1)));
    assert_eq!(values[7].value, Some(Variant::Double(100.0)));
    plant.stop().await;
}

#[tokio::test]
async fn a_browse_page_past_the_array_limit_kills_the_connection() {
    // Why the browser never asks for more than the limit: a server that serves the page asked
    // for sends an answer this client cannot decode, and the connection dies with it.
    let plant = DemoPlant::start_with(DemoConfig { browse_page: Some(5_000), ..DemoConfig::default() })
        .await
        .expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");
    let at = session.browse(&[describe(plant.node_id("bulk"))], ARRAY_LIMIT as u32, None).await.expect("at the limit");
    assert_eq!(at[0].references.as_ref().map(|r| r.len()), Some(ARRAY_LIMIT));
    let past = session.browse(&[describe(plant.node_id("bulk"))], ARRAY_LIMIT as u32 + 1, None).await;
    let err = past.expect_err("one past the limit");
    assert_eq!(err.status(), StatusCode::BadConnectionClosed, "{err}");
    plant.stop().await;
}

#[test]
fn a_status_codes_severity_is_its_top_two_bits_and_it_displays_its_name() {
    let good = StatusCode::Good;
    assert!(good.is_good() && !good.is_uncertain() && !good.is_bad());
    let unsure = StatusCode::UncertainLastUsableValue;
    assert!(unsure.is_uncertain() && !unsure.is_good() && !unsure.is_bad());
    assert_eq!(unsure.bits() >> 30, 0b01);
    let bad = StatusCode::BadNodeIdUnknown;
    assert!(bad.is_bad() && !bad.is_good() && !bad.is_uncertain());
    assert_eq!(bad.bits() >> 30, 0b10);
    // The reserved severity reads as bad.
    let reserved = StatusCode::from(0xC000_0000u32);
    assert!(reserved.is_bad() && !reserved.is_good() && !reserved.is_uncertain());
    // A good code with sub-status or info bits is still good: a server answering GoodClamped
    // or GoodLocalOverride has given a value.
    for (bits, name) in [(0x0030_0000u32, "GoodClamped"), (0x0096_0000, "GoodLocalOverride")] {
        let code = StatusCode::from(bits);
        assert!(code.is_good() && !code.is_uncertain() && !code.is_bad(), "{name}");
        assert_eq!(code.to_string(), name);
    }
    // Info bits (a limit, an overflow) leave a good code good too.
    for bits in [0x0000_0100u32, 0x0000_0200, 0x0000_0300, 0x0000_0480] {
        let code = StatusCode::from(bits);
        assert!(code.is_good() && !code.is_uncertain() && !code.is_bad(), "{bits:#x}");
    }
    // Names, as the failure texts and the browser's rows show them.
    assert_eq!(good.to_string(), "Good");
    assert_eq!(bad.to_string(), "BadNodeIdUnknown");
    assert_eq!(StatusCode::BadTooManyOperations.to_string(), "BadTooManyOperations");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_slow_tag_takes_its_stated_time() {
    let plant = DemoPlant::start().await.expect("demo plant starts");
    let (session, _loop) = connect(&plant, IdentityToken::Anonymous).await.expect("connect");
    let t0 = Instant::now();
    let v = session
        .read(&[read_value_id(&plant.node_id(tags::SLOW))], TimestampsToReturn::Both, 0.0)
        .await
        .expect("read");
    let took = t0.elapsed();
    assert_eq!(v[0].value, Some(Variant::Int32(0)));
    assert!(took >= SLOW_READ && took < SLOW_READ + Duration::from_secs(2), "{took:?}");
    plant.stop().await;
}
