//! An OPC UA server for a component to serve its values with: one folder of typed variables on a
//! real endpoint, each client's write checked and handed to the caller (contract: the OPC UA
//! server clause; `docs/core-plan.md`, "The OPC UA server").
//!
//! Behind the `opc-server` feature. Controlador's embedded portal serves OPC UA with it, and the
//! bench's plant server stands on it (maquinar keeps its draws, ramps and reactions). Core names
//! no component: the application's URI and name, the data namespace and the folder are the
//! caller's ([`ServerSpec`]).
//!
//! The server runs on a thread and a current-thread runtime of its own ([`Server::start`]), bound
//! before it is built so it advertises the port it really got. Dropping the handle drops the
//! runtime, which takes every client connection with it and frees the port. A caller's own loop
//! can run on that thread ([`Server::runtime`]), so it keeps the server's timing.
//!
//! The write guard: the stack's own validation takes any scalar into any typed variable, so a
//! client's write of another wire type is answered `BadTypeMismatch` here, as a controller
//! answers it, and a write with an index range `BadIndexRangeInvalid`. A right one lands as soon
//! as the address space's lock is free, then is handed to the caller ([`Server::writes`]) with the
//! variable's id, in the order the writes landed. A value the caller sets through the handle is
//! never handed back as a client's write.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opcua_server::address_space::{AccessLevel, NodeType, ObjectBuilder, VariableBuilder};
use opcua_server::diagnostics::NamespaceMetadata;
use opcua_server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
use opcua_server::{ServerBuilder, ServerEndpoint, ServerHandle, ServerUserToken};
use opcua_types::{
    DataEncoding, DataTypeId, DataValue, NodeId, NumericRange, ObjectId, StatusCode,
    TimestampsToReturn, VariableTypeId, Variant,
};
use serde::Deserialize;
use tokio::net::TcpListener;

/// How many client writes wait for the caller at most; past that, a new one is counted in
/// [`ServerStatus::writes_dropped`] and not kept (the value has landed all the same).
pub const WRITES_KEPT: usize = 4096;

/// OPC UA's own namespace, index 0 on every server.
const OPC_UA_NAMESPACE: &str = "http://opcfoundation.org/UA/";

/// What a server serves and how.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerSpec {
    /// The host the server binds and advertises: a client matches the endpoint URL the server
    /// advertises, so this is the host clients reach, not `0.0.0.0`.
    pub bind: String,
    /// The port; 0 lets the system choose, and the server advertises the one it got.
    pub port: u16,
    /// The application's URI. It must differ from [`ServerSpec::namespace`].
    pub application_uri: String,
    /// The application's name, as a client's discovery shows it.
    pub application_name: String,
    /// The data namespace's URI: every variable's node id is `ns=<its index>;s=<id>`.
    pub namespace: String,
    /// The folder every variable hangs under, organized by the Objects folder.
    pub folder: Folder,
    /// A user the server accepts on the username and password path; anonymous is always
    /// accepted.
    pub user: Option<Credentials>,
    /// The most sessions held at once; the stack's own limit when absent.
    pub max_sessions: Option<usize>,
    /// The most nodes one Read may carry, a call with more refused whole; the stack's own limit
    /// when absent.
    pub max_nodes_per_read: Option<usize>,
    pub variables: Vec<Variable>,
}

/// The folder: its node id's string identifier in the data namespace, and its browse name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
}

/// A username and password. The password shows in no `Debug` form.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub name: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("name", &self.name)
            .field("password", &"<hidden>")
            .finish()
    }
}

/// One variable.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    /// The string identifier in the data namespace.
    pub id: String,
    /// The browse and display name; the id when absent.
    pub name: Option<String>,
    pub ty: VarType,
    pub access: Access,
    /// The initial value, of the variable's wire type ([`parse_value`] makes one from text).
    pub value: Variant,
}

