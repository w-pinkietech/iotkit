# MQTT Custody Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. Repository worker rules override the generic skill's
> commit examples: workers do not commit; Main reviews and commits approved checkpoints.

**Goal:** Prove that the Site Server can receive MQTT 3.1.1 QoS 1 publications over mutually
authenticated TLS and emit PUBACK only after an application-controlled durable SQLite commit.

**Architecture:** Start the real `iotkit-site-server` package as a library-only foundation. Use
`mqttbytes-ng` only as a bounded MQTT packet codec; IoTKit owns the connection state machine,
commit queue, disconnect behavior, and exact PUBACK send point. A test-only SQLite custody sink
holds a transaction open so the tests can observe that PUBACK is absent before commit, present
after commit, and absent on failure. This plan does not create the public egress contract, Site
production schema, enrollment, query API, or Gateway publisher.

**Tech Stack:** Rust 2024, Tokio 1, `mqttbytes-ng` 0.7.0 (MQTT 3.1.1 codec only),
`tokio-util::codec`, rustls 0.23, tokio-rustls 0.26, rusqlite 0.32.

## Global Constraints

- MQTT version for this spike is 3.1.1 (`Protocol::V4`) and production publications are QoS 1.
- `mqttbytes-ng` is pinned to `=0.7.0`; changing the version requires rerunning the entire spike.
- The codec input and output limits are both 1 MiB; no packet may allocate beyond the configured
  bound.
- The first packet must be CONNECT, `clean_session` must be true, retained publications are
  rejected, and only `iotkit/v1/gateways/{gateway_identity}/records` is accepted.
- The session allows at most two pending publications and a one-entry commit queue in the spike.
- PUBACK is written only after `CustodySink::commit` returns `Ok(())`.
- Any storage/commit failure closes the connection without PUBACK or a terminal rejection.
- The network loop continues to answer PINGREQ while the commit worker is blocked and the bounded
  queue is occupied.
- TLS requires a client certificate rooted in the configured Gateway trust store. Tailnet
  reachability does not replace mTLS.
- No credential, private key, raw payload, or ticket is formatted through `Debug`, logs, or errors.
- This spike has no mutation API and therefore introduces no alternative to R14 dispatch.
- Full `scripts/verify.sh` is required once at the final spike milestone because this adds Rust
  product code and a new layer classification. Focused tests are used during red/green cycles.

## Evidence behind the candidate

- The rumqttd architecture states that receipt of a PUBLISH appends the message and immediately
  adds PUBACK to its acknowledgement log. That is too early for a Site-owned SQLite transaction:
  <https://docs.rs/crate/rumqttd/0.20.0/source/architecture.md>.
- `mqttbytes-ng` exposes bounded `Packet::read`/`Packet::write` and a Tokio codec without owning
  broker acknowledgement policy: <https://docs.rs/crate/mqttbytes-ng/0.7.0>.
- MQTT 3.1.1 defines PUBACK as the point at which ownership transfers to the receiver. IoTKit
  intentionally delays that permitted acknowledgement until its durable archive commit:
  <https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/mqtt-v3.1.1.pdf>.

---

### Task 1: Classify and scaffold the Site Server package

**Files:**

- Modify: `Cargo.toml`
- Create: `iotkit-site-server/Cargo.toml`
- Create: `iotkit-site-server/src/lib.rs`
- Create: `iotkit-site-server/src/mqtt/mod.rs`
- Modify: `scripts/check-layers`
- Modify: `docs/architecture.md`

**Interfaces:**

- Produces: workspace package `iotkit-site-server`, classified as `SITE`.
- Produces: `iotkit_site_server::mqtt` module boundary used by all later tasks.
- Consumes: no Gateway workspace crate and no adapter crate.

- [ ] **Step 1: Add the unclassified package and prove the layer gate catches it**

Add `"iotkit-site-server"` to the root workspace members and create:

```toml
[package]
name = "iotkit-site-server"
version = "0.1.0"
edition = "2024"

[dependencies]
bytes = "1"
futures-util = { version = "0.3", features = ["sink"] }
mqttbytes-ng = { version = "=0.7.0", features = ["codec"] }
rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12"] }
thiserror = "2"
tokio = { version = "1", features = ["io-util", "net", "rt", "sync", "time"] }
tokio-rustls = { version = "0.26", default-features = false, features = ["ring", "tls12"] }
tokio-util = { version = "0.7", features = ["codec"] }

[dev-dependencies]
rcgen = "0.13"
rusqlite = { version = "0.32", features = ["bundled"] }
tempfile = "3"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "test-util"] }
```

Create `iotkit-site-server/src/lib.rs`:

```rust
//! Site Server transport and composition boundaries.

pub mod mqtt;
```

Create `iotkit-site-server/src/mqtt/mod.rs` temporarily with only:

```rust
//! Custody-aware MQTT transport.
```

Run:

```bash
scripts/check-layers
```

Expected: FAIL with
`iotkit-site-server: new crate is not classified in scripts/check-layers`.

- [ ] **Step 2: Add the SITE classification and rule**

In `scripts/check-layers`, add:

