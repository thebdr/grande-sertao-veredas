//! The address-space browser: walking a server's tree, and reading one node's attributes.
//!
//! A short-lived connection of its own, on a thread of its own, answering one request at a
//! time. Deliberately NOT a poll worker's session: browsing happens while a source is being
//! configured, before any worker exists, and a browse that stalled must never stall live data.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use opcua_client::Session;
use opcua_types::{
    AttributeId, BrowseDescription, BrowseDirection, BrowseResult, BrowseResultMask, Identifier,
    NodeClass, NodeId, ObjectId, ReadValueId, ReferenceTypeId, StatusCode, TimestampsToReturn,
    Variant,
};

use crate::SeriesType;
use crate::opc::error::{Call, OpcError};
use crate::opc::spec::{Auth, PollSpec, parse_node_id};
use crate::opc::worker::{EventLoop, STOP_CLOSE_TIMEOUT, close_quietly, connect};

/// How long one browse or attribute call may take before the browser gives up on it.
pub const BROWSE_TIMEOUT: Duration = Duration::from_secs(15);

/// Children asked for per Browse/BrowseNext round. A field controller typically serves far fewer
/// per response and hands out a continuation point for the rest, which the browse follows.
pub const BROWSE_PAGE: u32 = 1_000;

/// Ceiling on the pages read under ONE node, the first included (64 pages of up to
/// [`BROWSE_PAGE`]). A node with more is refused: something is wrong, and an unbounded loop
/// against a confused server is how workers wedge.
pub const MAX_BROWSE_PAGES: usize = 64;

/// How often the browser's thread looks for a request or a stop.
const IDLE_CHECK: Duration = Duration::from_millis(30);

/// One child in the address space.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowsedNode {
    /// `ns=2;s=…`: the text a tag's configuration holds.
    pub node_id: String,
    /// What the server calls it (its browse name when it gives no display name).
    pub display_name: String,
    pub browse_name: String,
    /// A variable can be polled as a tag.
    pub is_variable: bool,
    /// Whether it may have children worth expanding.
    pub is_container: bool,
    /// The server's DataType for a variable, as a series type, so a tag's type can be filled in
    /// without knowing that a UInt16 is an `int`.
    pub series_type: Option<SeriesType>,
    /// The DataType's own node id, as text.
    pub data_type: String,
}

/// A request to the browser's thread.
enum Job {
    /// One node's children; `None` is the Objects folder (the root).
    Browse(Option<String>),
    /// One node's attribute set.
    Details(String),
}

/// What the browser sends back.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowseEvent {
    Connected,
    /// One node's children, keyed by the node that was browsed (`None` for the root) so a late
    /// reply cannot fill the wrong branch.
    Children { parent: Option<String>, nodes: Vec<BrowsedNode> },
    /// One node's attributes as ordered name → rendered-value rows, keyed by the node read.
    Details { node_id: String, rows: Vec<(String, String)> },
    /// A connection or a request failed. After a failed connection nothing more arrives.
    Failed(OpcError),
}

/// A live browse connection: send requests, receive [`BrowseEvent`]s.
pub struct Browser {
    jobs: Sender<Job>,
    pub events: Receiver<BrowseEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Browser {
    /// Connect and start serving requests. Returns at once; the first event says whether the
    /// connection worked. Requests sent meanwhile are served once it has.
    pub fn connect(endpoint_url: &str, auth: Auth) -> Browser {
        let (jobs_tx, jobs_rx) = std::sync::mpsc::channel();
        let (ev_tx, ev_rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let spec = PollSpec {
            source_id: "browse".into(),
            display_name: "browse".into(),
            endpoint_url: endpoint_url.to_string(),
            auth,
            poll: Duration::from_secs(1),
            rules: Vec::new(),
        };
        let thread_events = ev_tx.clone();
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("opc-browse".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = thread_events.send(BrowseEvent::Failed(OpcError::Runtime(e.to_string())));
                        return;
                    }
                };
                rt.block_on(run(spec, jobs_rx, thread_events, thread_stop));
                // Never wait for a blocking task (a host name being resolved): the drop is now.
                rt.shutdown_background();
            });
        let thread = match thread {
            Ok(t) => Some(t),
            Err(e) => {
                let _ = ev_tx.send(BrowseEvent::Failed(OpcError::Thread(e.to_string())));
                None
            }
        };
        Browser { jobs: jobs_tx, events: ev_rx, stop, thread }
    }

    /// Ask for a node's children (`None` is the root).
    pub fn browse(&self, node_id: Option<String>) {
        let _ = self.jobs.send(Job::Browse(node_id));
    }

    /// Ask for a node's attribute details.
    pub fn details(&self, node_id: String) {
        let _ = self.jobs.send(Job::Details(node_id));
    }
}

