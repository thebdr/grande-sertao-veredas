//! An in-process OPC UA server standing in for a small plant: the endpoint every test of the
//! OPC layer drives.
//!
//! Behind the `demo-server` feature, which core's own tests switch on (see Cargo.toml): a
//! component that only polls carries the client half alone. A standing demo plant to drive by
//! hand is not here (core builds no executable); it is the bench's.
//!
//! The node set is shaped like what the layer must handle: a **float** measurement, a packed
//! **UInt16 status word** whose bits mean things, **writable command tags** (so a pulse, set 1
//! then 0, has somewhere real to land), a subtree hung off **HasComponent** references, a folder
//! larger than one browse page and than one Read may carry, folders of exactly 64 and 65
//! children (for a server configured to serve one reference per page, [`DemoConfig`]), a child
//! linked to its folder twice, and one tag whose read is **slow**, so a request can be made to
//! wait behind it. A [`Relay`] puts a link that can go silent between a client and a plant.

use std::sync::Arc;
use std::time::Duration;

use opcua_server::address_space::{ObjectBuilder, VariableBuilder};
use opcua_server::diagnostics::NamespaceMetadata;
use opcua_server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
use opcua_server::{ServerBuilder, ServerEndpoint, ServerHandle, ServerUserToken};
use opcua_types::{DataTypeId, DataValue, NodeId, ObjectId, Variant, VariableTypeId};
use tokio::net::TcpListener;

/// The demo plant's namespace URI: the node ids below live here.
///
/// It must NOT equal the server's `application_uri`: the built-in diagnostics node manager
/// claims the application uri as its own namespace and is registered BEFORE this one, so a
/// collision gives it this namespace's index, and every read of a tag comes back
/// `BadNodeIdUnknown` with the nodes plainly present in the address space.
pub const NAMESPACE: &str = "urn:veredas:demo-plant:tags";

/// The user the demo server accepts on the username/password path. The anonymous path needs
/// none.
pub const DEMO_USER: &str = "operator";
/// Password paired with [`DEMO_USER`].
pub const DEMO_PASS: &str = "hunter2";

/// String node identifiers, so tests agree on one spelling.
pub mod tags {
    /// Float measurement (°C), a Double.
    pub const TEMPERATURE: &str = "line1.temperature";
    /// Packed UInt16 status word: bit 0 running, bit 1 fault, bits 4..6 a 3-bit mode field.
    pub const STATUS_WORD: &str = "line1.status";
    /// Writable Int32 command (the start pulse).
    pub const CMD_START: &str = "line1.cmd.start";
    /// Writable Double setpoint.
    pub const SETPOINT: &str = "line1.setpoint";
    /// A String tag: non-numeric values survive the round trip.
    pub const BATCH_ID: &str = "line1.batch";
    /// A UInt32 under `PLC_1 → DB process`, wired by **HasComponent** as some controllers keep
    /// their data blocks: browsing must follow more reference kinds than `Organizes`.
    pub const PLC_COUNTER: &str = "plc1.db_process.counter";
    /// An Int32 (always 0) whose every read takes [`super::SLOW_READ`]: a request queued behind
    /// it waits that long.
    pub const SLOW: &str = "slow.value";
}

/// How many variables the `Bulk` folder holds: more than one browse page
/// ([`crate::opc::browse::BROWSE_PAGE`]) and more than one Read call may carry
/// ([`MAX_NODES_PER_READ`]), so listing it requires following continuation points and polling it
/// requires chunked reads. `bulk.v0000` .. `bulk.v1199`, each holding its own index as an Int32.
pub const BULK_COUNT: usize = 1_200;

/// The most nodes the demo server takes in one Read call; a call with more is refused whole
/// (`BadTooManyOperations`). Under [`BULK_COUNT`], over the client's chunk, and under the stack's
/// own array limit of 1 000 (a request past that fails to decode before any cap is checked).
pub const MAX_NODES_PER_READ: usize = 800;

/// Folders whose children are numbered `pages64.v00` .. and `pages65.v00` ..: 64 and 65
/// variables, so a server serving one reference per browse page (see [`DemoConfig`]) takes
/// exactly 64 and 65 pages to list them.
pub const PAGED_FOLDERS: [(&str, usize); 2] = [("pages64", 64), ("pages65", 65)];