/// A variable's wire type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarType {
    /// An Int32.
    Int,
    /// A Double.
    Float,
    /// A Boolean.
    Bool,
    /// A String.
    Text,
}

impl VarType {
    pub fn label(self) -> &'static str {
        match self {
            VarType::Int => "int",
            VarType::Float => "float",
            VarType::Bool => "bool",
            VarType::Text => "text",
        }
    }

    /// Whether a value is of this wire type.
    pub fn holds(self, value: &Variant) -> bool {
        matches!(
            (self, value),
            (VarType::Int, Variant::Int32(_))
                | (VarType::Float, Variant::Double(_))
                | (VarType::Bool, Variant::Boolean(_))
                | (VarType::Text, Variant::String(_))
        )
    }
}

/// What a client may do with a variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    #[default]
    Read,
    /// Written and never read back by a client, as a command a controller accepts and does not
    /// report.
    Write,
    ReadWrite,
}

impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
            Access::ReadWrite => "read/write",
        }
    }
}

/// A value from text in a wire type: `int` an Int32, `float` a finite Double, `bool` one of
/// `true`, `false`, `1`, `0`, `on`, `off`, `yes`, `no` (any case), `text` as it is. Empty text
/// (or only spaces) is the type's zero for a number or a bool; a text keeps its spaces.
pub fn parse_value(ty: VarType, text: &str) -> Result<Variant, String> {
    let t = text.trim();
    Ok(match ty {
        VarType::Int if t.is_empty() => Variant::Int32(0),
        VarType::Int => Variant::Int32(t.parse().map_err(|_| format!("`{t}` is not an int"))?),
        VarType::Float if t.is_empty() => Variant::Double(0.0),
        VarType::Float => {
            let v: f64 = t.parse().map_err(|_| format!("`{t}` is not a float"))?;
            if !v.is_finite() {
                return Err(format!("`{t}` is not a finite float"));
            }
            Variant::Double(v)
        }
        VarType::Bool => Variant::Boolean(match t.to_ascii_lowercase().as_str() {
            "" | "false" | "0" | "off" | "no" => false,
            "true" | "1" | "on" | "yes" => true,
            _ => return Err(format!("`{t}` is not a bool")),
        }),
        VarType::Text => Variant::from(text.to_string()),
    })
}

/// A value as a monitor shows it: `-` for none.
pub fn render_value(value: &Option<Variant>) -> String {
    match value {
        None => "-".into(),
        Some(Variant::Double(v)) => format!("{v}"),
        Some(Variant::Float(v)) => format!("{v}"),
        Some(Variant::Int32(v)) => format!("{v}"),
        Some(Variant::Int64(v)) => format!("{v}"),
        Some(Variant::UInt32(v)) => format!("{v}"),
        Some(Variant::Boolean(v)) => format!("{v}"),
        Some(Variant::String(s)) => s.as_ref().to_string(),
        Some(Variant::Empty) => "(empty)".into(),
        Some(other) => format!("{other:?}"),
    }
}

/// A client's write, as it landed.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientWrite {
    /// The variable's id.
    pub id: String,
    pub value: Variant,
}

/// One variable's value, from the server's side.
#[derive(Clone, Debug, PartialEq)]
pub struct VarValue {
    pub id: String,
    /// `None` when the variable holds no value.
    pub value: Option<Variant>,
}

/// What the write guard counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerStatus {
    /// Values of the wrong wire type: a client's write refused (`BadTypeMismatch`) or a set
    /// through the handle refused.
    pub type_mismatches: u64,
    /// The last of them: the variable's id and the value it was given, as `Debug` shows it.
    pub last_mismatch: Option<(String, String)>,
    /// Clients' writes landed.
    pub client_writes: u64,
    /// Clients' writes landed but not kept for the caller, [`WRITES_KEPT`] already waiting.
    pub writes_dropped: u64,
}

