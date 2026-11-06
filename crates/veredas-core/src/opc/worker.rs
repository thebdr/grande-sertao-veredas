//! The poll worker: one per OPC source, a thread with its own tokio runtime.
//!
//! It has the shape of every source worker: spawned, pushing events into a channel, stopped by a
//! shared flag, ending in a [`SourceHandle`]. So a consumer holds an OPC source as it holds any
//! other: one health model, one stop, one join.
//!
//! Two things it does that a reader of a file does not:
//!
//! * **It reconnects on its own.** A plant server restarts, a switch reboots; the worker retries
//!   with backoff and reports its state rather than dying, which would need someone to notice.
//! * **It serves writes.** Reads are a timer; writes and on-demand reads arrive over a channel
//!   and are answered on a oneshot. Both share the one session, which is why they live in the
//!   same `select!` rather than opening a connection per request.
//!
//! What a write's answer means, because a command pulse must never fire when nobody expects it:
//!
//! * A request that arrives while there is no session is answered at once,
//!   [`WriteError::NotSent`] with [`OpcError::NotConnected`], never held until one exists: it
//!   cannot fire when the plant returns. So is one still queued when the worker ends.
//! * A request whose caller stopped waiting before the worker reached it (the caller's own
//!   timeout dropped the future) is never sent: a pulse its operator gave up on, and pressed
//!   again, does not land twice later.
//! * A write whose call fails or times out is [`WriteError::Unconfirmed`]: it may have reached
//!   the plant, and a link that stalled without the worker noticing can still deliver it when it
//!   recovers. No client can recall bytes already on the wire, so the caller is told the truth
//!   rather than "refused". The worker drops that session and reconnects.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use opcua_client::{Client, ClientBuilder, IdentityToken, Session};
use opcua_types::{
    AttributeId, DataValue, EndpointDescription, MessageSecurityMode, NodeId, ReadValueId,
    StatusCode, TimestampsToReturn, UserTokenPolicy, Variant, WriteValue,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::opc::error::{Call, OpcError};
use crate::opc::spec::{Auth, PollSpec};
use crate::{Quality, SeriesType, SourceError, SourceHandle, SourceId, now_unix_ms, stop_flag};

/// The first wait before a new connection attempt. Fast, because a blipped switch is back in a
/// second.
pub const RETRY_MIN: Duration = Duration::from_millis(500);
/// The longest wait between two connection attempts, so a plant that is off for the night does
/// not spin.
pub const RETRY_MAX: Duration = Duration::from_secs(30);

/// How long one Read or Write is given before the worker treats the session as wedged and
/// reconnects. Servers do occasionally accept a connection and then never answer.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a connection attempt, endpoint discovery to usable session, is given. A peer that
/// accepts TCP and never speaks OPC UA must not hold the worker.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a graceful close (CloseSession, so the server frees the session's slot) may take
/// when the session is being dropped for a failure. By then the peer is believed gone.
pub const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);

/// How long a graceful close may take when the worker is stopping: a stop is honoured within a
/// fraction of a second, whatever the peer does.
pub const STOP_CLOSE_TIMEOUT: Duration = Duration::from_millis(100);

/// How often a wait checks the stop flag.
const STOP_CHECK: Duration = Duration::from_millis(50);

/// The most nodes one Read service call carries. Servers cap operations per call and refuse a
/// call past the cap whole, and this stack refuses to encode or decode an array of more than
/// 1 000 elements either way, so a source of thousands of tags is read in bounded chunks instead
/// of one all-or-nothing request (a field source of 3,048 tags was refused whole while one of 839
/// passed). 500 sits under every stack met so far while keeping a poll of a few thousand tags at
/// a handful of round trips.
pub const MAX_NODES_PER_READ: usize = 500;

/// The waits between connection attempts, in order: [`RETRY_MIN`], doubling, held at
/// [`RETRY_MAX`]. The schedule starts over after every connection that reaches a session.
pub fn retry_delays() -> impl Iterator<Item = Duration> {
    std::iter::successors(Some(RETRY_MIN), |d| Some((*d * 2).min(RETRY_MAX)))
}