/// How a demo plant is started.
#[derive(Clone, Debug, Default)]
pub struct DemoConfig {
    /// The port to bind; 0 lets the OS choose.
    pub port: u16,
    /// The most references the server returns per browse page, whatever the client asks;
    /// `None` keeps the stack's default (1 000).
    pub browse_page: Option<usize>,
}

/// How long a read of [`tags::SLOW`] takes. The server thread that serves it is blocked for that
/// long, so only a multi-threaded runtime keeps serving others meanwhile.
pub const SLOW_READ: Duration = Duration::from_millis(400);

/// Why a demo plant did not start.
#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("bind 127.0.0.1:{port}: {source}")]
    Bind { port: u16, source: std::io::Error },
    #[error("local_addr: {0}")]
    LocalAddr(std::io::Error),
    #[error("build demo server: {0}")]
    Build(String),
    #[error("demo server has no simple node manager")]
    NoNodeManager,
    #[error("namespace {NAMESPACE} was not registered")]
    NoNamespace,
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    #[error("spawn demo plant thread: {0}")]
    Thread(std::io::Error),
    #[error("the demo plant thread died on startup")]
    DiedOnStartup,
    #[error("relay: {0}")]
    Relay(std::io::Error),
}

/// A running demo server on the caller's runtime: the address to point a client at, and the
/// handles to move values and shut it down.
pub struct DemoPlant {
    /// `opc.tcp://127.0.0.1:<port>/`: feed this straight to a source's configuration.
    pub endpoint_url: String,
    pub handle: ServerHandle,
    port: u16,
    namespace: u16,
    nodes: Arc<SimpleNodeManager>,
    task: tokio::task::JoinHandle<Result<(), String>>,
}

impl DemoPlant {
    /// Bind an **ephemeral** port and start serving. Ephemeral on purpose: tests run in
    /// parallel, and a fixed port makes them fight.
    pub async fn start() -> Result<DemoPlant, DemoError> {
        DemoPlant::start_on(0).await
    }

    /// Start on a specific port (0 lets the OS choose).
    pub async fn start_on(port: u16) -> Result<DemoPlant, DemoError> {
        DemoPlant::start_with(DemoConfig { port, ..DemoConfig::default() }).await
    }

    /// Start as configured.
    pub async fn start_with(config: DemoConfig) -> Result<DemoPlant, DemoError> {
        let port = config.port;
        // Bind first, then tell the server the port actually got: otherwise its advertised
        // endpoint urls disagree with reality and `connect_to_matching_endpoint` finds nothing.
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|source| DemoError::Bind { port, source })?;
        let bound = listener.local_addr().map_err(DemoError::LocalAddr)?.port();

        let mut builder = ServerBuilder::new()
            .application_name("veredas demo plant")
            .application_uri("urn:veredas:demo-plant")
            .product_uri("urn:veredas:demo-plant")
            .host("127.0.0.1")
            .port(bound)
            .discovery_urls(vec!["/".to_owned()])
            .add_user_token(DEMO_USER, ServerUserToken::user_pass(DEMO_USER, DEMO_PASS))
            .add_endpoint(
                "none",
                ServerEndpoint::new_none(
                    "/",
                    &[opcua_server::ANONYMOUS_USER_TOKEN_ID.to_string(), DEMO_USER.to_string()],
                ),
            )
            .default_endpoint("none")
            .with_node_manager(simple_node_manager(
                NamespaceMetadata { namespace_uri: NAMESPACE.to_owned(), ..Default::default() },
                "demo-plant",
            ));
        builder.limits_mut().operational.max_nodes_per_read = MAX_NODES_PER_READ;
        if let Some(page) = config.browse_page {
            builder.limits_mut().operational.max_references_per_browse_node = page;
        }
        let (server, handle) = builder.build().map_err(|e| DemoError::Build(e.to_string()))?;

        let nodes = handle.node_managers().get_of_type::<SimpleNodeManager>().ok_or(DemoError::NoNodeManager)?;
        let namespace = handle.get_namespace_index(NAMESPACE).ok_or(DemoError::NoNamespace)?;

        populate(&nodes, namespace);

        let task = tokio::spawn(server.run_with(listener));