/// Why a server did not start.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The data namespace is the application's URI: the diagnostics node manager, registered
    /// first, would claim its index and every read would come back `BadNodeIdUnknown` with the
    /// nodes plainly there.
    #[error("the data namespace {0} is the application's URI; give it a URI of its own")]
    NamespaceIsApplication(String),
    /// The data namespace is OPC UA's own, index 0: the variables' node ids would be taken as
    /// OPC UA's, and every read would come back `BadNodeIdUnknown`.
    #[error("the data namespace {0} is OPC UA's own; give it a URI of its own")]
    NamespaceIsOpcUa(String),
    /// A user the stack cannot take, refused before the bind rather than by the stack after it.
    #[error("user `{name}`: {reason}")]
    User { name: String, reason: String },
    /// A variable or the folder that cannot be served, with its id.
    #[error("variable `{id}`: {reason}")]
    Variable { id: String, reason: String },
    #[error("bind {addr}: {source}")]
    Bind {
        addr: String,
        source: std::io::Error,
    },
    #[error("local_addr: {0}")]
    LocalAddr(std::io::Error),
    #[error("build the server: {0}")]
    Build(String),
    #[error("the server has no simple node manager")]
    NoNodeManager,
    #[error("namespace {0} was not registered")]
    NoNamespace(String),
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    #[error("spawn the server thread: {0}")]
    Thread(std::io::Error),
    #[error("the server thread died on startup")]
    DiedOnStartup,
}

/// Why a set through the handle was refused; nothing changed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SetError {
    #[error("no variable `{0}`")]
    Unknown(String),
    #[error("variable `{id}` is {ty}, not {value}")]
    TypeMismatch {
        id: String,
        ty: &'static str,
        value: String,
    },
    #[error("variable `{id}`: the server refused the value ({status})")]
    Refused { id: String, status: String },
}

/// What the handle and the guard share.
struct Shared {
    nodes: Arc<SimpleNodeManager>,
    handle: ServerHandle,
    namespace: u16,
    types: Vec<(String, VarType)>,
    status: Mutex<ServerStatus>,
}

impl Shared {
    fn mismatch(&self, id: &str, value: &Variant) {
        if let Ok(mut s) = self.status.lock() {
            s.type_mismatches += 1;
            s.last_mismatch = Some((id.to_string(), format!("{value:?}")));
        }
    }
}