/// What a worker sends upstream.
#[derive(Debug, Clone)]
pub enum OpcEvent {
    /// One completed poll of every tag on this source.
    Poll {
        source_id: String,
        /// Where the poll sits on the plant's time axis: plant-local wall clock as naive
        /// milliseconds, the axis a stamp parsed from a plant's own text lands on, so a polled
        /// row and a logged line from the same second line up.
        index_ms: i64,
        /// When the poll arrived on the unix clock ([`crate::now_unix_ms`]).
        ///
        /// The two differ by the UTC offset, and liveness is judged on this one: judging it on
        /// the plant axis would read every source stale by the offset west of Greenwich.
        arrival_ms: i64,
        /// One entry per node, in [`PollSpec::poll_nodes`] order: the value, or why it is
        /// absent.
        values: Vec<TagRead>,
    },
    /// The connection state changed: the source's health, not an error path.
    Status { source_id: String, state: ConnState },
}

/// One tag's result within a poll.
#[derive(Debug, Clone, PartialEq)]
pub struct TagRead {
    pub node_id: NodeId,
    /// The value, only when the server's status for this node is good and it sent one (an
    /// empty variant is none): one bad tag never invalidates the rest of the poll, and an
    /// uncertain value is not passed off as a reading.
    pub value: Option<Variant>,
    /// The server's status, for naming which tag is unhappy.
    pub status: StatusCode,
}

impl TagRead {
    /// A value, with a good status.
    pub fn is_good(&self) -> bool {
        self.value.as_ref().is_some_and(|v| !matches!(v, Variant::Empty)) && self.status.is_good()
    }

    /// How far this reading can be trusted: good only when [`TagRead::is_good`]; uncertain when
    /// the server said so; bad otherwise (a bad status, or a good one that carried no value).
    pub fn quality(&self) -> Quality {
        if self.is_good() {
            Quality::Good
        } else if self.status.is_uncertain() {
            Quality::Uncertain
        } else {
            Quality::Bad
        }
    }
}

/// A Read's answer as one [`TagRead`] per node asked for, positionally: no status is good, a
/// value is kept only beside a good status (an empty variant is none), and an answer shorter
/// than the request pads the missing nodes with `BadNoData` rather than shifting every later
/// node onto the wrong column. Answers beyond the request are ignored.
pub fn tag_reads(nodes: &[NodeId], values: &[DataValue]) -> Vec<TagRead> {
    nodes
        .iter()
        .enumerate()
        .map(|(i, node)| match values.get(i) {
            Some(dv) => {
                let status = dv.status.unwrap_or(StatusCode::Good);
                let value = dv.value.as_ref().filter(|v| status.is_good() && !matches!(v, Variant::Empty));
                TagRead { node_id: node.clone(), status, value: value.cloned() }
            }
            None => TagRead { node_id: node.clone(), value: None, status: StatusCode::BadNoData },
        })
        .collect()
}

/// Where a worker's connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    Connecting,
    Connected,
    /// Lost or refused: why, and when the next attempt is due.
    Retrying { error: OpcError, in_ms: u64 },
}

/// A write for the worker, answered on `reply`.
#[derive(Debug)]
struct WriteRequest {
    writes: Vec<(NodeId, Variant)>,
    reply: oneshot::Sender<Result<Vec<StatusCode>, WriteError>>,
}

/// An on-demand read for the worker, answered on `reply`.
#[derive(Debug)]
struct ReadRequest {
    nodes: Vec<NodeId>,
    reply: oneshot::Sender<Result<Vec<TagRead>, OpcError>>,
}

/// One job for a live session, off the poll timer.
#[derive(Debug)]
enum Job {
    Write(WriteRequest),
    Read(ReadRequest),
}

impl Job {
    /// Answer without a session: nothing was sent.
    fn refuse(self, why: OpcError) {
        // The requester may have hung up; that is not the worker's problem.
        match self {
            Job::Write(r) => {
                let _ = r.reply.send(Err(WriteError::NotSent(why)));
            }
            Job::Read(r) => {
                let _ = r.reply.send(Err(why));
            }
        }
    }
}