impl Drop for Browser {
    /// Stops the thread and waits for it: whatever it was doing (connecting, browsing a long
    /// folder) is abandoned within a few tens of milliseconds, and its session closed.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The DataType node id → the series type an operator most likely wants.
///
/// Numeric ids in namespace 0 are the standard OPC UA built-ins, so this is a stable mapping
/// rather than a guess; anything else is `None`, left for the operator to choose.
pub fn series_type_of(data_type: &NodeId) -> Option<SeriesType> {
    if data_type.namespace != 0 {
        return None;
    }
    let Identifier::Numeric(id) = &data_type.identifier else {
        return None;
    };
    Some(match id {
        // Boolean, SByte, Byte, Int16, UInt16, Int32, UInt32, Int64, UInt64
        1..=9 => SeriesType::Int,
        // Float, Double
        10 | 11 => SeriesType::Float,
        // String, Guid, ByteString, XmlElement, NodeId, LocalizedText
        12 | 14 | 15 | 16 | 17 | 21 => SeriesType::Text,
        // DateTime
        13 => SeriesType::Time,
        _ => return None,
    })
}

fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst)
}

async fn wait_for_stop(stop: &AtomicBool) {
    while !stopped(stop) {
        tokio::time::sleep(IDLE_CHECK).await;
    }
}

async fn run(spec: PollSpec, jobs: Receiver<Job>, events: Sender<BrowseEvent>, stop: Arc<AtomicBool>) {
    let connected = tokio::select! {
        biased;
        _ = wait_for_stop(&stop) => return,
        r = connect(&spec) => r,
    };
    let (session, event_loop): (Arc<Session>, EventLoop) = match connected {
        Ok(v) => v,
        Err(e) => {
            let _ = events.send(BrowseEvent::Failed(e));
            return;
        }
    };
    if events.send(BrowseEvent::Connected).is_ok() {
        serve(&session, &jobs, &events, &stop).await;
    }
    // A short CloseSession, so the server frees the session's slot, never a wait.
    close_quietly(&session, event_loop, STOP_CLOSE_TIMEOUT).await;
}

/// Answer requests until the browser is dropped.
async fn serve(session: &Session, jobs: &Receiver<Job>, events: &Sender<BrowseEvent>, stop: &AtomicBool) {
    while !stopped(stop) {
        // The request channel is sync: look without blocking the runtime.
        let job = match jobs.try_recv() {
            Ok(j) => j,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                tokio::time::sleep(IDLE_CHECK).await;
                continue;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
        };
        let answer = tokio::select! {
            biased;
            _ = wait_for_stop(stop) => return,
            a = answer(session, job) => a,
        };
        if events.send(answer).is_err() {
            return;
        }
    }
}

async fn answer(session: &Session, job: Job) -> BrowseEvent {
    match job {
        Job::Browse(parent) => {
            let node = match &parent {
                None => Ok(NodeId::from(ObjectId::ObjectsFolder)),
                Some(raw) => parse_node_id(raw).map_err(OpcError::from),
            };
            match node {
                Ok(node) => match browse_children(session, node).await {
                    Ok(nodes) => BrowseEvent::Children { parent, nodes },
                    Err(e) => BrowseEvent::Failed(e),
                },
                Err(e) => BrowseEvent::Failed(e),
            }
        }
        Job::Details(raw) => match parse_node_id(&raw) {
            Ok(node) => match read_details(session, node).await {
                Ok(rows) => BrowseEvent::Details { node_id: raw, rows },
                Err(e) => BrowseEvent::Failed(e),
            },
            Err(e) => BrowseEvent::Failed(e.into()),
        },
    }
}