/// A running server. Dropping it stops the server, closes every connection and frees the port.
pub struct Server {
    /// What a client connects to: `opc.tcp://<bind>:<port>/`.
    pub endpoint_url: String,
    /// The data namespace's index, as the server assigned it.
    pub namespace: u16,
    /// The port the server bound and advertises.
    pub port: u16,
    shared: Arc<Shared>,
    writes: Mutex<Receiver<ClientWrite>>,
    runtime: tokio::runtime::Handle,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

type Ready = Result<(Arc<Shared>, String, u16, tokio::runtime::Handle), ServerError>;

impl Server {
    /// Check the spec, then start the server on a thread of its own. Returns once it is
    /// listening (or failed), so a caller holding the handle knows the port is claimed.
    pub fn start(spec: &ServerSpec) -> Result<Server, ServerError> {
        check(spec)?;
        let spec = spec.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Ready>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let (writes_tx, writes_rx) = std::sync::mpsc::sync_channel::<ClientWrite>(WRITES_KEPT);
        let thread = std::thread::Builder::new()
            .name("veredas-opc-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(ServerError::Runtime(e)));
                        return;
                    }
                };
                let runtime = rt.handle().clone();
                rt.block_on(async move {
                    match serve(&spec, writes_tx).await {
                        Ok((shared, url, port)) => {
                            if ready_tx.send(Ok((shared, url, port, runtime))).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    }
                    let _ = shutdown_rx.await;
                });
                // Dropping the runtime here takes every client connection with it.
            })
            .map_err(ServerError::Thread)?;
        let (shared, endpoint_url, port, runtime) =
            ready_rx.recv().map_err(|_| ServerError::DiedOnStartup)??;
        Ok(Server {
            endpoint_url,
            namespace: shared.namespace,
            port,
            shared,
            writes: Mutex::new(writes_rx),
            runtime,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        })
    }

    /// A variable's node id.
    pub fn node_id(&self, id: &str) -> NodeId {
        NodeId::new(self.namespace, id)
    }

    /// A variable's node id in the `ns=N;s=...` form a configuration holds.
    pub fn node_id_str(&self, id: &str) -> String {
        format!("ns={};s={id}", self.namespace)
    }

    /// Set a variable's value from the caller's side, write-only variables included; it is what
    /// the next read sees. A value of another wire type is refused and counted with the clients'
    /// mismatches. A set is never handed back as a client's write.
    pub fn set(&self, id: &str, value: impl Into<Variant>) -> Result<(), SetError> {
        let value = value.into();
        let Some((_, ty)) = self.shared.types.iter().find(|(v, _)| v == id) else {
            return Err(SetError::Unknown(id.to_string()));
        };
        if !ty.holds(&value) {
            self.shared.mismatch(id, &value);
            return Err(SetError::TypeMismatch {
                id: id.to_string(),
                ty: ty.label(),
                value: format!("{value:?}"),
            });
        }
        self.shared
            .nodes
            .set_value(
                self.shared.handle.subscriptions(),
                &self.node_id(id),
                None,
                DataValue::new_now(value),
            )
            .map_err(|status| SetError::Refused {
                id: id.to_string(),
                status: format!("{status}"),
            })
    }

    /// A variable's value from the server's side, write-only variables included; `None` for no
    /// such variable or no value.
    pub fn get(&self, id: &str) -> Option<Variant> {
        if !self.shared.types.iter().any(|(v, _)| v == id) {
            return None;
        }
        read_node(&self.shared.nodes, &self.node_id(id))
    }

    /// Every variable's value, in the spec's order.
    pub fn snapshot(&self) -> Vec<VarValue> {
        self.shared
            .types
            .iter()
            .map(|(id, _)| VarValue {
                id: id.clone(),
                value: read_node(&self.shared.nodes, &self.node_id(id)),
            })
            .collect()
    }

    /// The clients' writes landed since the last call, in the order they landed.
    pub fn writes(&self) -> Vec<ClientWrite> {
        let Ok(rx) = self.writes.lock() else {
            return Vec::new();
        };
        rx.try_iter().collect()
    }

    /// The next client's write, waiting at most `within`.
    pub fn next_write(&self, within: Duration) -> Option<ClientWrite> {
        self.writes.lock().ok()?.recv_timeout(within).ok()
    }

    /// What the write guard counted.
    pub fn status(&self) -> ServerStatus {
        self.shared
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// The server's runtime: a task spawned on it runs on the server's thread, between the
    /// stack's own tasks, and is dropped with the server.
    pub fn runtime(&self) -> tokio::runtime::Handle {
        self.runtime.clone()
    }

    fn shutdown_and_join(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Stop the server and wait until its port is free.
    pub fn stop(mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("endpoint_url", &self.endpoint_url)
            .field("namespace", &self.namespace)
            .field("port", &self.port)
            .finish()
    }
}

/// What can be refused before anything binds.
fn check(spec: &ServerSpec) -> Result<(), ServerError> {
    if spec.namespace.trim() == spec.application_uri.trim() {
        return Err(ServerError::NamespaceIsApplication(spec.namespace.clone()));
    }
    if spec.namespace.trim() == OPC_UA_NAMESPACE {
        return Err(ServerError::NamespaceIsOpcUa(spec.namespace.clone()));
    }
    if let Some(user) = &spec.user {
        let reason = if user.name.is_empty() {
            Some("the name is empty")
        } else if user.name == opcua_server::ANONYMOUS_USER_TOKEN_ID {
            Some("the anonymous token's id, which the stack reserves")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(ServerError::User {
                name: user.name.clone(),
                reason: reason.to_string(),
            });
        }
    }
    let refuse = |id: &str, reason: &str| {
        Err(ServerError::Variable {
            id: id.to_string(),
            reason: reason.to_string(),
        })
    };
    if spec.folder.id.trim().is_empty() {
        return refuse(&spec.folder.id, "the folder's id is empty");
    }
    let mut seen = std::collections::BTreeSet::new();
    for v in &spec.variables {
        if v.id.trim().is_empty() || v.id.trim() != v.id {
            return refuse(&v.id, "an id is not empty and has no spaces around it");
        }
        if v.id == spec.folder.id {
            return refuse(&v.id, "the folder's id");
        }
        if !seen.insert(v.id.as_str()) {
            return refuse(&v.id, "given twice");
        }
        if !v.ty.holds(&v.value) {
            return refuse(
                &v.id,
                &format!("its initial value {:?} is not {}", v.value, v.ty.label()),
            );
        }
    }
    Ok(())
}

async fn serve(
    spec: &ServerSpec,
    writes: SyncSender<ClientWrite>,
) -> Result<(Arc<Shared>, String, u16), ServerError> {
    let host = spec.bind.trim();
    // Bind first, then tell the server the port it got: with port 0 the advertised endpoint
    // URLs would otherwise disagree with the socket and a client's match would find nothing.
    let addr = format!("{host}:{}", spec.port);
    let listener = TcpListener::bind((host, spec.port))
        .await
        .map_err(|source| ServerError::Bind { addr, source })?;
    let port = listener
        .local_addr()
        .map_err(ServerError::LocalAddr)?
        .port();

    let mut users = vec![opcua_server::ANONYMOUS_USER_TOKEN_ID.to_string()];
    let mut builder = ServerBuilder::new()
        .application_name(spec.application_name.as_str())
        .application_uri(spec.application_uri.as_str())
        .product_uri(spec.application_uri.as_str())
        .host(host)
        .port(port)
        .discovery_urls(vec!["/".to_owned()]);
    if let Some(user) = &spec.user {
        builder = builder.add_user_token(
            &user.name,
            ServerUserToken::user_pass(&user.name, &user.password),
        );
        users.push(user.name.clone());
    }
    builder = builder
        .add_endpoint("none", ServerEndpoint::new_none("/", &users))
        .default_endpoint("none")
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: spec.namespace.clone(),
                ..Default::default()
            },
            "veredas",
        ));
    if let Some(n) = spec.max_sessions {
        builder = builder.max_sessions(n);
    }
    if let Some(n) = spec.max_nodes_per_read {
        builder.limits_mut().operational.max_nodes_per_read = n;
    }
    let (server, handle) = builder
        .build()
        .map_err(|e| ServerError::Build(e.to_string()))?;
    let nodes = handle
        .node_managers()
        .get_of_type::<SimpleNodeManager>()
        .ok_or(ServerError::NoNodeManager)?;
    let namespace = handle
        .get_namespace_index(&spec.namespace)
        .ok_or_else(|| ServerError::NoNamespace(spec.namespace.clone()))?;
    populate(&nodes, namespace, spec);
    let shared = Arc::new(Shared {
        nodes,
        handle,
        namespace,
        types: spec
            .variables
            .iter()
            .map(|v| (v.id.clone(), v.ty))
            .collect(),
        status: Mutex::new(ServerStatus::default()),
    });
    guard_writes(&shared, spec, writes);
    tokio::spawn(server.run_with(listener));
    Ok((shared, format!("opc.tcp://{host}:{port}/"), port))
}