/// The master write switch, shared live between whoever owns it and every handle that obeys it.
///
/// An `Arc<AtomicBool>` rather than a copied `bool` on purpose: disarming is a safety action and
/// takes effect on the next write, not the next session. It is checked when a handle takes a
/// write and again by the worker just before the write goes on the wire, so a write still queued
/// when the switch is turned off never reaches the plant.
#[derive(Clone, Debug, Default)]
pub struct WriteGate(Arc<AtomicBool>);

impl WriteGate {
    pub fn new(armed: bool) -> WriteGate {
        WriteGate(Arc::new(AtomicBool::new(armed)))
    }

    /// Arm or disarm every handle sharing this gate, at once.
    pub fn set(&self, armed: bool) {
        self.0.store(armed, Ordering::SeqCst);
    }

    pub fn is_armed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Why a write did not come back with the server's statuses.
///
/// Three of these say the plant never saw the request; one says it may have. A server that
/// takes the call and answers a bad status per write comes back as `Ok(statuses)` instead.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The master switch is off (checked by the handle, and again by the worker before the
    /// wire). Nothing was sent.
    #[error("writes are turned off for this project")]
    Disarmed,
    /// The tag is unknown, or known and not marked writable. The caller cannot tell those apart,
    /// and must not: both mean "not something you may write". Nothing was sent.
    #[error("`{source_id}` / `{tag}` is not a writable tag in this session")]
    NotWritable { source_id: String, tag: String },
    /// Nothing was sent: no session ([`OpcError::NotConnected`]), or the worker had ended or
    /// ended with the request still queued ([`OpcError::WorkerGone`]).
    #[error("{0}")]
    NotSent(OpcError),
    /// The write went to the wire, or may have, and no answer came: the call failed or timed
    /// out, or the worker was stopped mid-call. The plant may apply it, now or when a stalled
    /// link recovers.
    #[error("the write may have reached the plant: {0}")]
    Unconfirmed(OpcError),
}

/// The handle to a running worker: writes and on-demand reads.
///
/// **The write gates live here**, so every caller holding a handle, whatever brought it the
/// request, goes through them: the node-level write is private, and [`OpcHandle::write_tags`] is
/// the only way to the wire.
#[derive(Clone, Debug)]
pub struct OpcHandle {
    pub source_id: String,
    jobs: UnboundedSender<Job>,
    /// The whitelist: the compiled spec this worker runs. A tag that is not `writable` in here
    /// has no route to the wire.
    spec: Arc<PollSpec>,
    /// The master switch.
    gate: WriteGate,
}

impl OpcHandle {
    /// Whether writes are armed right now, to render an honest disabled control rather than one
    /// that fails when pressed.
    pub fn is_armed(&self) -> bool {
        self.gate.is_armed()
    }

    /// The compiled spec this source runs.
    pub fn spec(&self) -> &PollSpec {
        &self.spec
    }

    /// The configured type of a writable tag, to coerce typed text before sending it. `None`
    /// when the tag is not writable: the same answer as an unknown tag, deliberately.
    pub fn writable_type(&self, tag: &str) -> Option<SeriesType> {
        self.spec.writable_tag(tag).map(|t| t.ty)
    }

    /// Write tags named by their configured node id text, and await the per-write statuses.
    ///
    /// **The only write path.** The master switch is checked first, then every tag against the
    /// whitelist, all before anything is sent: a batch with one tag not permitted is refused
    /// whole, so a half-permitted batch never half-fires. The worker checks the switch once more
    /// before the wire. See [`WriteError`] for what each failure says about the plant.
    pub async fn write_tags(&self, writes: Vec<(String, Variant)>) -> Result<Vec<StatusCode>, WriteError> {
        if !self.gate.is_armed() {
            return Err(WriteError::Disarmed);
        }
        let mut resolved: Vec<(NodeId, Variant)> = Vec::with_capacity(writes.len());
        for (tag, value) in writes {
            let spec = self.spec.writable_tag(&tag).ok_or_else(|| WriteError::NotWritable {
                source_id: self.source_id.clone(),
                tag: tag.clone(),
            })?;
            resolved.push((spec.node_id.clone(), value));
        }
        let (reply, rx) = oneshot::channel();
        self.jobs
            .send(Job::Write(WriteRequest { writes: resolved, reply }))
            .map_err(|_| WriteError::NotSent(OpcError::WorkerGone))?;
        // The worker answers every job it takes; one it drops was mid-call when it stopped.
        rx.await.map_err(|_| WriteError::Unconfirmed(OpcError::WorkerDropped))?
    }