```python
SITE = {"iotkit-site-server"}
EGRESS_CONTRACT = "iotkit-egress-contract"
SITE_ALLOWED = {EGRESS_CONTRACT, STORAGE, TYPES}
```

Include `SITE` in `CLASSIFIED`. After the existing INGRESS check, add:

```python
        if name in SITE:
            forbidden = internal - SITE_ALLOWED
            if forbidden:
                errors.append(
                    f"{name} has workspace dependencies outside the SITE allowlist: "
                    f"{sorted(forbidden)} (rule 9: Site consumes contracts and shared leaves, "
                    "not Gateway internals)"
                )
```

Update the script docstring to list rule 9. Do not add a dependency on the not-yet-created egress
contract merely to exercise the allowlist.

- [ ] **Step 3: Move the package from planned to present in the crate map**

In `docs/architecture.md`:

- change the crate count from twenty-two to twenty-three;
- add `iotkit-site-server` to the present crate table as `SITE [3] package`;
- state that this task creates the MQTT custody transport library foundation and that the binary
  composition root comes with the first executable Site slice;
- remove only the `iotkit-site-server` row from “Approved next-slice placements”; and
- leave `iotkit-egress-contract`, `iotkit-site-store`, and `iotkit-site-serverctl` as planned.

- [ ] **Step 4: Verify the scaffold**

Run:

```bash
cargo check -p iotkit-site-server
scripts/check-layers
git diff --check
```

Expected: all exit 0; `check-layers` reports 23 classified crates.

- [ ] **Step 5: Main checkpoint**

Worker: report the exact files and commands; do not commit. Main: review that no Gateway/internal
dependency entered the package before committing the approved checkpoint.

---

### Task 2: Implement the application-controlled PUBACK session

**Files:**

- Modify: `iotkit-site-server/src/mqtt/mod.rs`
- Create: `iotkit-site-server/src/mqtt/session.rs`
- Create: `iotkit-site-server/tests/mqtt_session.rs`

**Interfaces:**

- Produces: `SessionLimits { max_packet_size, max_inflight, commit_queue }`.
- Produces: `CustodyPublish { client_id, topic, packet_id, duplicate, payload }`.
- Produces: `CustodySink::commit(CustodyPublish) -> CommitFuture`.
- Produces: `run_session(io, sink, limits) -> Result<(), SessionError>`.
- Consumes: an already-authenticated `AsyncRead + AsyncWrite` stream; TLS is Task 4.

- [ ] **Step 1: Write the failing delayed-ack and failure tests**

Create `iotkit-site-server/tests/mqtt_session.rs`. Use a duplex stream and the same codec on both
sides. The test sink must expose deterministic entry/release points without sleeps:

```rust
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iotkit_site_server::mqtt::{
    CommitFailure, CommitFuture, CustodyPublish, CustodySink, SessionLimits, run_session,
};
use mqttbytes_ng::{QoS, v4::{Codec, Connect, Packet, Publish}};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::codec::Framed;

struct GateSink {
    entered: mpsc::UnboundedSender<u16>,
    release: Arc<Semaphore>,
    fail: AtomicBool,
}

impl CustodySink for GateSink {
    fn commit(&self, publish: CustodyPublish) -> CommitFuture<'_> {
        Box::pin(async move {
            self.entered.send(publish.packet_id).unwrap();
            self.release.acquire().await.unwrap().forget();
            if self.fail.load(Ordering::SeqCst) {
                Err(CommitFailure)
            } else {
                Ok(())
            }
        })
    }
}

fn codec() -> Codec {
    Codec { max_incoming_size: 1024 * 1024, max_outgoing_size: 1024 * 1024 }
}

async fn connected_pair(sink: Arc<GateSink>) -> (
    Framed<tokio::io::DuplexStream, Codec>,
    tokio::task::JoinHandle<Result<(), iotkit_site_server::mqtt::SessionError>>,
) {
    let (client, server) = tokio::io::duplex(2 * 1024 * 1024);
    let task = tokio::spawn(run_session(server, sink, SessionLimits::spike()));
    let mut client = Framed::new(client, codec());
    let mut connect = Connect::new("gateway-a");
    connect.clean_session = true;
    client.send(Packet::Connect(connect)).await.unwrap();
    assert!(matches!(client.next().await.unwrap().unwrap(), Packet::ConnAck(_)));
    (client, task)
}

fn publication(packet_id: u16, payload: &'static [u8]) -> Packet {
    let mut publish = Publish::new(
        "iotkit/v1/gateways/gateway-a/records",
        QoS::AtLeastOnce,
        payload,
    );
    publish.pkid = packet_id;
    Packet::Publish(publish)
}
```

Add these three tests:

```rust
#[tokio::test]
async fn puback_is_absent_until_commit_succeeds() {
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let sink = Arc::new(GateSink {
        entered: entered_tx,
        release: release.clone(),
        fail: AtomicBool::new(false),
    });
    let (mut client, server) = connected_pair(sink).await;
    client.send(publication(7, b"record-7")).await.unwrap();
    assert_eq!(entered_rx.recv().await, Some(7));
    assert!(tokio::time::timeout(Duration::from_millis(50), client.next()).await.is_err());
    release.add_permits(1);
    let ack = client.next().await.unwrap().unwrap();
    assert!(matches!(ack, Packet::PubAck(ref ack) if ack.pkid == 7));
    client.send(Packet::Disconnect).await.unwrap();
    assert!(server.await.unwrap().is_ok());
}

#[tokio::test]
async fn commit_failure_closes_without_puback() {
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let sink = Arc::new(GateSink {
        entered: entered_tx,
        release: release.clone(),
        fail: AtomicBool::new(true),
    });
    let (mut client, server) = connected_pair(sink).await;
    client.send(publication(8, b"record-8")).await.unwrap();
    assert_eq!(entered_rx.recv().await, Some(8));
    release.add_permits(1);
    assert!(client.next().await.is_none());
    assert!(server.await.unwrap().is_err());
}

#[tokio::test]
async fn ping_progresses_while_commit_worker_and_queue_are_occupied() {
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let sink = Arc::new(GateSink {
        entered: entered_tx,
        release: release.clone(),
        fail: AtomicBool::new(false),
    });
    let (mut client, server) = connected_pair(sink).await;
    client.send(publication(9, b"record-9")).await.unwrap();
    assert_eq!(entered_rx.recv().await, Some(9));
    client.send(publication(10, b"record-10")).await.unwrap();
    client.send(Packet::PingReq).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_millis(250), client.next()).await.unwrap().unwrap().unwrap(),
        Packet::PingResp
    ));
    release.add_permits(2);
    assert_eq!(entered_rx.recv().await, Some(10));
    let first = client.next().await.unwrap().unwrap();
    let second = client.next().await.unwrap().unwrap();
    assert!(matches!(first, Packet::PubAck(_)));
    assert!(matches!(second, Packet::PubAck(_)));
    client.send(Packet::Disconnect).await.unwrap();
    assert!(server.await.unwrap().is_ok());
}
```

Run:

```bash
cargo test -p iotkit-site-server --test mqtt_session -- --nocapture
```

Expected: compilation fails because the session interfaces do not exist.

- [ ] **Step 2: Define the public transport boundary**

Replace `src/mqtt/mod.rs` with:

```rust
//! Custody-aware MQTT transport.

mod session;

pub use session::{
    CommitFailure, CommitFuture, CustodyPublish, CustodySink, SessionError, SessionLimits,
    run_session,
};
```

In `src/mqtt/session.rs`, define the exact boundary:

```rust
use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use mqttbytes_ng::{Protocol, QoS, v4::{Codec, ConnAck, ConnectReturnCode, Packet, PubAck}};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::codec::Framed;

pub type CommitFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), CommitFailure>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitFailure;

#[derive(Clone, PartialEq, Eq)]
pub struct CustodyPublish {
    pub client_id: String,
    pub topic: String,
    pub packet_id: u16,
    pub duplicate: bool,
    pub payload: Bytes,
}

impl std::fmt::Debug for CustodyPublish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustodyPublish")
            .field("client_id", &self.client_id)
            .field("topic", &self.topic)
            .field("packet_id", &self.packet_id)
            .field("duplicate", &self.duplicate)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

pub trait CustodySink: Send + Sync + 'static {
    fn commit(&self, publish: CustodyPublish) -> CommitFuture<'_>;
}

#[derive(Debug, Clone, Copy)]
pub struct SessionLimits {
    pub max_packet_size: usize,
    pub max_inflight: usize,
    pub commit_queue: usize,
}

impl SessionLimits {
    pub const fn spike() -> Self {
        Self { max_packet_size: 1024 * 1024, max_inflight: 2, commit_queue: 1 }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("mqtt codec: {0}")]
    Codec(#[from] mqttbytes_ng::Error),
    #[error("connection ended before CONNECT")]
    MissingConnect,
    #[error("unsupported or out-of-order MQTT packet")]
    Protocol,
    #[error("publication is outside the authenticated namespace")]
    Namespace,
    #[error("bounded commit capacity exceeded")]
    Capacity,
    #[error("durable custody commit failed")]
    Commit,
    #[error("commit worker stopped")]
    WorkerStopped,
}
```

Do not derive `Debug` for any future TLS private-key holder or credential holder.

- [ ] **Step 3: Implement the session loop and exact ack point**

Continue `session.rs` with a single serial commit worker and a network loop that independently
services packets:

