//! The compiled shape of an OPC source (C-018): node ids parsed with typed refusals, the polled
//! node set, the column order, the writable whitelist, the identity's label. Veredas 1's tests,
//! plus the edges.

use std::time::Duration;

use veredas_core::SeriesType;
use veredas_core::opc::spec::{Auth, BitSpec, PollSpec, RuleSpec, TagSpec, parse_node_id};
use veredas_core::opc::{NodeId, NodeIdError};

fn tag(raw: &str, series: &str, ty: SeriesType, writable: bool, bits: Vec<BitSpec>) -> TagSpec {
    TagSpec { node_id: parse_node_id(raw).unwrap(), raw: raw.to_string(), series: series.to_string(), ty, writable, bits }
}

fn bit(bit: u8, width: u8, series: &str) -> BitSpec {
    BitSpec { bit, width, series: series.into() }
}

fn rule(name: &str, tags: Vec<TagSpec>) -> RuleSpec {
    RuleSpec { name: name.into(), tags, store_on_change: false }
}

fn spec(rules: Vec<RuleSpec>) -> PollSpec {
    PollSpec {
        source_id: "plc".into(),
        display_name: "PLC".into(),
        endpoint_url: "opc.tcp://x/".into(),
        auth: Auth::Anonymous,
        poll: Duration::from_millis(500),
        rules,
    }
}

#[test]
fn node_ids_parse_in_every_standard_form() {
    assert_eq!(parse_node_id("ns=2;s=line1.temp").unwrap(), NodeId::new(2, "line1.temp"));
    assert_eq!(parse_node_id("i=2258").unwrap(), NodeId::new(0, 2258u32));
    assert_eq!(parse_node_id("ns=1;i=42").unwrap(), NodeId::new(1, 42u32));
    let guid = parse_node_id("ns=3;g=72962b91-fa75-4ae6-8d28-b404dc7daf63").unwrap();
    assert_eq!(guid.namespace, 3);
    assert_eq!(guid.to_string(), "ns=3;g=72962b91-fa75-4ae6-8d28-b404dc7daf63");
    let opaque = parse_node_id("ns=4;b=M/RbKBsRVkePCePcx24oRA==").unwrap();
    assert_eq!(opaque.namespace, 4);
    // Surrounding whitespace is the operator's, not the id's.
    assert_eq!(parse_node_id("  ns=2;s=a b \t").unwrap(), NodeId::new(2, "a b"));
    // The text form round-trips: what a browser shows is what a tag stores.
    assert_eq!(parse_node_id("ns=2;s=line1.temp").unwrap().to_string(), "ns=2;s=line1.temp");
}

#[test]
fn a_node_id_is_refused_with_the_input_quoted_and_the_form_shown() {
    assert_eq!(parse_node_id(""), Err(NodeIdError::Empty));
    assert_eq!(parse_node_id(" \t "), Err(NodeIdError::Empty));
    assert_eq!(parse_node_id("").unwrap_err().to_string(), "node id is empty");
    let err = parse_node_id(" line1.temp ").unwrap_err();
    assert_eq!(err, NodeIdError::Malformed("line1.temp".into()));
    assert_eq!(
        err.to_string(),
        "`line1.temp` is not a node id — expected `ns=<index>;s=<name>` (or `i=`, `g=`, `b=`)"
    );
    for bad in ["ns=;s=x", "ns=x;s=y", "i=notanumber", "q=1", "ns=2;", "s"] {
        assert_eq!(parse_node_id(bad), Err(NodeIdError::Malformed(bad.into())), "{bad}");
    }
}

