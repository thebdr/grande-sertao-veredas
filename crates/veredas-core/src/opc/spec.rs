//! The compiled shape of an OPC source: what a source's configuration becomes once validated,
//! and what the poll worker runs.
//!
//! Compiling configuration text into this shape before anything connects means a mistyped node
//! id is refused where it was typed, with its text quoted, not discovered when the first poll
//! comes back bad.

use std::str::FromStr;
use std::time::Duration;

use opcua_types::NodeId;

use crate::SeriesType;
use crate::opc::error::NodeIdError;

/// How the client proves who it is to the server.
///
/// The two identities an unsecured plant endpoint accepts. Certificate-based identities are
/// absent rather than stubbed: adding one starts here.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    Anonymous,
    /// The password lives only in this value, in memory, for as long as the spec does; nothing
    /// in this layer writes it anywhere or shows it: neither [`Auth::label`] nor `Debug` (so
    /// neither does a `PollSpec`'s `Debug`).
    UserPass { username: String, password: String },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Anonymous => f.write_str("Anonymous"),
            Auth::UserPass { username, .. } => {
                f.debug_struct("UserPass").field("username", username).field("password", &"<hidden>").finish()
            }
        }
    }
}

impl Auth {
    /// What to show an operator: never the password.
    pub fn label(&self) -> String {
        match self {
            Auth::Anonymous => "anonymous".to_string(),
            Auth::UserPass { username, .. } => format!("user {username}"),
        }
    }
}

/// One tag to poll, and the series it becomes.
#[derive(Clone, Debug)]
pub struct TagSpec {
    /// The parsed node id: parsing happened when the spec was compiled.
    pub node_id: NodeId,
    /// The node id as configured, for messages that quote what the operator wrote, and the key
    /// a write names the tag by.
    pub raw: String,
    /// Column name in this rule's table. Empty means "bits only": the tag is polled to be
    /// decomposed, and the raw word is not itself a column.
    pub series: String,
    pub ty: SeriesType,
    /// May be written through the worker's handle. A write to a tag not marked here is refused
    /// before it reaches the wire, whatever the server would allow.
    pub writable: bool,
    /// Bit fields carved out of this tag's integer value.
    pub bits: Vec<BitSpec>,
}

impl TagSpec {
    /// Every column this tag contributes, in table order: the raw value (when it has a name)
    /// then its bits, each bit field an `Int`.
    pub fn columns(&self) -> impl Iterator<Item = (&str, SeriesType)> {
        let raw = if self.series.is_empty() { None } else { Some((self.series.as_str(), self.ty)) };
        raw.into_iter().chain(self.bits.iter().map(|b| (b.series.as_str(), SeriesType::Int)))
    }
}

/// One bit field of a packed integer tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitSpec {
    /// Least-significant bit of the field, 0-based.
    pub bit: u8,
    /// How many bits wide; 1 is the usual flag, 0 reads as 1.
    pub width: u8,
    /// Column name for the extracted field.
    pub series: String,
}

/// One mapping rule: a named table whose columns are this source's tags.
#[derive(Clone, Debug)]
pub struct RuleSpec {
    /// The table's name.
    pub name: String,
    pub tags: Vec<TagSpec>,
    /// Keep transitions, not every identical poll: when set, a poll whose values equal the last
    /// kept ones for this rule yields no row and no samples, except a heartbeat every
    /// [`crate::opc::map::HEARTBEAT_POLLS`] polls, so a window over a quiet stretch still has
    /// points in it and a stale source stays distinguishable from a still one. For plants whose
    /// tags are words that rarely move; off, every poll is a row.
    pub store_on_change: bool,
}

impl RuleSpec {
    /// The rule's columns in table order.
    pub fn columns(&self) -> Vec<(String, SeriesType)> {
        self.tags.iter().flat_map(|t| t.columns()).map(|(n, t)| (n.to_string(), t)).collect()
    }
}

/// Everything one OPC source's worker needs: where to connect, how often, and which tags feed
/// which tables.
#[derive(Clone, Debug)]
pub struct PollSpec {
    /// The source's id: tags its rows, its samples and its worker thread.
    pub source_id: String,
    pub display_name: String,
    pub endpoint_url: String,
    pub auth: Auth,
    /// The poll interval; [`crate::opc::spawn`] refuses zero.
    pub poll: Duration,
    pub rules: Vec<RuleSpec>,
}

impl PollSpec {
    /// Every distinct node this source polls, in first-seen order: one poll reads the union,
    /// and rules index back into the result by node.
    ///
    /// Deduplicated on purpose: two rules mapping the same tag (a raw value in one table, its
    /// bits in another) must not double the wire traffic.
    pub fn poll_nodes(&self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = Vec::new();
        for tag in self.rules.iter().flat_map(|r| r.tags.iter()) {
            if !out.contains(&tag.node_id) {
                out.push(tag.node_id.clone());
            }
        }
        out
    }

    /// A writable tag by the node id text it was configured with (surrounding whitespace
    /// ignored). `None` when the tag is unknown OR not marked writable: the caller cannot tell
    /// those apart, and should not, since both mean "not something you may write". A write
    /// flag is always respected: the same text also configured read-only, anywhere, does not
    /// hide the writable tag.
    pub fn writable_tag(&self, raw: &str) -> Option<&TagSpec> {
        let raw = raw.trim();
        self.rules.iter().flat_map(|r| r.tags.iter()).find(|t| t.writable && t.raw.trim() == raw)
    }

    /// Any polled tag, writable or not, by the node id text it was configured with (surrounding
    /// whitespace ignored).
    pub fn tag(&self, raw: &str) -> Option<&TagSpec> {
        let raw = raw.trim();
        self.rules.iter().flat_map(|r| r.tags.iter()).find(|t| t.raw.trim() == raw)
    }
}

/// Parse a node id in the standard `ns=<n>;<type>=<value>` text form, refusing with the input
/// quoted and the expected form shown.
pub fn parse_node_id(s: &str) -> Result<NodeId, NodeIdError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(NodeIdError::Empty);
    }
    NodeId::from_str(s).map_err(|_| NodeIdError::Malformed(s.to_string()))
}