```rust
struct CommitJob {
    packet_id: u16,
    publish: CustodyPublish,
}

struct CommitResult {
    packet_id: u16,
    result: Result<(), CommitFailure>,
}

pub async fn run_session<IO, S>(
    io: IO,
    sink: Arc<S>,
    limits: SessionLimits,
) -> Result<(), SessionError>
where
    IO: AsyncRead + AsyncWrite + Unpin,
    S: CustodySink,
{
    if limits.max_packet_size == 0 || limits.max_inflight == 0 || limits.commit_queue == 0 {
        return Err(SessionError::Capacity);
    }
    let codec = Codec {
        max_incoming_size: limits.max_packet_size,
        max_outgoing_size: limits.max_packet_size,
    };
    let mut framed = Framed::new(io, codec);
    let connect = match framed.next().await {
        Some(Ok(Packet::Connect(connect))) => connect,
        Some(Ok(_)) | Some(Err(_)) => return Err(SessionError::Protocol),
        None => return Err(SessionError::MissingConnect),
    };
    if connect.protocol != Protocol::V4 || !connect.clean_session || connect.client_id.is_empty() {
        return Err(SessionError::Protocol);
    }
    let client_id = connect.client_id;
    framed
        .send(Packet::ConnAck(ConnAck::new(ConnectReturnCode::Success, false)))
        .await?;

    let (job_tx, mut job_rx) = mpsc::channel::<CommitJob>(limits.commit_queue);
    let (result_tx, mut result_rx) = mpsc::channel::<CommitResult>(limits.max_inflight);
    let worker = tokio::spawn(async move {
        while let Some(job) = job_rx.recv().await {
            let result = sink.commit(job.publish).await;
            if result_tx.send(CommitResult { packet_id: job.packet_id, result }).await.is_err() {
                break;
            }
        }
    });

    let mut pending = BTreeSet::new();
    let result = loop {
        tokio::select! {
            packet = framed.next() => match packet {
                Some(Ok(Packet::PingReq)) => framed.send(Packet::PingResp).await?,
                Some(Ok(Packet::Disconnect)) | None => break Ok(()),
                Some(Ok(Packet::Publish(publish))) => {
                    if publish.qos != QoS::AtLeastOnce || publish.retain || publish.pkid == 0 {
                        break Err(SessionError::Protocol);
                    }
                    let expected = format!("iotkit/v1/gateways/{client_id}/records");
                    if publish.topic != expected {
                        break Err(SessionError::Namespace);
                    }
                    if pending.len() >= limits.max_inflight || !pending.insert(publish.pkid) {
                        break Err(SessionError::Capacity);
                    }
                    let packet_id = publish.pkid;
                    let job = CommitJob {
                        packet_id,
                        publish: CustodyPublish {
                            client_id: client_id.clone(),
                            topic: publish.topic,
                            packet_id,
                            duplicate: publish.dup,
                            payload: publish.payload,
                        },
                    };
                    if job_tx.try_send(job).is_err() {
                        break Err(SessionError::Capacity);
                    }
                }
                Some(Ok(_)) | Some(Err(_)) => break Err(SessionError::Protocol),
            },
            completed = result_rx.recv(), if !pending.is_empty() => match completed {
                Some(CommitResult { packet_id, result: Ok(()) }) => {
                    if !pending.remove(&packet_id) {
                        break Err(SessionError::Protocol);
                    }
                    // The only PUBACK construction site: CustodySink has returned after commit.
                    framed.send(Packet::PubAck(PubAck::new(packet_id))).await?;
                }
                Some(CommitResult { result: Err(_), .. }) => break Err(SessionError::Commit),
                None => break Err(SessionError::WorkerStopped),
            }
        }
    };

    drop(job_tx);
    worker.abort();
    let _ = worker.await;
    result
}
```

`mqttbytes_ng::Error` already contains its redacted `std::io::Error` transport variant, so do not
add a second stringified transport error or include packet content.

- [ ] **Step 4: Run the focused tests**

Run:

```bash
cargo test -p iotkit-site-server --test mqtt_session -- --nocapture
```

Expected: all three tests PASS. If the queued publish races before the worker receives the first
job, keep the test's `entered_rx.recv()` barrier; do not add sleeps or increase unbounded capacity.

- [ ] **Step 5: Main checkpoint**

Worker: report the exact test output; do not commit. Main: inspect that `Packet::PubAck` is created
at exactly one code site and only under `CommitResult::Ok`.

---

### Task 3: Prove the ack boundary against a real SQLite transaction and replay

**Files:**

- Create: `iotkit-site-server/tests/mqtt_sqlite_custody.rs`

**Interfaces:**

- Consumes: `CustodySink`, `CustodyPublish`, and `run_session` from Task 2.
- Produces: test-only SQLite `raw_probe` and `cursor_probe` tables.
- Proves: atomic raw+cursor commit, identical replay idempotency, conflicting replay failure.

- [ ] **Step 1: Write the SQLite sink and failing integration assertions**

The test-only schema is deliberately not the future Site production schema:

```sql
CREATE TABLE raw_probe (
    client_id TEXT NOT NULL,
    publication_seq INTEGER NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (client_id, publication_seq)
);
CREATE TABLE cursor_probe (
    client_id TEXT PRIMARY KEY,
    accepted_through INTEGER NOT NULL
);
```

Implement `SqliteProbeSink` in the test using
`Arc<tokio::sync::Mutex<rusqlite::Connection>>`. Its payload format is exactly
`<decimal publication_seq>:<opaque bytes>`. Within `commit`:

1. parse only the decimal prefix;
2. execute `BEGIN IMMEDIATE`;
3. insert the raw row, or verify byte-identical content for an existing key;
4. reject a same-key/different-content replay with `ROLLBACK` and `CommitFailure`;
5. upsert `cursor_probe` only when `publication_seq == accepted_through + 1`;
6. notify the test after writes but before COMMIT and wait on a semaphore;
7. execute `COMMIT`; and
8. return `Ok(())` only after COMMIT returns success.

Use these imports, fields, constructor, and parser:

