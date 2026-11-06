//! The address-space browser (C-021): the renderers and the refusal it reads, then the browser
//! against the demo plant over real OPC UA. Veredas 1's unit and end-to-end tests, plus every
//! renderer's exact text, a silent endpoint, a malformed node id, and a drop mid-connect.

use std::net::TcpListener;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use opcua_types::{BrowseResult, ByteString, DateTime, LocalizedText, QualifiedName};
use veredas_core::SeriesType;
use veredas_core::opc::browse::{
    BRIEF_MAX, BrowseEvent, BrowsedNode, access_level_text, data_type_text, node_class_text, result_refusal,
    series_type_of, value_rank_text, variant_brief,
};
use veredas_core::opc::demo::{BULK_COUNT, DemoPlant, tags};
use veredas_core::opc::spec::Auth;
use veredas_core::opc::worker::CONNECT_TIMEOUT;
use veredas_core::opc::{Browser, Call, NodeId, NodeIdError, OpcError, StatusCode, Variant};

fn builtin(id: u32) -> NodeId {
    NodeId::new(0, id)
}

#[test]
fn builtin_data_types_map_to_series_types() {
    // An operator should not have to know that a UInt16 is an `int` series.
    let int = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    let text = [12, 14, 15, 16, 17, 21];
    for id in 0..=30u32 {
        let want = if int.contains(&id) {
            Some(SeriesType::Int)
        } else if id == 10 || id == 11 {
            Some(SeriesType::Float)
        } else if text.contains(&id) {
            Some(SeriesType::Text)
        } else if id == 13 {
            Some(SeriesType::Time)
        } else {
            None
        };
        assert_eq!(series_type_of(&builtin(id)), want, "i={id}");
    }
}

#[test]
fn unknown_and_custom_types_prefill_nothing() {
    // A custom structure has no sensible default: the operator chooses.
    assert_eq!(series_type_of(&builtin(9999)), None);
    // A custom type in another namespace, likewise, even with a built-in's number.
    assert_eq!(series_type_of(&NodeId::new(2, 11u32)), None);
    // A string-identified data type is not a built-in.
    assert_eq!(series_type_of(&NodeId::new(0, "Double")), None);
}

#[test]
fn detail_rows_render_in_words_not_magic_numbers() {
    assert_eq!(access_level_text(0x01), "read (0x01)");
    assert_eq!(access_level_text(0x02), "write (0x02)");
    assert_eq!(access_level_text(0x03), "read, write (0x03)");
    assert_eq!(access_level_text(0x04), "history read (0x04)");
    assert_eq!(access_level_text(0x08), "history write (0x08)");
    assert_eq!(access_level_text(0x0F), "read, write, history read, history write (0x0F)");
    assert_eq!(access_level_text(0x00), "none (0x00)");
    // Bits beyond the four named ones show only in the byte.
    assert_eq!(access_level_text(0xF0), "none (0xF0)");
    assert_eq!(access_level_text(0x21), "read (0x21)");

    assert_eq!(value_rank_text(-3), "scalar or one-dimensional array");
    assert_eq!(value_rank_text(-2), "scalar or array");
    assert_eq!(value_rank_text(-1), "scalar");
    assert_eq!(value_rank_text(0), "array (any dimensions)");
    assert_eq!(value_rank_text(1), "1-dimensional array");
    assert_eq!(value_rank_text(2), "2-dimensional array");
    assert_eq!(value_rank_text(-4), "-4");

    let classes = [
        (1, "Object"),
        (2, "Variable"),
        (4, "Method"),
        (8, "ObjectType"),
        (16, "VariableType"),
        (32, "ReferenceType"),
        (64, "DataType"),
        (128, "View"),
        (0, "Unspecified"),
        (3, "Unspecified"),
    ];
    for (n, name) in classes {
        assert_eq!(node_class_text(n), name, "{n}");
    }

    // A DataType id carries its built-in name and the series mapping.
    assert_eq!(data_type_text(&builtin(11)), "Double (i=11) → float series");
    assert_eq!(data_type_text(&builtin(5)), "UInt16 (i=5) → int series");
    assert_eq!(data_type_text(&builtin(13)), "DateTime (i=13) → time series");
    assert_eq!(data_type_text(&builtin(22)), "Structure (i=22)");
    assert_eq!(data_type_text(&builtin(25)), "DiagnosticInfo (i=25)");
    assert_eq!(data_type_text(&builtin(26)), "i=26");
    // A custom type in another namespace stays the raw id, no guess.
    assert_eq!(data_type_text(&NodeId::new(3, "DTL")), "ns=3;s=DTL");
}