/// The folder and its variables.
fn populate(nodes: &SimpleNodeManager, ns: u16, spec: &ServerSpec) {
    let mut space = nodes.address_space().write();
    let folder = NodeId::new(ns, spec.folder.id.as_str());
    let name = spec.folder.name.as_str();
    ObjectBuilder::new(&folder, name, name)
        .is_folder()
        .organized_by(ObjectId::ObjectsFolder)
        .insert(&mut *space);
    for v in &spec.variables {
        let name = v
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&v.id);
        let builder = VariableBuilder::new(&NodeId::new(ns, v.id.as_str()), name, name)
            // A variable without a type definition browses with a null one, and a browsing
            // client reads it to tell a plain variable from a structured one.
            .has_type_definition(VariableTypeId::BaseDataVariableType)
            .value(v.value.clone())
            .organized_by(folder.clone());
        let builder = match v.ty {
            VarType::Int => builder.data_type(DataTypeId::Int32),
            VarType::Float => builder.data_type(DataTypeId::Double),
            VarType::Bool => builder.data_type(DataTypeId::Boolean),
            VarType::Text => builder.data_type(DataTypeId::String),
        };
        let builder = match v.access {
            Access::Read => builder,
            Access::ReadWrite => builder.writable(),
            // Set apart from `writable()`, which adds write to the default read.
            Access::Write => builder
                .access_level(AccessLevel::CURRENT_WRITE)
                .user_access_level(AccessLevel::CURRENT_WRITE),
        };
        builder.insert(&mut *space);
    }
}