```rust
use std::{str, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use iotkit_site_server::mqtt::{
    CommitFailure, CommitFuture, CustodyPublish, CustodySink, SessionError, SessionLimits,
    run_session,
};
use mqttbytes_ng::{QoS, v4::{Codec, Connect, Packet, Publish}};
use rusqlite::{Connection, OptionalExtension};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_util::codec::Framed;

struct SqliteProbeSink {
    connection: Mutex<Connection>,
    before_commit: mpsc::UnboundedSender<i64>,
    release: Arc<Semaphore>,
    _directory: tempfile::TempDir,
}

impl SqliteProbeSink {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<i64>) {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join("custody-probe.sqlite3")).unwrap();
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE raw_probe (
                 client_id TEXT NOT NULL,
                 publication_seq INTEGER NOT NULL,
                 payload BLOB NOT NULL,
                 PRIMARY KEY (client_id, publication_seq)
             );
             CREATE TABLE cursor_probe (
                 client_id TEXT PRIMARY KEY,
                 accepted_through INTEGER NOT NULL
             );",
        ).unwrap();
        let (before_commit, receiver) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                connection: Mutex::new(connection),
                before_commit,
                release: Arc::new(Semaphore::new(0)),
                _directory: directory,
            }),
            receiver,
        )
    }
}

fn parse_probe_payload(payload: &[u8]) -> Option<(i64, &[u8])> {
    let separator = payload.iter().position(|byte| *byte == b':')?;
    let sequence = str::from_utf8(&payload[..separator]).ok()?.parse().ok()?;
    Some((sequence, &payload[separator + 1..]))
}
```

The trait implementation must use this shape so the SQLite connection guard stays inside the
boxed future:

```rust
impl CustodySink for SqliteProbeSink {
    fn commit(&self, publish: CustodyPublish) -> CommitFuture<'_> {
        Box::pin(async move {
            let (seq, _) = parse_probe_payload(&publish.payload).ok_or(CommitFailure)?;
            let conn = self.connection.lock().await;
            conn.execute_batch("BEGIN IMMEDIATE").map_err(|_| CommitFailure)?;
            let result = apply_probe_commit(&conn, &publish.client_id, seq, &publish.payload);
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(CommitFailure);
            }
            if self.before_commit.send(seq).is_err() {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(CommitFailure);
            }
            let permit = match self.release.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(CommitFailure);
                }
            };
            permit.forget();
            conn.execute_batch("COMMIT").map_err(|_| CommitFailure)?;
            Ok(())
        })
    }
}
```

Add local test helpers; do not make them part of the product API:

```rust
fn codec() -> Codec {
    Codec { max_incoming_size: 1024 * 1024, max_outgoing_size: 1024 * 1024 }
}

async fn connect_probe(
    sink: Arc<SqliteProbeSink>,
) -> (
    Framed<tokio::io::DuplexStream, Codec>,
    tokio::task::JoinHandle<Result<(), SessionError>>,
) {
    let (client, server) = tokio::io::duplex(2 * 1024 * 1024);
    let task = tokio::spawn(run_session(server, sink, SessionLimits::spike()));
    let mut client = Framed::new(client, codec());
    let mut connect = Connect::new("gateway-a");
    connect.clean_session = true;
    client.send(Packet::Connect(connect)).await.unwrap();
    assert!(matches!(client.next().await.unwrap().unwrap(), Packet::ConnAck(_)));
    (client, task)
}

fn probe_publication(packet_id: u16, duplicate: bool, payload: &'static [u8]) -> Packet {
    let mut publish = Publish::new(
        "iotkit/v1/gateways/gateway-a/records",
        QoS::AtLeastOnce,
        payload,
    );
    publish.pkid = packet_id;
    publish.dup = duplicate;
    Packet::Publish(publish)
}

fn assert_puback(packet: Packet, packet_id: u16) {
    assert!(matches!(packet, Packet::PubAck(ref ack) if ack.pkid == packet_id));
}

async fn probe_state(sink: &SqliteProbeSink) -> (i64, Vec<u8>, i64) {
    let conn = sink.connection.lock().await;
    let count = conn.query_row("SELECT count(*) FROM raw_probe", [], |row| row.get(0)).unwrap();
    let payload = conn.query_row(
        "SELECT payload FROM raw_probe WHERE client_id = 'gateway-a' AND publication_seq = 1",
        [],
        |row| row.get(0),
    ).unwrap();
    let cursor = conn.query_row(
        "SELECT accepted_through FROM cursor_probe WHERE client_id = 'gateway-a'",
        [],
        |row| row.get(0),
    ).unwrap();
    (count, payload, cursor)
}
```