#[test]
fn a_bad_result_status_is_a_named_refusal_not_an_empty_folder() {
    // A strict server refuses a browse with a bad per-result status and zero references;
    // swallowing the status showed a whole controller as an empty tree. The demo stack is lax
    // (it answers an unknown node with Good and no references), so the refusal is pinned here
    // on a constructed result.
    let refused = BrowseResult {
        status_code: StatusCode::BadNodeIdUnknown,
        continuation_point: ByteString::null(),
        references: None,
    };
    let err = result_refusal(&refused, Call::Browse).expect("a bad status is a refusal");
    assert_eq!(err, OpcError::Refused { call: Call::Browse, status: StatusCode::BadNodeIdUnknown });
    assert_eq!(err.to_string(), "the server refused the browse: BadNodeIdUnknown");
    assert_eq!(
        result_refusal(&refused, Call::BrowseNext).unwrap().to_string(),
        "the server refused the browse continuation: BadNodeIdUnknown"
    );

    let ok = BrowseResult { status_code: StatusCode::Good, continuation_point: ByteString::null(), references: None };
    assert_eq!(result_refusal(&ok, Call::Browse), None, "good and empty IS empty");
    // An uncertain verdict is not a refusal.
    let unsure = BrowseResult {
        status_code: StatusCode::UncertainNotAllNodesAvailable,
        continuation_point: ByteString::null(),
        references: None,
    };
    assert_eq!(result_refusal(&unsure, Call::Browse), None);
}

#[test]
fn variant_brief_is_one_line_and_bounded() {
    assert_eq!(variant_brief(&Variant::Double(21.5)), "21.5");
    assert_eq!(variant_brief(&Variant::Boolean(true)), "true");
    assert_eq!(variant_brief(&Variant::from("plain")), "plain");
    assert_eq!(variant_brief(&Variant::Empty), "(empty)");
    assert_eq!(variant_brief(&Variant::UInt64(u64::MAX)), "18446744073709551615");
    assert_eq!(variant_brief(&Variant::from(StatusCode::BadTimeout)), "BadTimeout");
    assert_eq!(variant_brief(&Variant::from(LocalizedText::new("en", "Running"))), "Running");
    assert_eq!(variant_brief(&Variant::from(QualifiedName::new(2, "Motor"))), "2:Motor");
    assert_eq!(variant_brief(&Variant::from(NodeId::new(2, "x"))), "ns=2;s=x");
    let epoch = DateTime::from(chrono::DateTime::from_timestamp(0, 0).unwrap());
    assert_eq!(variant_brief(&Variant::from(epoch)), epoch.to_string());
    // Arrays: the length, the first four, an ellipsis past them.
    assert_eq!(variant_brief(&Variant::from(vec![1i32, 2, 3])), "[3 items] 1, 2, 3");
    assert_eq!(variant_brief(&Variant::from(vec![1i32, 2, 3, 4, 5, 6])), "[6 items] 1, 2, 3, 4, …");
    // Long values cut at BRIEF_MAX characters and say so; exactly BRIEF_MAX is not cut.
    let long = variant_brief(&Variant::from("x".repeat(500)));
    assert_eq!(long, format!("{}…", "x".repeat(BRIEF_MAX)));
    assert_eq!(variant_brief(&Variant::from("y".repeat(BRIEF_MAX))), "y".repeat(BRIEF_MAX));
    // Counted in characters, not bytes: a multi-byte text under the limit is not cut.
    let wide = "é".repeat(100);
    assert_eq!(variant_brief(&Variant::from(wide.as_str())), wide);
    let wider = "é".repeat(BRIEF_MAX + 1);
    assert_eq!(variant_brief(&Variant::from(wider.as_str())), format!("{}…", "é".repeat(BRIEF_MAX)));
    // One line: a control character reads as a space.
    assert_eq!(variant_brief(&Variant::from("a\nb\r\tc")), "a b  c");
}