/// A write callback on every writable variable. The callback runs under the address space's lock
/// and cannot store the value itself, so it hands a right one to a task on the server's runtime,
/// which lands it the moment the lock is free and then hands it to the caller. The runtime is a
/// current-thread one, so the tasks run in the order the writes came, and the caller sees them in
/// the order they landed.
fn guard_writes(shared: &Arc<Shared>, spec: &ServerSpec, writes: SyncSender<ClientWrite>) {
    for v in spec.variables.iter().filter(|v| v.access != Access::Read) {
        let id = v.id.clone();
        let ty = v.ty;
        let node = NodeId::new(shared.namespace, id.as_str());
        let shared_cb = Arc::clone(shared);
        let writes = writes.clone();
        shared
            .nodes
            .inner()
            .add_write_callback(node.clone(), move |value, range| {
                if *range != NumericRange::None {
                    return StatusCode::BadIndexRangeInvalid;
                }
                let Some(variant) = value.value else {
                    return StatusCode::BadTypeMismatch;
                };
                if !ty.holds(&variant) {
                    shared_cb.mismatch(&id, &variant);
                    return StatusCode::BadTypeMismatch;
                }
                let (shared, node, id, writes) = (
                    Arc::clone(&shared_cb),
                    node.clone(),
                    id.clone(),
                    writes.clone(),
                );
                tokio::spawn(async move {
                    let landed = shared.nodes.set_value(
                        shared.handle.subscriptions(),
                        &node,
                        None,
                        DataValue::new_now(variant.clone()),
                    );
                    if landed.is_err() {
                        return;
                    }
                    // Counted and handed under the status's lock, so a caller handed this write
                    // and asking the status next finds it counted.
                    let mut status = shared.status.lock().ok();
                    let kept = writes.try_send(ClientWrite { id, value: variant });
                    if let Some(s) = status.as_mut() {
                        s.client_writes += 1;
                        if let Err(TrySendError::Full(_)) = kept {
                            s.writes_dropped += 1;
                        }
                    }
                });
                StatusCode::Good
            });
    }
}

/// A node's value, straight from the address space.
fn read_node(nodes: &SimpleNodeManager, node: &NodeId) -> Option<Variant> {
    let space = nodes.address_space().read();
    match space.find(node)? {
        NodeType::Variable(v) => {
            v.value(
                TimestampsToReturn::Neither,
                &NumericRange::None,
                &DataEncoding::Binary,
                0.0,
            )
            .value
        }
        _ => None,
    }
}