```rust
#[tokio::test]
async fn sqlite_commit_precedes_puback_and_advances_raw_and_cursor_atomically() {
    let (sink, mut before_commit) = SqliteProbeSink::new();
    let (mut client, server) = connect_probe(sink.clone()).await;

    client.send(probe_publication(21, false, b"1:lux=42.5")).await.unwrap();
    assert_eq!(before_commit.recv().await, Some(1));
    assert!(tokio::time::timeout(Duration::from_millis(50), client.next()).await.is_err());

    sink.release.add_permits(1);
    assert_puback(client.next().await.unwrap().unwrap(), 21);
    assert_eq!(probe_state(&sink).await, (1, b"1:lux=42.5".to_vec(), 1));

    client.send(Packet::Disconnect).await.unwrap();
    assert!(server.await.unwrap().is_ok());
}

#[tokio::test]
async fn clean_session_replay_is_idempotent_but_conflicting_content_is_not_acked() {
    let (sink, mut before_commit) = SqliteProbeSink::new();

    let (mut first, first_server) = connect_probe(sink.clone()).await;
    first.send(probe_publication(21, false, b"1:lux=42.5")).await.unwrap();
    assert_eq!(before_commit.recv().await, Some(1));
    sink.release.add_permits(1);
    assert_puback(first.next().await.unwrap().unwrap(), 21);
    first.send(Packet::Disconnect).await.unwrap();
    assert!(first_server.await.unwrap().is_ok());

    let (mut replay, replay_server) = connect_probe(sink.clone()).await;
    replay.send(probe_publication(22, true, b"1:lux=42.5")).await.unwrap();
    assert_eq!(before_commit.recv().await, Some(1));
    sink.release.add_permits(1);
    assert_puback(replay.next().await.unwrap().unwrap(), 22);
    replay.send(Packet::Disconnect).await.unwrap();
    assert!(replay_server.await.unwrap().is_ok());
    assert_eq!(probe_state(&sink).await, (1, b"1:lux=42.5".to_vec(), 1));

    let (mut conflict, conflict_server) = connect_probe(sink.clone()).await;
    conflict.send(probe_publication(23, true, b"1:lux=99.9")).await.unwrap();
    assert!(conflict.next().await.is_none());
    assert!(conflict_server.await.unwrap().is_err());
    assert_eq!(probe_state(&sink).await, (1, b"1:lux=42.5".to_vec(), 1));
}
```

Run:

```bash
cargo test -p iotkit-site-server --test mqtt_sqlite_custody -- --nocapture
```

Expected before completing the helper functions: FAIL to compile. Do not weaken the assertions to
observe only in-memory flags.

- [ ] **Step 2: Complete the transaction helpers minimally**

Use parameterized rusqlite statements only. `apply_probe_commit` must implement identity checking
and contiguous cursor movement in the same open transaction:

```rust
fn apply_probe_commit(
    conn: &rusqlite::Connection,
    client_id: &str,
    seq: i64,
    payload: &[u8],
) -> Result<(), CommitFailure> {
    let existing = conn.query_row(
        "SELECT payload FROM raw_probe WHERE client_id = ?1 AND publication_seq = ?2",
        rusqlite::params![client_id, seq],
        |row| row.get::<_, Vec<u8>>(0),
    ).optional().map_err(|_| CommitFailure)?;
    match existing {
        Some(existing) if existing != payload => return Err(CommitFailure),
        Some(_) => {}
        None => {
            conn.execute(
                "INSERT INTO raw_probe(client_id, publication_seq, payload) VALUES (?1, ?2, ?3)",
                rusqlite::params![client_id, seq, payload],
            ).map_err(|_| CommitFailure)?;
        }
    }
    let cursor = conn.query_row(
        "SELECT accepted_through FROM cursor_probe WHERE client_id = ?1",
        [client_id],
        |row| row.get::<_, i64>(0),
    ).optional().map_err(|_| CommitFailure)?.unwrap_or(0);
    if seq == cursor + 1 {
        conn.execute(
            "INSERT INTO cursor_probe(client_id, accepted_through) VALUES (?1, ?2)
             ON CONFLICT(client_id) DO UPDATE SET accepted_through = excluded.accepted_through",
            rusqlite::params![client_id, seq],
        ).map_err(|_| CommitFailure)?;
    } else if seq > cursor {
        return Err(CommitFailure);
    }
    Ok(())
}
```

The constructor above configures `WAL + synchronous=FULL` before creating the probe schema. Do not
replace the file-backed database with `open_in_memory`, where WAL behavior would not be representative.

- [ ] **Step 3: Run the SQLite custody tests**

Run:

```bash
cargo test -p iotkit-site-server --test mqtt_sqlite_custody -- --nocapture
```

Expected: both tests PASS. The output must not contain payload bytes or credentials.

- [ ] **Step 4: Main checkpoint**

Worker: report the transaction boundaries and output; do not commit. Main: confirm the cursor write
and raw insert occur before the same COMMIT, and that every storage-error path returns no ack.

---

### Task 4: Prove mutually authenticated TLS admission

**Files:**

- Modify: `iotkit-site-server/src/mqtt/mod.rs`
- Create: `iotkit-site-server/src/mqtt/tls.rs`
- Create: `iotkit-site-server/tests/mqtt_mtls.rs`

**Interfaces:**

- Produces: `mtls_server_config(cert_chain, private_key, client_roots)`.
- Produces: `accept_mtls(stream, config)` with a five-second handshake timeout.
- Consumes: rustls DER certificate/key types; never path strings or secret-bearing errors.

- [ ] **Step 1: Write the failing mTLS tests**

Create a test CA with rcgen, then issue one server certificate for `localhost` and one Gateway
client certificate. Use:

```rust
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use iotkit_site_server::mqtt::{
    CommitFuture, CustodyPublish, CustodySink, SessionLimits, accept_mtls, mtls_server_config,
    run_session,
};
use mqttbytes_ng::v4::{Codec, Connect, Packet};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
use tokio_rustls::TlsConnector;
use tokio_util::codec::Framed;

struct TestCertificates {
    ca: CertificateDer<'static>,
    server: CertificateDer<'static>,
    server_key: Vec<u8>,
    client: CertificateDer<'static>,
    client_key: Vec<u8>,
}

fn certificate_set() -> TestCertificates {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(vec!["iotkit-test-ca".into()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let server_key = KeyPair::generate().unwrap();
    let server = CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .signed_by(&server_key, &ca, &ca_key)
        .unwrap();
    let client_key = KeyPair::generate().unwrap();
    let client = CertificateParams::new(vec!["gateway-a".into()])
        .unwrap()
        .signed_by(&client_key, &ca, &ca_key)
        .unwrap();
    TestCertificates {
        ca: ca.der().clone(),
        server: server.der().clone(),
        server_key: server_key.serialize_der(),
        client: client.der().clone(),
        client_key: client_key.serialize_der(),
    }
}
```

The helper stores only public certificate DER and raw PKCS#8 bytes so each rustls config constructs
its own owned key wrapper.

Add exact config helpers and an immediate sink:

```rust
fn server_config(certs: &TestCertificates) -> Arc<ServerConfig> {
    let mut client_roots = RootCertStore::empty();
    client_roots.add(certs.ca.clone()).unwrap();
    mtls_server_config(
        vec![certs.server.clone()],
        PrivatePkcs8KeyDer::from(certs.server_key.clone()).into(),
        client_roots,
    ).unwrap()
}

fn authenticated_client_config(certs: &TestCertificates) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(certs.ca.clone()).unwrap();
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![certs.client.clone()],
                PrivatePkcs8KeyDer::from(certs.client_key.clone()).into(),
            )
            .unwrap(),
    )
}

fn anonymous_client_config(certs: &TestCertificates) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(certs.ca.clone()).unwrap();
    Arc::new(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
}

struct ImmediateSink;

impl CustodySink for ImmediateSink {
    fn commit(&self, _: CustodyPublish) -> CommitFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
```

Add the complete handshake tests using an in-memory duplex transport:

```rust
#[tokio::test]
async fn trusted_gateway_certificate_reaches_mqtt_connack() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs = certificate_set();
    let server_config = server_config(&certs);
    let client_config = authenticated_client_config(&certs);
    let (client_io, server_io) = tokio::io::duplex(2 * 1024 * 1024);

    let server = tokio::spawn(async move {
        let tls = accept_mtls(server_io, server_config).await.unwrap();
        run_session(tls, Arc::new(ImmediateSink), SessionLimits::spike()).await
    });
    let connector = TlsConnector::from(client_config);
    let name = ServerName::try_from("localhost").unwrap();
    let tls = connector.connect(name, client_io).await.unwrap();
    let mut mqtt = Framed::new(tls, Codec {
        max_incoming_size: 1024 * 1024,
        max_outgoing_size: 1024 * 1024,
    });
    let mut connect = Connect::new("gateway-a");
    connect.clean_session = true;
    mqtt.send(Packet::Connect(connect)).await.unwrap();
    assert!(matches!(mqtt.next().await.unwrap().unwrap(), Packet::ConnAck(_)));
    mqtt.send(Packet::Disconnect).await.unwrap();
    assert!(server.await.unwrap().is_ok());
}

#[tokio::test]
async fn missing_gateway_certificate_is_rejected_before_mqtt() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs = certificate_set();
    let server_config = server_config(&certs);
    let client_config = anonymous_client_config(&certs);
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);

    let server = tokio::spawn(async move { accept_mtls(server_io, server_config).await });
    let connector = TlsConnector::from(client_config);
    let name = ServerName::try_from("localhost").unwrap();
    assert!(connector.connect(name, client_io).await.is_err());
    assert!(server.await.unwrap().is_err());
}
```

Run:

```bash
cargo test -p iotkit-site-server --test mqtt_mtls -- --nocapture
```

Expected: compilation fails because the TLS helpers do not exist.

- [ ] **Step 2: Implement the mTLS-only server configuration**

Create `src/mqtt/tls.rs`:

```rust
use std::{sync::Arc, time::Duration};

use rustls::{RootCertStore, ServerConfig, server::WebPkiClientVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("Gateway trust store is empty")]
    EmptyClientRoots,
    #[error("invalid Gateway client-certificate verifier")]
    ClientVerifier,
    #[error("invalid Site certificate/private-key pair")]
    SiteIdentity,
    #[error("TLS handshake failed")]
    Handshake,
    #[error("TLS handshake timed out")]
    Timeout,
}

pub fn mtls_server_config(
    cert_chain: Vec<CertificateDer<'static>>,
    private_key: PrivateKeyDer<'static>,
    client_roots: RootCertStore,
) -> Result<Arc<ServerConfig>, TlsError> {
    if client_roots.is_empty() {
        return Err(TlsError::EmptyClientRoots);
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .map_err(|_| TlsError::ClientVerifier)?;
    let config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert_chain, private_key)
        .map_err(|_| TlsError::SiteIdentity)?;
    Ok(Arc::new(config))
}

pub async fn accept_mtls<IO>(
    io: IO,
    config: Arc<ServerConfig>,
) -> Result<TlsStream<IO>, TlsError>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, TlsAcceptor::from(config).accept(io))
        .await
        .map_err(|_| TlsError::Timeout)?
        .map_err(|_| TlsError::Handshake)
}
```