#[test]
fn the_polled_node_set_is_deduplicated_across_and_within_rules_in_first_seen_order() {
    // The same status word feeding two tables, and twice in one, is read ONCE.
    let s = spec(vec![
        rule(
            "raw",
            vec![
                tag("ns=1;s=w", "word", SeriesType::Int, false, vec![]),
                tag("ns=1;s=t", "temp", SeriesType::Float, false, vec![]),
                tag("ns=1;s=w", "word_again", SeriesType::Int, false, vec![]),
            ],
        ),
        rule(
            "flags",
            vec![
                tag("ns=1;s=x", "x", SeriesType::Int, false, vec![]),
                tag("ns=1;s=w", "", SeriesType::Int, false, vec![bit(0, 1, "running")]),
            ],
        ),
    ]);
    assert_eq!(
        s.poll_nodes(),
        vec![NodeId::new(1, "w"), NodeId::new(1, "t"), NodeId::new(1, "x")]
    );
    assert_eq!(spec(vec![]).poll_nodes(), Vec::<NodeId>::new());

    // Different nodes that share an id value are all kept: a namespace per device reuses tag
    // names, and a numeric id is not the string of its digits.
    let s = spec(vec![
        rule(
            "a",
            vec![
                tag("ns=2;s=Temperature", "t2", SeriesType::Float, false, vec![]),
                tag("ns=3;s=Temperature", "t3", SeriesType::Float, false, vec![]),
            ],
        ),
        rule(
            "b",
            vec![
                tag("ns=1;i=5", "n", SeriesType::Int, false, vec![]),
                tag("ns=1;s=5", "s", SeriesType::Int, false, vec![]),
                tag("ns=3;s=Temperature", "t3_again", SeriesType::Float, false, vec![]),
            ],
        ),
    ]);
    assert_eq!(
        s.poll_nodes(),
        vec![NodeId::new(2, "Temperature"), NodeId::new(3, "Temperature"), NodeId::new(1, 5u32), NodeId::new(1, "5")]
    );
}

#[test]
fn columns_follow_tag_order_and_bits_expand_in_place_as_ints() {
    let r = rule(
        "line",
        vec![
            tag("ns=1;s=t", "temp", SeriesType::Float, false, vec![]),
            tag("ns=1;s=w", "word", SeriesType::Text, false, vec![bit(0, 1, "running"), bit(1, 1, "fault")]),
            tag("ns=1;s=d", "when", SeriesType::Time, false, vec![]),
        ],
    );
    assert_eq!(
        r.columns(),
        vec![
            ("temp".to_string(), SeriesType::Float),
            ("word".to_string(), SeriesType::Text),
            // A bit column is an Int, whatever its parent tag's type.
            ("running".to_string(), SeriesType::Int),
            ("fault".to_string(), SeriesType::Int),
            ("when".to_string(), SeriesType::Time),
        ]
    );
    let t = &r.tags[1];
    assert_eq!(
        t.columns().collect::<Vec<_>>(),
        vec![("word", SeriesType::Text), ("running", SeriesType::Int), ("fault", SeriesType::Int)]
    );
}

#[test]
fn a_bits_only_tag_contributes_no_raw_column() {
    // Polling a word purely to decompose it: the word itself is not a column.
    let r = rule("flags", vec![tag("ns=1;s=w", "", SeriesType::Float, false, vec![bit(3, 2, "mode")])]);
    assert_eq!(r.columns(), vec![("mode".to_string(), SeriesType::Int)]);
    // And a tag with neither a name nor bits contributes nothing.
    let r = rule("none", vec![tag("ns=1;s=w", "", SeriesType::Int, false, vec![])]);
    assert_eq!(r.columns(), Vec::new());
}

