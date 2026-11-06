//! Why a call to an OPC UA server produced nothing, and why a node id did not parse.
//!
//! Veredas 1 returned these as `String`s; here each is a variant an operator-facing layer can
//! match on, and its `Display` is the text Veredas 1 showed, so the operator reads the same words.

use opcua_types::StatusCode;

/// Which service call an error is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Call {
    /// A Read of tag values.
    Read,
    /// A Write of tag values.
    Write,
    /// The first page of a Browse.
    Browse,
    /// A BrowseNext, following a continuation point.
    BrowseNext,
    /// The Read of browsed variables' DataType attributes.
    ReadDataTypes,
    /// The Read of one node's attribute set.
    ReadDetails,
}

impl Call {
    fn timed_out(self) -> &'static str {
        match self {
            Call::Read => "read timed out",
            Call::Write => "write timed out",
            Call::Browse => "browse timed out",
            Call::BrowseNext => "browse (next page) timed out",
            Call::ReadDataTypes => "read of data types timed out",
            Call::ReadDetails => "read of node details timed out",
        }
    }

    fn failed(self) -> &'static str {
        match self {
            Call::Read => "read",
            Call::Write => "write",
            Call::Browse => "browse",
            Call::BrowseNext => "browse next",
            Call::ReadDataTypes => "read data types",
            Call::ReadDetails => "read details",
        }
    }

    fn refused(self) -> &'static str {
        match self {
            Call::BrowseNext => "browse continuation",
            other => other.failed(),
        }
    }
}

/// Why a call to a server, or the connection it needs, produced no result.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpcError {
    /// The client could not be configured; the builder's complaints, joined.
    #[error("client: {0}")]
    Client(String),
    /// The server refused the connection, or could not be reached: the stack's status, and its
    /// whole message (which leads with the status).
    #[error("{detail}")]
    Connect { status: StatusCode, detail: String },
    /// The server took longer than the connect timeout to accept a session.
    #[error("no session within {0} s")]
    ConnectTimedOut(u64),
    /// The connection's event loop ended before the session became usable (a rejected
    /// credential ends here, rather than in a wait that never resolves); `None` when the loop
    /// itself died.
    #[error("connection failed ({})", status_text(*.0))]
    ConnectionFailed(Option<StatusCode>),
    /// The connection settled without the session ever becoming usable.
    #[error("the session never became usable")]
    NeverUsable,
    /// A live connection ended; `None` when its event loop itself died.
    #[error("connection closed ({})", status_text(*.0))]
    ConnectionClosed(Option<StatusCode>),
    /// A call took longer than its bound.
    #[error("{}", .0.timed_out())]
    TimedOut(Call),
    /// The call failed: the server answered with a service fault, or the stack could not send
    /// it. The stack's status, and its whole message (which leads with the status).
    #[error("{}: {detail}", call.failed())]
    Failed { call: Call, status: StatusCode, detail: String },
    /// The call succeeded but its one result carries a bad status: the server refused that
    /// operation (a browse of a node it will not show), which is not an empty answer.
    #[error("the server refused the {}: {status}", call.refused())]
    Refused { call: Call, status: StatusCode },
    /// The call succeeded with no result at all.
    #[error("{} returned no result", call.failed())]
    NoResult { call: Call },
    /// A browse still had a continuation point after the page ceiling: `pages` were read.
    #[error(
        "browse did not finish after {pages} pages — the server keeps returning continuation points"
    )]
    EndlessBrowse { pages: usize },
    /// A node id given as text did not parse.
    #[error(transparent)]
    NodeId(#[from] NodeIdError),
    /// The worker's runtime could not be built.
    #[error("runtime: {0}")]
    Runtime(String),
    /// The worker's thread could not be started.
    #[error("cannot start the worker thread: {0}")]
    Thread(String),
    /// The worker is between connections (connecting, or waiting to retry): a request is
    /// answered at once rather than held until a connection exists.
    #[error("not connected to the server")]
    NotConnected,
    /// The worker has ended, or ended with the request still queued: nothing was sent.
    #[error("the OPC worker is not running")]
    WorkerGone,
    /// The worker ended while serving the request (stopped mid-call).
    #[error("the OPC worker dropped the request")]
    WorkerDropped,
}

impl OpcError {
    /// A failed call, from the stack's error.
    pub(crate) fn failed(call: Call, e: &opcua_types::Error) -> OpcError {
        OpcError::Failed { call, status: e.status(), detail: e.to_string() }
    }
}

fn status_text(status: Option<StatusCode>) -> String {
    match status {
        Some(s) => s.to_string(),
        None => "event loop ended".to_string(),
    }
}

/// Why a node id given as text did not parse.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NodeIdError {
    /// Nothing but whitespace.
    #[error("node id is empty")]
    Empty,
    /// Not the standard text form; carries the input, trimmed.
    #[error("`{0}` is not a node id — expected `ns=<index>;s=<name>` (or `i=`, `g=`, `b=`)")]
    Malformed(String),
}