Export `accept_mtls`, `mtls_server_config`, and `TlsError` from `mqtt/mod.rs`. The errors are
intentionally non-diagnostic at this boundary so certificate or key material is never formatted.

- [ ] **Step 3: Run the mTLS tests**

Run:

```bash
cargo test -p iotkit-site-server --test mqtt_mtls -- --nocapture
```

Expected: both tests PASS. Confirm the negative connection never reaches `CustodySink::commit`.

- [ ] **Step 4: Main checkpoint**

Worker: report results; do not commit. Main: confirm there is no plaintext listener helper in the
product API and no `Debug` implementation exposes `PrivateKeyDer`.

---

### Task 5: Decide the spike and record evidence

**Files:**

- Create on success: `docs/superpowers/spikes/2026-07-13-mqtt-custody-result.md`
- Modify only if implementation facts differ: `docs/superpowers/specs/2026-07-13-minimum-gateway-site-server-design.md`

**Interfaces:**

- Consumes: all focused tests from Tasks 2–4 and the full repository verifier.
- Produces: an explicit GO decision for the next `egress-contract + Site raw store` plan, or a NO-GO
  report with the failing invariant.

- [ ] **Step 1: Run formatter and focused spike verification**

Run:

```bash
cargo fmt --all --check
cargo test -p iotkit-site-server --test mqtt_session -- --nocapture
cargo test -p iotkit-site-server --test mqtt_sqlite_custody -- --nocapture
cargo test -p iotkit-site-server --test mqtt_mtls -- --nocapture
cargo clippy -p iotkit-site-server --all-targets -- -D warnings
scripts/check-layers
git diff --check
```

Expected: every command exits 0. A failure in delayed PUBACK, failure-without-ack, bounded keepalive,
replay/conflict handling, or mTLS is a NO-GO; do not continue by weakening the design.

- [ ] **Step 2: Run the single broad milestone verification**

Run:

```bash
scripts/verify.sh
```

Expected: fmt, layer rules, all workspace tests, and Clippy `-D warnings` pass. This is the only broad
workspace run in the spike; do not add unrelated stress suites.

- [ ] **Step 3: Record a successful GO result**

Only if every required command passed, create the result document with this exact content:

```markdown
# MQTT custody spike result

Date: 2026-07-13
Verdict: **GO**

## Proven

- `mqttbytes-ng = 0.7.0` leaves PUBACK emission under IoTKit application control.
- A QoS 1 PUBACK is absent while the SQLite transaction is open and appears only after COMMIT.
- SQLite failure and same-key/different-content conflict close the connection without PUBACK.
- Identical clean-session replay is idempotently acknowledged.
- PINGREQ is serviced while one commit is blocked and one bounded job is queued.
- A client without a certificate rooted in the configured Gateway trust store is rejected during
  TLS handshake before MQTT processing.

## Verification evidence

- `cargo test -p iotkit-site-server --test mqtt_session`: PASS
- `cargo test -p iotkit-site-server --test mqtt_sqlite_custody`: PASS
- `cargo test -p iotkit-site-server --test mqtt_mtls`: PASS
- `cargo clippy -p iotkit-site-server --all-targets -- -D warnings`: PASS
- `scripts/check-layers`: PASS (23 classified crates)
- `scripts/verify.sh`: PASS

## Scope boundary

The SQLite tables and payload format are test probes, not the Site production schema or R10 wire
contract. The next plan owns `iotkit-egress-contract`, the production raw archive/cursor transaction,
and the first executable Site listener.
```

If any invariant fails, do not write a GO file. Report the exact failing command, invariant, and
observed packet/transaction ordering to the user; selection of another codec or a design change is a
new decision.

- [ ] **Step 4: Self-review against the umbrella spec**

Confirm all six spike gates from design section 6 are represented:

1. post-commit PUBACK;
2. disconnect after commit failure;
3. bounded inflight and commit queue;
4. keepalive progress under backpressure;
5. TLS client-certificate verification; and
6. clean-session replay with idempotent custody.

Also confirm this plan did not implement enrollment, credentials, Site query, projection, backup,
Gateway MQTT publishing, or the final canonical egress schema.

- [ ] **Step 5: Main checkpoint**

Worker: do not commit. Main: review the evidence, then make one intentional spike commit if GO. Do
not push, open a PR, merge, or start the next plan without the corresponding user authorization.

## Execution handoff

After this plan is saved, execute it either with:

1. **Subagent-Driven** — fresh implementer per task and Main review between tasks; or
2. **Inline Execution** — execute in this session in batches with checkpoints.

The repository role intent is Luna/max for implementers/executors and Sol/high for Main/reviewer
when native role dispatch supports model selection.