/// The next `Children` reply, or a failure naming what arrived instead.
fn next_children(b: &Browser, within: Duration) -> (Option<String>, Vec<BrowsedNode>) {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match b.events.recv_timeout(left) {
            Ok(BrowseEvent::Children { parent, nodes }) => return (parent, nodes),
            Ok(BrowseEvent::Connected | BrowseEvent::Details { .. }) => continue,
            Ok(BrowseEvent::Failed(e)) => panic!("browse failed: {e}"),
            Err(RecvTimeoutError::Timeout) => panic!("no children within {within:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the browser exited"),
        }
    }
}

/// The next `Details` reply.
fn next_details(b: &Browser, within: Duration) -> (String, Vec<(String, String)>) {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match b.events.recv_timeout(left) {
            Ok(BrowseEvent::Details { node_id, rows }) => return (node_id, rows),
            Ok(BrowseEvent::Connected | BrowseEvent::Children { .. }) => continue,
            Ok(BrowseEvent::Failed(e)) => panic!("details failed: {e}"),
            Err(RecvTimeoutError::Timeout) => panic!("no details within {within:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the browser exited"),
        }
    }
}

/// The next `Failed`, skipping `Connected`.
fn next_failure(b: &Browser, within: Duration) -> OpcError {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match b.events.recv_timeout(left) {
            Ok(BrowseEvent::Failed(e)) => return e,
            Ok(BrowseEvent::Connected) => continue,
            Ok(other) => panic!("expected a failure, got {other:?}"),
            Err(e) => panic!("no failure within {within:?}: {e:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_browser_walks_the_tree_and_prefills_tag_types() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let browser = Browser::connect(&plant.endpoint_url, Auth::Anonymous);
    assert_eq!(browser.events.recv_timeout(Duration::from_secs(20)).unwrap(), BrowseEvent::Connected);

    // The root is the Objects folder; the demo plant hangs off it.
    browser.browse(None);
    let (parent, roots) = next_children(&browser, Duration::from_secs(20));
    assert_eq!(parent, None, "the reply is keyed by what was browsed");
    let folder = roots.iter().find(|n| n.display_name == "Line 1").unwrap_or_else(|| panic!("{roots:?}"));
    assert!(folder.is_container, "a folder is expandable");
    assert!(!folder.is_variable, "a folder is not a tag");
    assert_eq!(folder.node_id, plant.node_id_str("line1"));
    // Siblings arrive ordered by display name.
    let names: Vec<&str> = roots.iter().map(|n| n.display_name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);

    // Expanding it lists the five demo tags, each with what a tag's configuration needs.
    browser.browse(Some(folder.node_id.clone()));
    let (parent, children) = next_children(&browser, Duration::from_secs(20));
    assert_eq!(parent.as_deref(), Some(folder.node_id.as_str()), "a late reply must not fill the wrong branch");
    type Row<'a> = (&'a str, &'a str, bool, bool, Option<SeriesType>, &'a str);
    let got: Vec<Row> = children
        .iter()
        .map(|c| {
            (c.display_name.as_str(), c.browse_name.as_str(), c.is_variable, c.is_container, c.series_type, c.data_type.as_str())
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("Batch id", "BatchId", true, true, Some(SeriesType::Text), "i=12"),
            ("Setpoint", "Setpoint", true, true, Some(SeriesType::Float), "i=11"),
            ("Start command", "CmdStart", true, true, Some(SeriesType::Int), "i=6"),
            ("Status word", "StatusWord", true, true, Some(SeriesType::Int), "i=5"),
            ("Temperature", "Temperature", true, true, Some(SeriesType::Float), "i=11"),
        ]
    );
    // The node id is in the exact text form a tag's configuration stores.
    let temp = children.iter().find(|c| c.display_name == "Temperature").unwrap();
    assert_eq!(temp.node_id, plant.node_id_str(tags::TEMPERATURE));

    drop(browser);
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hascomponent_subtrees_browse_like_organizes_ones() {
    // A controller wires its structure with HasComponent references; the demo's PLC_1 subtree
    // has that shape, and this is the walk an operator does.
    let plant = DemoPlant::start().await.expect("demo plant");
    let browser = Browser::connect(&plant.endpoint_url, Auth::Anonymous);

    browser.browse(None);
    let (_, roots) = next_children(&browser, Duration::from_secs(20));
    let plc = roots.iter().find(|n| n.display_name == "PLC_1").unwrap_or_else(|| panic!("{roots:?}"));

    browser.browse(Some(plc.node_id.clone()));
    let (_, inside) = next_children(&browser, Duration::from_secs(20));
    let db = inside.iter().find(|n| n.display_name == "DB process").unwrap_or_else(|| panic!("{inside:?}"));
    assert!(db.is_container && !db.is_variable);

    browser.browse(Some(db.node_id.clone()));
    let (_, in_db) = next_children(&browser, Duration::from_secs(20));
    assert_eq!(in_db.len(), 1, "{in_db:?}");
    assert_eq!(in_db[0].node_id, plant.node_id_str(tags::PLC_COUNTER));
    assert!(in_db[0].is_variable);
    assert_eq!(in_db[0].series_type, Some(SeriesType::Int), "UInt32 → int");

    // The standard Server object is listed under Objects too: nothing is filtered away.
    assert!(roots.iter().any(|n| n.browse_name == "Server"), "{roots:?}");

    drop(browser);
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_node_on_a_lax_stack_is_an_empty_list_not_an_error() {
    // The demo stack answers an unknown-node browse with a GOOD status and no references (a
    // strict stack refuses; that half is pinned on a constructed result above). Empty means
    // empty, delivered as a reply, so a branch never waits on a reply that will not come.
    let plant = DemoPlant::start().await.expect("demo plant");
    let browser = Browser::connect(&plant.endpoint_url, Auth::Anonymous);

    browser.browse(Some("ns=0;i=999999999".to_string()));
    let (parent, nodes) = next_children(&browser, Duration::from_secs(20));
    assert_eq!(parent.as_deref(), Some("ns=0;i=999999999"));
    assert!(nodes.is_empty(), "{nodes:?}");

    drop(browser);
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_node_id_fails_that_request_and_the_browser_serves_the_next() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let browser = Browser::connect(&plant.endpoint_url, Auth::Anonymous);

    browser.browse(Some("line1".to_string()));
    assert_eq!(
        next_failure(&browser, Duration::from_secs(20)),
        OpcError::NodeId(NodeIdError::Malformed("line1".into()))
    );
    browser.details("  ".to_string());
    assert_eq!(next_failure(&browser, Duration::from_secs(20)), OpcError::NodeId(NodeIdError::Empty));
    browser.browse(Some(plant.node_id_str("line1")));
    let (_, children) = next_children(&browser, Duration::from_secs(20));
    assert_eq!(children.len(), 5);

    drop(browser);
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn folders_larger_than_one_page_arrive_whole() {
    // BULK_COUNT variables are more than one browse page: without the BrowseNext loop this
    // returns a truncated list (the capabilities test pins that the server pages it).
    let plant = DemoPlant::start().await.expect("demo plant");
    let browser = Browser::connect(&plant.endpoint_url, Auth::Anonymous);

    browser.browse(Some(plant.node_id_str("bulk")));
    let (_, children) = next_children(&browser, Duration::from_secs(60));
    let ids: Vec<String> = children.iter().map(|c| c.node_id.clone()).collect();
    let want: Vec<String> = (0..BULK_COUNT).map(|i| plant.node_id_str(&format!("bulk.v{i:04}"))).collect();
    assert_eq!(ids, want, "every page arrived, each child once, in name order");
    // And every one of them, across chunks of the DataType read, got its type.
    assert!(children.iter().all(|c| c.series_type == Some(SeriesType::Int) && c.data_type == "i=6"));

    drop(browser);
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn details_read_the_standard_attribute_set() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let browser = Browser::connect(&plant.endpoint_url, Auth::Anonymous);
    plant.set(tags::TEMPERATURE, 21.5f64);

    let temp_id = plant.node_id_str(tags::TEMPERATURE);
    browser.details(temp_id.clone());
    let (node_id, rows) = next_details(&browser, Duration::from_secs(20));
    assert_eq!(node_id, temp_id, "keyed by the node asked about");
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    // The node id first, then the attributes in their fixed order; the ones this server holds
    // no value for (a description, array dimensions, a sampling interval) and the ones a
    // variable does not have (an EventNotifier) are skipped silently.
    assert_eq!(
        names,
        vec![
            "NodeId",
            "NodeClass",
            "BrowseName",
            "DisplayName",
            "DataType",
            "ValueRank",
            "AccessLevel",
            "UserAccessLevel",
            "Historizing",
            "Value"
        ]
    );
    assert_eq!(rows[5].1, "scalar");
    assert_eq!(rows[8].1, "false");
    let get = |name: &str| {
        rows.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or_else(|| panic!("no `{name}` in {rows:?}"))
    };
    assert_eq!(get("NodeId"), temp_id);
    assert_eq!(get("NodeClass"), "Variable");
    assert_eq!(get("BrowseName"), "0:Temperature");
    assert_eq!(get("DisplayName"), "Temperature");
    assert_eq!(get("DataType"), "Double (i=11) → float series");
    assert_eq!(get("AccessLevel"), "read (0x01)");
    assert!(get("Value").starts_with("21.5   (source "), "{rows:?}");
    assert!(!rows.iter().any(|(n, _)| n == "EventNotifier"), "a variable has none: {rows:?}");

    // A writable command tag says so.
    browser.details(plant.node_id_str(tags::CMD_START));
    let (_, rows) = next_details(&browser, Duration::from_secs(20));
    let access = rows.iter().find(|(n, _)| n == "AccessLevel").map(|(_, v)| v.clone()).expect("AccessLevel row");
    assert_eq!(access, "read, write (0x03)");

    // A folder is an object, with an EventNotifier and no value.
    browser.details(plant.node_id_str("line1"));
    let (_, rows) = next_details(&browser, Duration::from_secs(20));
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"EventNotifier") && !names.contains(&"Value"), "{rows:?}");
    assert_eq!(rows.iter().find(|(n, _)| n == "NodeClass").unwrap().1, "Object");

    drop(browser);
    plant.stop().await;
}

#[test]
fn an_unreachable_endpoint_reports_instead_of_hanging() {
    // Browsing happens while configuring, often against a mistyped address: it says so.
    let browser = Browser::connect("opc.tcp://127.0.0.1:1/", Auth::Anonymous);
    let e = next_failure(&browser, Duration::from_secs(30));
    assert!(matches!(e, OpcError::Connect { .. }), "{e:?}");
    assert!(!e.to_string().is_empty());
    // Nothing more arrives after a failed connection.
    assert!(browser.events.recv_timeout(Duration::from_millis(300)).is_err());
    let t0 = Instant::now();
    drop(browser);
    assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
}

#[test]
fn a_silent_endpoint_fails_within_the_connect_timeout() {
    // A peer that accepts TCP and never speaks OPC UA must not hold the browser.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("opc.tcp://{}/", silent.local_addr().unwrap());
    let t0 = Instant::now();
    let browser = Browser::connect(&url, Auth::Anonymous);
    let e = next_failure(&browser, CONNECT_TIMEOUT + Duration::from_secs(5));
    let took = t0.elapsed();
    assert!(took >= CONNECT_TIMEOUT && took <= CONNECT_TIMEOUT + Duration::from_secs(2), "{took:?}");
    assert_eq!(e, OpcError::ConnectTimedOut(CONNECT_TIMEOUT.as_secs()));
    assert_eq!(e.to_string(), "no session within 10 s");
    drop(silent);
}

#[test]
fn dropping_the_browser_mid_connect_returns_at_once() {
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("opc.tcp://{}/", silent.local_addr().unwrap());
    let browser = Browser::connect(&url, Auth::Anonymous);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    drop(browser);
    assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
}