        Ok(DemoPlant {
            endpoint_url: format!("opc.tcp://127.0.0.1:{bound}/"),
            handle,
            port: bound,
            namespace,
            nodes,
            task,
        })
    }

    /// The port it bound.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The node id of one demo tag, in the namespace index the server actually assigned.
    pub fn node_id(&self, tag: &str) -> NodeId {
        NodeId::new(self.namespace, tag)
    }

    /// Its node id in the `ns=N;s=…` text form a configuration holds.
    pub fn node_id_str(&self, tag: &str) -> String {
        format!("ns={};s={tag}", self.namespace)
    }

    /// Set a tag's value from the plant side: how the process moves in a test. A tag the plant
    /// does not hold is ignored.
    pub fn set(&self, tag: &str, value: impl Into<Variant>) {
        let id = self.node_id(tag);
        let _ = self.nodes.set_value(self.handle.subscriptions(), &id, None, DataValue::new_now(value.into()));
    }

    /// Read a tag back on the server side: what a write test asserts against, since it proves
    /// the value reached the address space rather than merely that the write returned Good.
    pub fn get(&self, tag: &str) -> Option<Variant> {
        let id = self.node_id(tag);
        let space = self.nodes.address_space().read();
        match space.find(&id)? {
            opcua_server::address_space::NodeType::Variable(v) => {
                v.value(
                    opcua_types::TimestampsToReturn::Neither,
                    &opcua_types::NumericRange::None,
                    &opcua_types::DataEncoding::Binary,
                    0.0,
                )
                .value
            }
            _ => None,
        }
    }

    /// Stop the server and wait (at most 5 s) for its task to wind down.
    ///
    /// This stops the server ACCEPTING work; it does not guarantee an established session sees
    /// its connection drop. A test that needs a server to genuinely vanish wants
    /// [`DetachedPlant`].
    pub async fn stop(self) {
        self.handle.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

/// A demo plant on its OWN thread and runtime, killable outright.
///
/// Cancelling a server leaves live connections alive, which quietly let a reconnect test pass
/// without any reconnect happening. Dropping a whole runtime does not: every connection task
/// dies with it and the listening socket closes, which is what "the plant went away" looks like
/// from a client.
pub struct DetachedPlant {
    /// `opc.tcp://127.0.0.1:<port>/`.
    pub endpoint_url: String,
    port: u16,
    /// The namespace index the plant's tags actually landed on: asked for, never assumed.
    namespace: u16,
    setter: std::sync::mpsc::Sender<(String, Variant)>,
    getter: std::sync::mpsc::Sender<(String, std::sync::mpsc::Sender<Option<Variant>>)>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

type Ready = Result<(String, u16, u16), DemoError>;
type GetRequest = (String, std::sync::mpsc::Sender<Option<Variant>>);

impl DetachedPlant {
    /// Start on `port` (0 = any). Blocks until the server is listening.
    pub fn start(port: u16) -> Result<DetachedPlant, DemoError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Ready>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let (set_tx, set_rx) = std::sync::mpsc::channel::<(String, Variant)>();
        let (get_tx, get_rx) = std::sync::mpsc::channel::<GetRequest>();

        let thread = std::thread::Builder::new()
            .name("demo-plant".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(DemoError::Runtime(e)));
                        return;
                    }
                };
                rt.block_on(async move {
                    let plant = match DemoPlant::start_on(port).await {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    if ready_tx.send(Ok((plant.endpoint_url.clone(), plant.port, plant.namespace))).is_err() {
                        return;
                    }
                    // Apply set and get requests until shutdown; polling the std channels from
                    // async is fine at this cadence.
                    let mut shutdown = shutdown_rx;
                    loop {
                        tokio::select! {
                            _ = &mut shutdown => break,
                            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                                while let Ok((tag, value)) = set_rx.try_recv() {
                                    plant.set(&tag, value);
                                }
                                while let Ok((tag, reply)) = get_rx.try_recv() {
                                    let _ = reply.send(plant.get(&tag));
                                }
                            }
                        }
                    }
                });
                // Dropping `rt` here kills every connection task outright.
            })
            .map_err(DemoError::Thread)?;

        let (endpoint_url, bound, namespace) = ready_rx.recv().map_err(|_| DemoError::DiedOnStartup)??;
        Ok(DetachedPlant {
            endpoint_url,
            port: bound,
            namespace,
            setter: set_tx,
            getter: get_tx,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        })
    }

    /// The port it bound, so a restart can claim the same endpoint.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Set a tag from the plant side; applied within about 20 ms.
    pub fn set(&self, tag: &str, value: impl Into<Variant>) {
        let _ = self.setter.send((tag.to_string(), value.into()));
    }

    /// Read a tag on the SERVER side, waiting at most 5 s for the plant's thread; `None` when
    /// the tag holds no value or the plant did not answer.
    pub fn get(&self, tag: &str) -> Option<Variant> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.getter.send((tag.to_string(), tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(5)).ok()?
    }

    /// Its node id in `ns=N;s=…` form, in the index the server actually assigned.
    pub fn node_id_str(&self, tag: &str) -> String {
        format!("ns={};s={tag}", self.namespace)
    }

    /// Kill it: signal, join, and let the runtime's drop take the connections with it. Blocks
    /// until the port is released.
    pub fn kill(mut self) {
        self.shutdown_and_join();
    }

    fn shutdown_and_join(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for DetachedPlant {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

/// A TCP relay in front of a plant that can be frozen: a link that goes silent without closing,
/// the way a stalled switch or a wedged peer looks from a client.
///
/// It listens on `127.0.0.2` at the plant's own port and forwards to `127.0.0.1` at that port, so
/// a client given [`Relay::endpoint_url`] matches the plant's advertised endpoint (the port and
/// path agree) and still goes through the relay. Frozen, it forwards nothing in either
/// direction and holds what it has read until thawed; both sockets stay open. Dropping it closes
/// every connection it carries.
pub struct Relay {
    /// `opc.tcp://127.0.0.2:<port>/`.
    pub endpoint_url: String,
    frozen: Arc<std::sync::atomic::AtomicBool>,
    closed: Arc<std::sync::atomic::AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl Relay {
    /// Relay `127.0.0.2:<port>` to `127.0.0.1:<port>`.
    pub fn start(port: u16) -> Result<Relay, DemoError> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind(("127.0.0.2", port)).map_err(DemoError::Relay)?;
        listener.set_nonblocking(true).map_err(DemoError::Relay)?;
        let frozen = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let (f, c) = (Arc::clone(&frozen), Arc::clone(&closed));
        let accept = std::thread::Builder::new()
            .name("demo-relay".into())
            .spawn(move || {
                while !c.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((client, _)) => {
                            let Ok(server) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
                                continue;
                            };
                            let _ = client.set_nonblocking(false);
                            for (from, to) in [(&client, &server), (&server, &client)] {
                                let (Ok(from), Ok(to)) = (from.try_clone(), to.try_clone()) else { continue };
                                let (f, c) = (Arc::clone(&f), Arc::clone(&c));
                                let _ = std::thread::Builder::new().name("demo-relay-pump".into()).spawn(move || pump(from, to, &f, &c));
                            }
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            })
            .map_err(DemoError::Relay)?;
        Ok(Relay { endpoint_url: format!("opc.tcp://127.0.0.2:{port}/"), frozen, closed, accept: Some(accept) })
    }

    /// Stop forwarding, both ways, keeping every connection open.
    pub fn freeze(&self) {
        self.frozen.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Forward again, what was held first.
    pub fn thaw(&self) {
        self.frozen.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

/// Copy one direction of a relayed connection, holding each chunk while frozen, until either
/// side closes or the relay is dropped.
fn pump(
    mut from: std::net::TcpStream,
    mut to: std::net::TcpStream,
    frozen: &std::sync::atomic::AtomicBool,
    closed: &std::sync::atomic::AtomicBool,
) {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    let _ = from.set_read_timeout(Some(Duration::from_millis(20)));
    let mut buf = [0u8; 16 * 1024];
    while !closed.load(Ordering::SeqCst) {
        let n = match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => break,
        };
        while frozen.load(Ordering::SeqCst) && !closed.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if closed.load(Ordering::SeqCst) || to.write_all(&buf[..n]).is_err() {
            break;
        }
    }
    let _ = from.shutdown(std::net::Shutdown::Both);
    let _ = to.shutdown(std::net::Shutdown::Both);
}

/// Build the plant's node set.
fn populate(nodes: &SimpleNodeManager, ns: u16) {
    let mut space = nodes.address_space().write();

    let folder = NodeId::new(ns, "line1");
    ObjectBuilder::new(&folder, "Line1", "Line 1").organized_by(ObjectId::ObjectsFolder).insert(&mut *space);

    // `has_type_definition` is not decoration: a browsing client reads the type definition to
    // tell a plain variable from a structured one, and a node without it browses with a null one.
    let variable = |id: &str, browse: &str, display: &str, ty: DataTypeId| {
        VariableBuilder::new(&NodeId::new(ns, id), browse, display)
            .data_type(ty)
            .has_type_definition(VariableTypeId::BaseDataVariableType)
    };
    variable(tags::TEMPERATURE, "Temperature", "Temperature", DataTypeId::Double)
        .value(21.5f64)
        .organized_by(folder.clone())
        .insert(&mut *space);
    variable(tags::STATUS_WORD, "StatusWord", "Status word", DataTypeId::UInt16)
        .value(0u16)
        .organized_by(folder.clone())
        .insert(&mut *space);
    variable(tags::CMD_START, "CmdStart", "Start command", DataTypeId::Int32)
        .value(0i32)
        .writable()
        .organized_by(folder.clone())
        .insert(&mut *space);
    variable(tags::SETPOINT, "Setpoint", "Setpoint", DataTypeId::Double)
        .value(0.0f64)
        .writable()
        .organized_by(folder.clone())
        .insert(&mut *space);
    variable(tags::BATCH_ID, "BatchId", "Batch id", DataTypeId::String)
        .value("B-000")
        .organized_by(folder)
        .insert(&mut *space);

    // A controller's structure on HasComponent references (PLC_1 → data block → tag), not the
    // Organizes used above: a browse filtered to Organizes alone reads this subtree as empty,
    // which is how such a controller looked in the field while other clients showed it fine.
    let plc = NodeId::new(ns, "plc1");
    ObjectBuilder::new(&plc, "PLC_1", "PLC_1").organized_by(ObjectId::ObjectsFolder).insert(&mut *space);
    let db = NodeId::new(ns, "plc1.db_process");
    ObjectBuilder::new(&db, "DbProcess", "DB process").component_of(plc).insert(&mut *space);
    variable(tags::PLC_COUNTER, "Counter", "Counter", DataTypeId::UInt32)
        .value(0u32)
        .component_of(db)
        .insert(&mut *space);

    // More children than one browse page and more than one Read call may carry.
    let bulk = NodeId::new(ns, "bulk");
    ObjectBuilder::new(&bulk, "Bulk", "Bulk").organized_by(ObjectId::ObjectsFolder).insert(&mut *space);
    for i in 0..BULK_COUNT {
        let name = format!("V{i:04}");
        variable(&format!("bulk.v{i:04}"), &name, &name, DataTypeId::Int32)
            .value(i as i32)
            .organized_by(bulk.clone())
            .insert(&mut *space);
    }

    // Folders of exactly 64 and 65 children, for the browser's page ceiling.
    for (name, count) in PAGED_FOLDERS {
        let paged = NodeId::new(ns, name);
        ObjectBuilder::new(&paged, name, name).organized_by(ObjectId::ObjectsFolder).insert(&mut *space);
        for i in 0..count {
            let child = format!("{name}.v{i:02}");
            variable(&child, &child, &child, DataTypeId::Int32)
                .value(i as i32)
                .organized_by(paged.clone())
                .insert(&mut *space);
        }
    }

    // One child linked to its folder twice, by Organizes and by HasComponent: a browse sees two
    // references to the same node.
    let twins = NodeId::new(ns, "twins");
    ObjectBuilder::new(&twins, "Twins", "Twins").organized_by(ObjectId::ObjectsFolder).insert(&mut *space);
    variable("twins.only", "Only", "Only", DataTypeId::Int32)
        .value(1i32)
        .organized_by(twins.clone())
        .component_of(twins)
        .insert(&mut *space);

    // The slow tag, in a folder of its own.
    let slow = NodeId::new(ns, "slow");
    ObjectBuilder::new(&slow, "Slow", "Slow").organized_by(ObjectId::ObjectsFolder).insert(&mut *space);
    variable(tags::SLOW, "SlowValue", "Slow value", DataTypeId::Int32)
        .value(0i32)
        .organized_by(slow)
        .insert(&mut *space);
    drop(space);
    nodes.inner().add_read_callback(NodeId::new(ns, tags::SLOW), |_, _, _| {
        std::thread::sleep(SLOW_READ);
        Ok(DataValue::new_now(Variant::Int32(0)))
    });
}