    /// Read nodes on demand, on the worker's session, positionally like a poll.
    pub async fn read(&self, nodes: Vec<NodeId>) -> Result<Vec<TagRead>, OpcError> {
        let (reply, rx) = oneshot::channel();
        self.jobs.send(Job::Read(ReadRequest { nodes, reply })).map_err(|_| OpcError::WorkerGone)?;
        rx.await.map_err(|_| OpcError::WorkerDropped)?
    }
}

/// A spawned worker: the stop/join handle, plus the handle for writes and reads.
pub struct OpcWorker {
    pub handle: SourceHandle,
    pub writer: OpcHandle,
}

/// Spawn the poll worker for one source.
///
/// It runs on its own thread with a current-thread runtime, so a wedged server starves no other
/// source and the caller needs no runtime of its own. A zero poll interval is refused.
///
/// `gate` is the master write switch, shared: pass the SAME gate to every source so one toggle
/// disarms them all. There is no gate-less spawn: a caller says what the write policy is.
pub fn spawn(spec: PollSpec, tx: Sender<OpcEvent>, gate: WriteGate) -> Result<OpcWorker, SourceError> {
    if spec.poll.is_zero() {
        return Err(SourceError::NotConfigured(format!(
            "OPC source `{}`: the poll interval is zero",
            spec.source_id
        )));
    }
    let stop = stop_flag();
    let source_id = SourceId::new(&spec.source_id);
    let (jobs_tx, mut jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let spec = Arc::new(spec);
    let writer = OpcHandle {
        source_id: spec.source_id.clone(),
        jobs: jobs_tx,
        spec: Arc::clone(&spec),
        gate: gate.clone(),
    };

    let thread_stop = Arc::clone(&stop);
    let join = std::thread::Builder::new()
        .name(format!("opc-{}", spec.source_id))
        .spawn(move || {
            let outcome = run_on_own_runtime(run(spec, tx, &mut jobs_rx, thread_stop, gate))
                .map_err(|e| SourceError::NotConfigured(format!("OPC worker runtime: {e}")))
                .and_then(|ended| ended);
            // Nothing still queued was sent: say so, and refuse whatever comes after.
            jobs_rx.close();
            while let Ok(job) = jobs_rx.try_recv() {
                job.refuse(OpcError::WorkerGone);
            }
            outcome
        })
        .map_err(|e| SourceError::NotConfigured(format!("spawn OPC worker: {e}")))?;

    Ok(OpcWorker { handle: SourceHandle::new(source_id, stop, join), writer })
}

/// Run `task` to its end on a current-thread runtime of its own, then shut the runtime down
/// WITHOUT waiting for its blocking tasks, and return the task's output.
///
/// The OPC stack resolves a host name on the runtime's blocking pool, and a lookup that fails
/// can take seconds; dropping a runtime joins that pool, so a stop or a drop would wait as long
/// as the resolver does. Every thread of this layer that runs the stack (the poll worker, the
/// browser) runs it through here, so the thread ends as soon as its task does, whatever is still
/// blocking behind it (left to finish on its own). Errs only when the runtime cannot be built.
pub fn run_on_own_runtime<F: std::future::Future>(task: F) -> std::io::Result<F::Output> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let output = rt.block_on(task);
    rt.shutdown_background();
    Ok(output)
}