/// One node's hierarchical children: ALL of them, following continuation points, refusing
/// when the server refuses.
async fn browse_children(session: &Session, node: NodeId) -> Result<Vec<BrowsedNode>, OpcError> {
    let describe = BrowseDescription {
        node_id: node,
        browse_direction: BrowseDirection::Forward,
        // Every hierarchical reference, subtypes included: a controller may hang its structure
        // off HasComponent rather than Organizes, and filtering to one kind shows it empty.
        reference_type_id: ReferenceTypeId::HierarchicalReferences.into(),
        include_subtypes: true,
        // 0 = "no class filter", the standard's own everything value. Strict field servers
        // validate this field where lax stacks do not; the less it asks them to interpret, the
        // better.
        node_class_mask: 0,
        result_mask: BrowseResultMask::All as u32,
    };
    let results = tokio::time::timeout(BROWSE_TIMEOUT, session.browse(&[describe], BROWSE_PAGE, None))
        .await
        .map_err(|_| OpcError::TimedOut(Call::Browse))?
        .map_err(|e| OpcError::failed(Call::Browse, &e))?;
    let first = results.into_iter().next().ok_or(OpcError::NoResult { call: Call::Browse })?;
    if let Some(refusal) = result_refusal(&first, Call::Browse) {
        return Err(refusal);
    }
    let mut refs = first.references.unwrap_or_default();

    // Follow continuation points: a controller hands out small pages, and a client that stops
    // after the first sees a truncated folder.
    let mut cp = first.continuation_point;
    let mut pages = 1;
    while !cp.is_null_or_empty() {
        if pages == MAX_BROWSE_PAGES {
            // Release the point server-side rather than leaking it.
            let _ = tokio::time::timeout(BROWSE_TIMEOUT, session.browse_next(true, &[cp])).await;
            return Err(OpcError::EndlessBrowse { pages });
        }
        pages += 1;
        let next = tokio::time::timeout(BROWSE_TIMEOUT, session.browse_next(false, &[cp]))
            .await
            .map_err(|_| OpcError::TimedOut(Call::BrowseNext))?
            .map_err(|e| OpcError::failed(Call::BrowseNext, &e))?;
        let r = next.into_iter().next().ok_or(OpcError::NoResult { call: Call::BrowseNext })?;
        if let Some(refusal) = result_refusal(&r, Call::BrowseNext) {
            return Err(refusal);
        }
        refs.extend(r.references.unwrap_or_default());
        cp = r.continuation_point;
    }

    let mut out: Vec<BrowsedNode> = refs
        .into_iter()
        .map(|r| BrowsedNode {
            node_id: r.node_id.node_id.to_string(),
            display_name: match r.display_name.text.as_ref() {
                "" => r.browse_name.name.as_ref().to_string(),
                t => t.to_string(),
            },
            browse_name: r.browse_name.name.as_ref().to_string(),
            is_variable: r.node_class == NodeClass::Variable,
            // Objects hold things; a Variable can too (a structured tag), and a server that
            // reports no node class gets the benefit of the doubt. Only classes KNOWN to be
            // leaves (methods, types, views) are not expandable.
            is_container: matches!(
                r.node_class,
                NodeClass::Object | NodeClass::Variable | NodeClass::Unspecified
            ),
            series_type: None,
            data_type: String::new(),
        })
        .collect();
    // A server may list the same child twice through two reference types: sort by node id so
    // the twins are adjacent, drop them, then order by name.
    out.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    out.dedup_by(|a, b| a.node_id == b.node_id);
    out.sort_by(|a, b| a.display_name.cmp(&b.display_name).then_with(|| a.node_id.cmp(&b.node_id)));

    // Fill in each variable's DataType, so a tag's type can be prefilled. Best effort: a server
    // that refuses the read still gets its children listed.
    let (slots, ids): (Vec<usize>, Vec<NodeId>) = (0..out.len())
        .filter(|&i| out[i].is_variable)
        .filter_map(|i| parse_node_id(&out[i].node_id).ok().map(|id| (i, id)))
        .unzip();
    if !ids.is_empty()
        && let Ok(types) = read_data_types(session, &ids).await
    {
        for (&i, dt) in slots.iter().zip(types) {
            out[i].series_type = dt.as_ref().and_then(series_type_of);
            out[i].data_type = dt.map(|d| d.to_string()).unwrap_or_default();
        }
    }
    Ok(out)
}

/// Read the DataType attribute of several variables, in chunks like a poll's read, each bounded.
async fn read_data_types(session: &Session, nodes: &[NodeId]) -> Result<Vec<Option<NodeId>>, OpcError> {
    let mut out = Vec::with_capacity(nodes.len());
    for chunk in nodes.chunks(crate::opc::worker::MAX_NODES_PER_READ) {
        let to_read: Vec<ReadValueId> = chunk
            .iter()
            .map(|n| ReadValueId {
                node_id: n.clone(),
                attribute_id: AttributeId::DataType as u32,
                ..Default::default()
            })
            .collect();
        let values = tokio::time::timeout(BROWSE_TIMEOUT, session.read(&to_read, TimestampsToReturn::Neither, 0.0))
            .await
            .map_err(|_| OpcError::TimedOut(Call::ReadDataTypes))?
            .map_err(|e| OpcError::failed(Call::ReadDataTypes, &e))?;
        out.extend((0..chunk.len()).map(|i| match values.get(i).and_then(|v| v.value.as_ref()) {
            Some(Variant::NodeId(id)) => Some((**id).clone()),
            _ => None,
        }));
    }
    Ok(out)
}