#[test]
fn only_tags_marked_writable_resolve_for_writing() {
    let s = spec(vec![
        rule(
            "line",
            vec![
                tag("ns=1;s=cmd", "cmd", SeriesType::Int, true, vec![]),
                tag("ns=1;s=temp", "temp", SeriesType::Float, false, vec![]),
            ],
        ),
        rule("other", vec![tag("ns=1;s=sp", "sp", SeriesType::Float, true, vec![])]),
    ]);
    assert_eq!(s.writable_tag("ns=1;s=cmd").map(|t| t.series.as_str()), Some("cmd"));
    // In a later rule too.
    assert_eq!(s.writable_tag("ns=1;s=sp").map(|t| t.ty), Some(SeriesType::Float));
    // Readable but not writable: refused at lookup, before any wire call.
    assert!(s.writable_tag("ns=1;s=temp").is_none());
    assert_eq!(s.tag("ns=1;s=temp").map(|t| t.series.as_str()), Some("temp"));
    assert_eq!(s.tag("ns=1;s=cmd").map(|t| t.series.as_str()), Some("cmd"));
    // Not in the source at all.
    assert!(s.writable_tag("ns=1;s=anything").is_none());
    assert!(s.tag("ns=1;s=anything").is_none());
    // Whitespace around the request does not defeat the check, and is no part of the key.
    assert!(s.writable_tag("  ns=1;s=cmd  ").is_some());
    assert!(s.tag("\tns=1;s=temp\n").is_some());
    // The lookup is by the configured text, exactly: another node (ids are case-sensitive), a
    // blank request, a prefix, or an extension at either end is not the tag. `ns=2;s=ns=1;s=cmd`
    // is a valid id for another node that ends with the configured text.
    for other in [
        "ns=1;s=CMD",
        "",
        "   ",
        "ns=1;s=c",
        "ns=1;s=cmdX",
        "ns=1;s=cm d",
        "ns=1",
        "ns=2;s=ns=1;s=cmd",
        "Xns=1;s=cmd",
        "ns=1;s=cmd;ns=1;s=cmd",
    ] {
        assert!(s.writable_tag(other).is_none(), "{other:?}");
        assert!(s.tag(other).is_none(), "{other:?}");
    }
    // Nor is an equivalent spelling of the same node: the stack reads `ns=01` as namespace 1,
    // but the whitelist holds text, not nodes.
    assert_eq!(parse_node_id("ns=01;s=cmd").unwrap(), parse_node_id("ns=1;s=cmd").unwrap());
    assert!(s.writable_tag("ns=01;s=cmd").is_none());
    assert!(s.tag("ns=01;s=cmd").is_none());
}

#[test]
fn a_tag_configured_in_a_non_standard_spelling_is_found_by_that_text_and_polled_once() {
    // `ns=0;i=5` and `i=5` are the same node; the stack prints it `i=5`, and `ns=01;s=cmd` as
    // `ns=1;s=cmd`. The whitelist holds the text as configured, whatever the stack prints.
    let s = spec(vec![
        rule(
            "line",
            vec![
                tag("ns=0;i=5", "five", SeriesType::Int, true, vec![]),
                tag("ns=01;s=cmd", "cmd", SeriesType::Int, true, vec![]),
            ],
        ),
        rule("again", vec![tag("i=5", "five_again", SeriesType::Int, false, vec![])]),
        // A spelling the stack never prints, after the others: still the same node.
        rule("third", vec![tag("ns=00;i=5", "five_third", SeriesType::Int, false, vec![])]),
    ]);
    assert_eq!(s.writable_tag("ns=0;i=5").map(|t| t.series.as_str()), Some("five"));
    assert_eq!(s.writable_tag("ns=01;s=cmd").map(|t| t.series.as_str()), Some("cmd"));
    assert!(s.writable_tag("ns=1;s=cmd").is_none());
    // `i=5` is configured too, but not writable: the read-only spelling does not unlock the
    // writable one, nor the other way round.
    assert!(s.writable_tag("i=5").is_none());
    assert_eq!(s.tag("i=5").map(|t| t.series.as_str()), Some("five_again"));
    // One node, however it is spelled, is polled once.
    assert_eq!(s.poll_nodes(), vec![NodeId::new(0, 5u32), NodeId::new(1, "cmd")]);
}

#[test]
fn a_tag_configured_with_surrounding_whitespace_still_resolves() {
    let mut t = tag("ns=1;s=cmd", "cmd", SeriesType::Int, true, vec![]);
    t.raw = " ns=1;s=cmd ".into();
    let s = spec(vec![rule("line", vec![t])]);
    assert!(s.writable_tag("ns=1;s=cmd").is_some());
    assert!(s.tag("ns=1;s=cmd").is_some());
}

#[test]
fn an_identity_shows_its_user_and_never_its_password() {
    assert_eq!(Auth::Anonymous.label(), "anonymous");
    let a = Auth::UserPass { username: "operator".into(), password: "s3cret-pw".into() };
    assert_eq!(a.label(), "user operator");
    assert!(!a.label().contains("s3cret"));
    // Nor does its debug form, or a spec's that carries it.
    assert_eq!(format!("{a:?}"), r#"UserPass { username: "operator", password: "<hidden>" }"#);
    assert_eq!(format!("{:?}", Auth::Anonymous), "Anonymous");
    let mut s = spec(vec![]);
    s.auth = a;
    let shown = format!("{s:?} {s:#?}");
    assert!(shown.contains("operator") && !shown.contains("s3cret"), "{shown}");
}