/// Build the client. No keypair: the layer speaks SecurityPolicy None, and a sample keypair
/// would write a PKI directory beside the executable for nothing.
fn client_for() -> Result<Client, OpcError> {
    ClientBuilder::new()
        .application_name("veredas")
        .application_uri("urn:veredas:client")
        .product_uri("urn:veredas:client")
        .create_sample_keypair(false)
        .trust_server_certs(true)
        .session_retry_limit(1)
        .client()
        // The builder reports a LIST of configuration complaints, not one error.
        .map_err(|errors| OpcError::Client(errors.join("; ")))
}

fn identity(auth: &Auth) -> IdentityToken {
    match auth {
        Auth::Anonymous => IdentityToken::Anonymous,
        Auth::UserPass { username, password } => {
            IdentityToken::UserName(username.clone(), password.as_str().into())
        }
    }
}

/// A session's event loop, aborted when dropped: a connection abandoned half-way (a timeout, a
/// stop) leaves no task retrying behind it.
pub(crate) struct EventLoop(JoinHandle<StatusCode>);

impl Drop for EventLoop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Connect and wait until the session is usable, within [`CONNECT_TIMEOUT`].
///
/// The event loop is raced against the connection wait: a REJECTED credential never resolves
/// `wait_for_connection`, so waiting on it alone turns a wrong password into a hang instead of a
/// message.
pub(crate) async fn connect(spec: &PollSpec) -> Result<(Arc<Session>, EventLoop), OpcError> {
    tokio::time::timeout(CONNECT_TIMEOUT, connect_unbounded(spec))
        .await
        .unwrap_or(Err(OpcError::ConnectTimedOut(CONNECT_TIMEOUT.as_secs())))
}

async fn connect_unbounded(spec: &PollSpec) -> Result<(Arc<Session>, EventLoop), OpcError> {
    let mut client = client_for()?;
    let endpoint: EndpointDescription = (
        spec.endpoint_url.as_str(),
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    )
        .into();
    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, identity(&spec.auth))
        .await
        .map_err(|e| OpcError::Connect { status: e.status(), detail: e.to_string() })?;
    let mut event_loop = EventLoop(event_loop.spawn());
    tokio::select! {
        status = &mut event_loop.0 => Err(OpcError::ConnectionFailed(status.ok())),
        ok = session.wait_for_connection() => {
            if ok { Ok((session, event_loop)) } else { Err(OpcError::NeverUsable) }
        }
    }
}

/// Read a batch of nodes into per-tag results ([`tag_reads`]), in chunks of at most
/// [`MAX_NODES_PER_READ`], each its own call bounded by [`CALL_TIMEOUT`]; results reassemble in
/// order, so chunking is invisible to the mapping.
pub(crate) async fn read_nodes(session: &Session, nodes: &[NodeId]) -> Result<Vec<TagRead>, OpcError> {
    let mut out = Vec::with_capacity(nodes.len());
    for chunk in nodes.chunks(MAX_NODES_PER_READ) {
        let to_read: Vec<ReadValueId> = chunk
            .iter()
            .map(|n| ReadValueId {
                node_id: n.clone(),
                attribute_id: AttributeId::Value as u32,
                ..Default::default()
            })
            .collect();
        let values = tokio::time::timeout(CALL_TIMEOUT, session.read(&to_read, TimestampsToReturn::Both, 0.0))
            .await
            .map_err(|_| OpcError::TimedOut(Call::Read))?
            .map_err(|e| OpcError::failed(Call::Read, &e))?;
        out.extend(tag_reads(chunk, &values));
    }
    Ok(out)
}

/// Write a batch of (node, value) pairs in one call, bounded by [`CALL_TIMEOUT`].
async fn write_nodes(session: &Session, writes: &[(NodeId, Variant)]) -> Result<Vec<StatusCode>, OpcError> {
    if writes.is_empty() {
        return Ok(Vec::new());
    }
    let to_write: Vec<WriteValue> = writes
        .iter()
        .map(|(node, value)| WriteValue {
            node_id: node.clone(),
            attribute_id: AttributeId::Value as u32,
            value: DataValue::value_only(value.clone()),
            ..Default::default()
        })
        .collect();
    tokio::time::timeout(CALL_TIMEOUT, session.write(&to_write))
        .await
        .map_err(|_| OpcError::TimedOut(Call::Write))?
        .map_err(|e| OpcError::failed(Call::Write, &e))
}