/// The attribute set the details read returns, in this order. Reading an attribute a node
/// class does not have is legal: the server answers `BadAttributeIdInvalid` for that entry and
/// the row is skipped.
pub const DETAIL_ATTRS: &[(AttributeId, &str)] = &[
    (AttributeId::NodeClass, "NodeClass"),
    (AttributeId::BrowseName, "BrowseName"),
    (AttributeId::DisplayName, "DisplayName"),
    (AttributeId::Description, "Description"),
    (AttributeId::DataType, "DataType"),
    (AttributeId::ValueRank, "ValueRank"),
    (AttributeId::ArrayDimensions, "ArrayDimensions"),
    (AttributeId::AccessLevel, "AccessLevel"),
    (AttributeId::UserAccessLevel, "UserAccessLevel"),
    (AttributeId::MinimumSamplingInterval, "MinimumSamplingInterval"),
    (AttributeId::Historizing, "Historizing"),
    (AttributeId::EventNotifier, "EventNotifier"),
    (AttributeId::Value, "Value"),
];

/// Read one node's attributes and render them as name → value rows, the node id first.
async fn read_details(session: &Session, node: NodeId) -> Result<Vec<(String, String)>, OpcError> {
    let to_read: Vec<ReadValueId> = DETAIL_ATTRS
        .iter()
        .map(|(attr, _)| ReadValueId {
            node_id: node.clone(),
            attribute_id: *attr as u32,
            ..Default::default()
        })
        .collect();
    let values = tokio::time::timeout(BROWSE_TIMEOUT, session.read(&to_read, TimestampsToReturn::Source, 0.0))
        .await
        .map_err(|_| OpcError::TimedOut(Call::ReadDetails))?
        .map_err(|e| OpcError::failed(Call::ReadDetails, &e))?;

    // The identity row comes from what was asked, not from a read.
    let mut rows: Vec<(String, String)> = vec![("NodeId".into(), node.to_string())];
    for ((attr, name), value) in DETAIL_ATTRS.iter().zip(values) {
        let status = value.status.unwrap_or(StatusCode::Good);
        if status.is_bad() {
            // Not-an-attribute-of-this-class is expected noise; any OTHER refusal is
            // information the operator should see.
            if status != StatusCode::BadAttributeIdInvalid {
                rows.push((name.to_string(), format!("({status})")));
            }
            continue;
        }
        let Some(v) = value.value else {
            continue;
        };
        let rendered = match (attr, v) {
            (AttributeId::NodeClass, Variant::Int32(n)) => node_class_text(n).to_string(),
            (AttributeId::DataType, Variant::NodeId(id)) => data_type_text(&id),
            (AttributeId::ValueRank, Variant::Int32(r)) => value_rank_text(r),
            (AttributeId::AccessLevel | AttributeId::UserAccessLevel, Variant::Byte(bits)) => {
                access_level_text(bits)
            }
            (AttributeId::MinimumSamplingInterval, Variant::Double(ms)) if ms < 0.0 => {
                "indeterminate".to_string()
            }
            (AttributeId::MinimumSamplingInterval, Variant::Double(ms)) => format!("{ms} ms"),
            (AttributeId::Value, v) => {
                let mut s = variant_brief(&v);
                if let Some(ts) = value.source_timestamp {
                    s.push_str(&format!("   (source {ts})"));
                }
                s
            }
            (_, v) => variant_brief(&v),
        };
        rows.push((name.to_string(), rendered));
    }
    Ok(rows)
}

/// A browse result carries its own verdict, apart from the call's: a bad status with no
/// references is the server REFUSING, not an empty folder. Swallowing it showed a real strict
/// controller as an empty tree; the refusal names the server's own status, so it says exactly
/// what to fix.
pub fn result_refusal(r: &BrowseResult, call: Call) -> Option<OpcError> {
    r.status_code.is_bad().then_some(OpcError::Refused { call, status: r.status_code })
}