/// Close a session within `bound` without ever blocking on it, and end its event loop.
///
/// A graceful `disconnect` sends CloseSession, so the server frees the session's slot (a
/// controller has few), and waits for the reply, which from a vanished peer never arrives.
/// Unbounded, that wait wedges the worker and any stop with it (Veredas 1 met it as silence).
pub(crate) async fn close_quietly(session: &Session, event_loop: EventLoop, bound: Duration) {
    let _ = tokio::time::timeout(bound, session.disconnect()).await;
    drop(event_loop);
}

/// Answer one off-timer job on a live session. A write meets the master switch once more here,
/// just before the wire. A request whose caller has stopped waiting (its future dropped, by a
/// timeout of its own) is not sent at all: nobody expects that pulse any more. A call that fails
/// or times out is returned: the session is suspect.
async fn serve_job(session: &Session, job: Job, gate: &WriteGate) -> Option<OpcError> {
    let abandoned = match &job {
        Job::Write(req) => req.reply.is_closed(),
        Job::Read(req) => req.reply.is_closed(),
    };
    if abandoned {
        return None;
    }
    match job {
        Job::Write(req) => {
            if !gate.is_armed() {
                let _ = req.reply.send(Err(WriteError::Disarmed));
                return None;
            }
            match write_nodes(session, &req.writes).await {
                Ok(statuses) => {
                    let _ = req.reply.send(Ok(statuses));
                    None
                }
                Err(e) => {
                    let _ = req.reply.send(Err(WriteError::Unconfirmed(e.clone())));
                    Some(e)
                }
            }
        }
        Job::Read(req) => match read_nodes(session, &req.nodes).await {
            Ok(reads) => {
                let _ = req.reply.send(Ok(reads));
                None
            }
            Err(e) => {
                let _ = req.reply.send(Err(e.clone()));
                Some(e)
            }
        },
    }
}

/// Plant-local "now" in milliseconds: local wall clock read as if it were UTC, the axis a
/// stamp parsed from a plant's own text lands on.
fn index_now_ms() -> i64 {
    chrono::Local::now().naive_local().and_utc().timestamp_millis()
}

fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed)
}

/// Resolve once the stop flag is raised. Polling, because the flag is a plain atomic shared
/// with callers that are not async, and giving it a notifier would make [`SourceHandle`] know
/// about tokio.
async fn wait_for_stop(stop: &AtomicBool) {
    while !stopped(stop) {
        tokio::time::sleep(STOP_CHECK).await;
    }
}

/// Wait `total` with no session: a stop ends the wait (false), and every job that arrives
/// meanwhile is answered [`OpcError::NotConnected`] at once.
async fn wait_out(total: Duration, stop: &AtomicBool, jobs: &mut UnboundedReceiver<Job>) -> bool {
    let sleep = tokio::time::sleep(total);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            biased;
            _ = wait_for_stop(stop) => return false,
            _ = &mut sleep => return !stopped(stop),
            Some(job) = jobs.recv() => job.refuse(OpcError::NotConnected),
        }
    }
}

/// Connect with no session yet: a stop abandons the attempt (`None`), and every job that arrives
/// meanwhile is answered [`OpcError::NotConnected`] at once.
async fn connect_answering(
    spec: &PollSpec,
    stop: &AtomicBool,
    jobs: &mut UnboundedReceiver<Job>,
) -> Option<Result<(Arc<Session>, EventLoop), OpcError>> {
    let connecting = connect(spec);
    tokio::pin!(connecting);
    loop {
        tokio::select! {
            biased;
            _ = wait_for_stop(stop) => return None,
            r = &mut connecting => return Some(r),
            Some(job) = jobs.recv() => job.refuse(OpcError::NotConnected),
        }
    }
}

/// Why a connected session ended.
enum Ended {
    /// A stop, or the consumer gone: close briefly and end the worker.
    Stop,
    /// The session failed: close briefly, report, wait, reconnect.
    Lost(OpcError),
}

/// The worker's whole life: connect, poll, serve jobs, reconnect, stop.
async fn run(
    spec: Arc<PollSpec>,
    tx: Sender<OpcEvent>,
    jobs: &mut UnboundedReceiver<Job>,
    stop: Arc<AtomicBool>,
    gate: WriteGate,
) -> Result<(), SourceError> {
    let nodes = spec.poll_nodes();
    let mut delays = retry_delays();
    let status = |state: ConnState| OpcEvent::Status { source_id: spec.source_id.clone(), state };

    // A send failure means the consumer is gone: wind down cleanly.
    macro_rules! emit {
        ($ev:expr) => {
            if tx.send($ev).is_err() {
                return Ok(());
            }
        };
    }

    while !stopped(&stop) {
        emit!(status(ConnState::Connecting));
        let (session, mut event_loop) = match connect_answering(&spec, &stop, jobs).await {
            None => return Ok(()),
            Some(Ok(connected)) => connected,
            Some(Err(error)) => {
                let wait = delays.next().unwrap_or(RETRY_MAX);
                emit!(status(ConnState::Retrying { error, in_ms: wait.as_millis() as u64 }));
                if !wait_out(wait, &stop, jobs).await {
                    return Ok(());
                }
                continue;
            }
        };
        delays = retry_delays();
        emit!(status(ConnState::Connected));

        // Connected: poll on the tick, serve jobs in between. A slow server must not queue up
        // missed ticks and then storm the wire catching up.
        let mut ticker = tokio::time::interval(spec.poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let ended = loop {
            tokio::select! {
                // The stop first: with a fast interval the ticker is almost always ready.
                biased;
                _ = wait_for_stop(&stop) => break Ended::Stop,
                status = &mut event_loop.0 => break Ended::Lost(OpcError::ConnectionClosed(status.ok())),
                Some(job) = jobs.recv() => {
                    // Raced against the stop, so a call on a vanished peer does not hold a
                    // stop for its whole timeout. A write dropped here answers `Unconfirmed`.
                    let failed = tokio::select! {
                        biased;
                        _ = wait_for_stop(&stop) => break Ended::Stop,
                        f = serve_job(&session, job, &gate) => f,
                    };
                    if let Some(e) = failed {
                        break Ended::Lost(e);
                    }
                }
                _ = ticker.tick() => {
                    let read = tokio::select! {
                        biased;
                        _ = wait_for_stop(&stop) => break Ended::Stop,
                        r = read_nodes(&session, &nodes) => r,
                    };
                    match read {
                        Ok(values) => {
                            let poll = OpcEvent::Poll {
                                source_id: spec.source_id.clone(),
                                index_ms: index_now_ms(),
                                arrival_ms: now_unix_ms(),
                                values,
                            };
                            if tx.send(poll).is_err() {
                                break Ended::Stop;
                            }
                        }
                        // A failed READ (not a bad per-tag status) means the session is
                        // unhealthy: drop it and reconnect rather than emit silence forever.
                        Err(e) => break Ended::Lost(e),
                    }
                }
            }
        };

        let error = match ended {
            Ended::Stop => {
                close_quietly(&session, event_loop, STOP_CLOSE_TIMEOUT).await;
                return Ok(());
            }
            Ended::Lost(error) => {
                tokio::select! {
                    biased;
                    _ = wait_for_stop(&stop) => {}
                    _ = close_quietly(&session, event_loop, CLOSE_TIMEOUT) => {}
                }
                error
            }
        };
        if stopped(&stop) {
            return Ok(());
        }
        let wait = delays.next().unwrap_or(RETRY_MAX);
        emit!(status(ConnState::Retrying { error, in_ms: wait.as_millis() as u64 }));
        if !wait_out(wait, &stop, jobs).await {
            return Ok(());
        }
    }
    Ok(())
}