/// A DataType node id with the standard built-in's name and its series mapping beside it:
/// `i=11` alone helps nobody.
pub fn data_type_text(id: &NodeId) -> String {
    let series = series_type_of(id).map(|t| format!(" → {t} series")).unwrap_or_default();
    match (&id.namespace, &id.identifier) {
        (0, Identifier::Numeric(n)) => match builtin_type_name(*n) {
            Some(name) => format!("{name} ({id}){series}"),
            None => format!("{id}{series}"),
        },
        _ => format!("{id}{series}"),
    }
}

/// Names for the OPC UA built-in DataTypes (namespace 0, numeric ids 1 to 25).
fn builtin_type_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "Boolean",
        2 => "SByte",
        3 => "Byte",
        4 => "Int16",
        5 => "UInt16",
        6 => "Int32",
        7 => "UInt32",
        8 => "Int64",
        9 => "UInt64",
        10 => "Float",
        11 => "Double",
        12 => "String",
        13 => "DateTime",
        14 => "Guid",
        15 => "ByteString",
        16 => "XmlElement",
        17 => "NodeId",
        18 => "ExpandedNodeId",
        19 => "StatusCode",
        20 => "QualifiedName",
        21 => "LocalizedText",
        22 => "Structure",
        23 => "DataValue",
        24 => "BaseDataType",
        25 => "DiagnosticInfo",
        _ => return None,
    })
}

/// The AccessLevel byte as words: the four bits an operator cares about, then the byte.
pub fn access_level_text(bits: u8) -> String {
    let parts: Vec<&str> = [(0x01, "read"), (0x02, "write"), (0x04, "history read"), (0x08, "history write")]
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, word)| *word)
        .collect();
    if parts.is_empty() {
        return format!("none (0x{bits:02X})");
    }
    format!("{} (0x{bits:02X})", parts.join(", "))
}

/// ValueRank's numbers as words.
pub fn value_rank_text(rank: i32) -> String {
    match rank {
        -1 => "scalar".to_string(),
        -2 => "scalar or array".to_string(),
        -3 => "scalar or one-dimensional array".to_string(),
        0 => "array (any dimensions)".to_string(),
        n if n > 0 => format!("{n}-dimensional array"),
        other => other.to_string(),
    }
}

/// The NodeClass attribute's integer as its name.
pub fn node_class_text(v: i32) -> &'static str {
    match v {
        1 => "Object",
        2 => "Variable",
        4 => "Method",
        8 => "ObjectType",
        16 => "VariableType",
        32 => "ReferenceType",
        64 => "DataType",
        128 => "View",
        _ => "Unspecified",
    }
}

/// The most characters [`variant_brief`] shows before it cuts with `…`.
pub const BRIEF_MAX: usize = 160;

/// A compact rendering of a Variant: one line (a control character reads as a space), at most
/// [`BRIEF_MAX`] characters and then `…`, an array as its length and first four items.
pub fn variant_brief(v: &Variant) -> String {
    let s = match v {
        Variant::Empty => "(empty)".to_string(),
        Variant::Boolean(b) => b.to_string(),
        Variant::SByte(n) => n.to_string(),
        Variant::Byte(n) => n.to_string(),
        Variant::Int16(n) => n.to_string(),
        Variant::UInt16(n) => n.to_string(),
        Variant::Int32(n) => n.to_string(),
        Variant::UInt32(n) => n.to_string(),
        Variant::Int64(n) => n.to_string(),
        Variant::UInt64(n) => n.to_string(),
        Variant::Float(n) => n.to_string(),
        Variant::Double(n) => n.to_string(),
        Variant::String(s) => s.as_ref().to_string(),
        Variant::DateTime(dt) => dt.to_string(),
        Variant::NodeId(id) => id.to_string(),
        Variant::LocalizedText(t) => t.text.as_ref().to_string(),
        Variant::QualifiedName(q) => format!("{}:{}", q.namespace_index, q.name.as_ref()),
        Variant::StatusCode(c) => c.to_string(),
        Variant::Array(a) => {
            let head: Vec<String> = a.values.iter().take(4).map(variant_brief).collect();
            let ellipsis = if a.values.len() > 4 { ", …" } else { "" };
            format!("[{} items] {}{}", a.values.len(), head.join(", "), ellipsis)
        }
        other => format!("{other:?}"),
    };
    let mut out: String = s.chars().take(BRIEF_MAX).map(crate::opc::flat).collect();
    if s.chars().nth(BRIEF_MAX).is_some() {
        out.push('…');
    }
    out
}
