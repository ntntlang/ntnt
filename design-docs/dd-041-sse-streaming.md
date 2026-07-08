# DD-041: Real-Time Streaming — SSE & Broadcast Channels

**Status:** Draft — design review 2026-07-08 (see § Design Review below); awaiting maintainer go/no-go for Phase 1
**Author:** Larri
**Created:** 2026-03-17
**Branch:** TBD
**Depends on:** DD-037 (concurrency primitives — `schedule`, `channel`)

---

## Table of Contents

1. [Design Review (2026-07-08)](#design-review-2026-07-08)
2. [Vision](#vision)
3. [Architecture](#architecture)
4. [API Design](#api-design)
5. [Full Stack Examples](#full-stack-examples)
6. [Implementation Notes](#implementation-notes)
7. [Phase Details](#phase-details)
8. [Relationship to std/events and std/jobs](#relationship-to-stdevents-and-stdjobs)
9. [Security](#security)
10. [Competitive Analysis](#competitive-analysis)
11. [Open Questions](#open-questions)
12. [Version History](#version-history)

---

## Design Review (2026-07-08)

The March draft was verified against current main (post multi-worker
hardening, DD-057/DD-063). The architecture holds; three assumptions do
not, and each changes the plan:

**1. There is no `respond` keyword.** The draft's "`respond` already
handles `return file(...)`" is drift — handlers return response VALUES
(`json()`, `html()`). This simplifies the design: `sse()` and
`sse_stream()` are ordinary response builders and Phase 1 needs zero
parser work:

```ntnt
import { broadcast, subscribe, sse } from "std/sse"

let metrics_bus = broadcast("metrics")

get("/metrics/stream", fn(req) {
    return sse(subscribe(metrics_bus))
})
```

The body of this document — API sections, all examples, implementation
notes, and the Phase 1 checklist — has been reconciled to this form and
to the decisions below.

**2. The bridge buffers whole responses.** `BridgeResponse` is
`{ status, headers, body: String }` — returned over a oneshot channel and
written once. SSE needs the one real structural change: a body variant
(`BridgeBody::Text(String) | BridgeBody::Sse { subscription_id }`). When
the Axum layer sees the SSE variant it builds a streaming body (the
draft's write-task design, unchanged: keep-alive comments, disconnect
cleanup, `X-Accel-Buffering: no`). The interpreter thread still returns
immediately — the zero-interpreter-overhead property survives contact
with the bridge.

**3. Multi-worker breaks the naive design — named broadcasts fix it.**
Production defaults to `min(num_cpus, 8)` workers, each re-evaluating the
program. Two consequences the draft predates:

- A module-level `let bus = broadcast()` creates N DISTINCT buses (one
  per worker). A subscriber lands on whichever worker served its request.
- `schedule()` is already gated correctly — Worker-mode interpreters lack
  `RuntimeCapability::Scheduling`, so a module-level sampler registers
  exactly once, on the primary interpreter (verified in source:
  `schedule`/`after` carry `requires: Some(RuntimeCapability::Scheduling)`
  and the Worker and HotReload capability sets exclude it; pinned by
  `test_capability_gate_scheduling_worker_mode_skips`). The March draft
  feared N× duplicate samplers; the multi-worker hardening already
  prevents that. But the combination is exactly the trap: the single
  sampler publishes on the PRIMARY interpreter's bus, while subscribers'
  handlers run on workers holding their own per-worker buses — so with
  anonymous module-level buses, subscribers receive nothing at all.

Resolution, now part of Phase 1:

- **Broadcasts are named and process-global**: `broadcast("metrics")`
  returns the same bus for the same name from every worker (registry
  keyed by name, same `LazyLock<Mutex<HashMap>>` pattern as the pool
  registry). Named buses also survive hot reload — a re-eval re-attaches
  to the existing bus instead of orphaning open connections.
  Anonymous `broadcast()` remains for single-connection/`sse_stream`
  patterns and is documented as per-worker.
- **No `schedule()` change needed** — the Worker-mode gate already
  exists. Phase 1 adds a multi-worker integration test (sampler on
  primary, subscribers via workers, events arrive once) to pin the
  behavior, not new gating code.

**Open questions resolved (recommendations):**

- Backing: each subscriber gets a **bounded ring buffer**
  (`Arc<Mutex<VecDeque>>`, cap ~1024) plus a coalescing flume `bounded(1)`
  wake signal. The ring — not a channel — is required because the default
  policy is drop-**oldest** (evict the stalest event so a slow client
  catches up to *now*), and neither flume nor crossbeam lets the sending
  end pop the oldest queued item; only the receiver drains. flume carries
  the payload-free wake so the Axum write task can `recv_async()` (flume
  is already a bridge dependency; crossbeam has no async recv). Bounded
  rather than unbounded: a stuck TCP connection with an unbounded queue is
  slow-motion OOM.
  Default policy drop-oldest with a deduped warn; `"drop_slow": false`
  opts into blocking sends for correctness-critical streams.
- `send()` reuses the existing dispatch on handle type — no new verb.
- `sse_stream` cleanup stays `on_close(fn)`; `push()` returns Bool so
  generators can stop on disconnect.
- WebSockets remain a separate DD; `ntnt sse status` stays Phase 3.

Phase 1 effort holds at 3-4 days with the one structural addition (the
BridgeBody variant). Awaiting go/no-go.

---

## Vision

ntnt already has `schedule()` for periodic sampling and channels for in-process communication. The missing piece for real-time dashboards — compute metrics, network monitoring, live logs, progress bars — is a way to push data from those samplers to a browser the moment it's collected.

**The goal:** make real-time dashboards trivially buildable in ntnt. One broadcast channel, one schedule loop, one SSE endpoint. No WebSocket server, no message broker, no external process. Three primitives:

1. **`broadcast()`** — a fan-out channel: every subscriber gets every message
2. **`subscribe(bc)`** — tap into a broadcast for one connection
3. **`return sse(...)`** — hold the HTTP connection open, push events as they arrive

The full real-time stack then looks like:
```
schedule(500ms) → send(metrics_bus) → [N browser connections] each subscribed via SSE
```

**Design principles:**
- **Single sampler, N clients** — one `schedule()` loop feeds all connected browsers. Not one loop per connection.
- **No interpreter overhead in the push path** — once the SSE response is established, the Rust HTTP layer handles writes directly from the broadcast channel receiver. The interpreter is not involved.
- **Works with auth** — SSE endpoints go through the same route middleware as any other route.
- **Graceful disconnect** — when a client disconnects, their subscription is dropped. No leaked threads or channels.
- **Composable with existing primitives** — `schedule`, `channel`, `spawn` all work alongside SSE without special cases.

---

## Architecture

### The Problem with ntnt's Current Channel Model

`channel()` creates a `TxChannelHandle`/`RxChannelHandle` pair backed by a crossbeam unbounded channel — one producer, one consumer (or multiple producers, one consumer via clone). Every `recv()` call removes the message from the queue. This is wrong for SSE: you want every connected browser to receive every metric sample, not one browser to receive each sample in round-robin.

### The New Primitive: Broadcast Channel

`broadcast()` creates a broadcast channel backed by a fan-out bus. Each subscriber gets their own bounded per-connection queue that receives a copy of every message sent to the bus. Messages are not consumed by one subscriber at the expense of others — they're fanned out.

```
broadcast_channel
    ├── subscriber_1 (browser tab A) → SSE connection
    ├── subscriber_2 (browser tab B) → SSE connection
    └── subscriber_3 (browser tab C) → SSE connection

send(bc, value) → all three subscribers receive a copy
```

**Implementation options (see Open Questions):**
- `bus` crate — lock-free SPMC broadcast, fastest, single-producer only
- `tokio::sync::broadcast` — MPMC, has lag/drop semantics, skip-behind
- Custom: per-subscriber bounded ring buffer + coalescing wake signal, added per subscriber, cleaned up on disconnect

The custom approach won — see the Design Review. A ring buffer (not a channel) is needed because the default backpressure policy is drop-**oldest**, a sender-side eviction that channels don't allow; a payload-free flume `bounded(1)` wake lets the Axum write task `recv_async()` and then drain (flume is already the bridge's library; crossbeam has no async recv).

### How the `sse()` Response Works

When a route handler returns `sse(subscription)`:

1. The Rust HTTP layer recognizes the SSE response type.
2. It sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`, connection stays open.
3. A Rust task (not an interpreter thread) awaits the subscription's wake signal (`recv_async()`), then drains the per-connection ring buffer, formatting each `Value` as an SSE event and writing to the HTTP response body.
4. When the client disconnects, the write fails, the loop exits, and the subscription is dropped — which removes this subscriber from the broadcast channel's sender list.
5. The interpreter thread is free after returning the SSE response token. It does not block.

This means SSE adds **zero interpreter-thread overhead** after the route handler completes.

### Keep-Alive

SSE connections die silently when passing through proxies and load balancers that close idle connections. The SSE writer task sends a keep-alive comment (`:\n\n`) every 15 seconds (configurable). Browsers ignore comments; proxies see activity and keep the connection open.

### Connection Registration Flow

```
Browser → GET /metrics/stream
  → Route handler runs (interpreter thread)
  → subscribe(metrics_bus) → creates bounded ring buffer + wake signal,
    registers the entry in BROADCAST_REGISTRY, parks the subscription
    (ring + wake receiver) in SUBSCRIPTION_REGISTRY under a subscription_id
  → return sse(subscription) → BridgeBody::Sse { subscription_id } to HTTP layer
  → HTTP layer take()s the subscription, spawns write task:
    recv_async() on the wake signal → drain ring buffer → write to socket
  → Route handler exits, interpreter thread free

schedule(500ms) loop (separate thread):
  → send(metrics_bus, value) → briefly locks BROADCAST_REGISTRY to snapshot
    the subscriber list, then (outside the lock) evaluates any filter
    predicates and, per subscriber, enqueues into the ring (drop-oldest if
    full) and signals its wake
  → Each write task wakes, drains the ring, writes SSE events to socket
```

---

## API Design

### Phase 1: Core

```ntnt
import { broadcast, subscribe, sse } from "std/sse"
import { send } from "std/concurrent"
import { now_millis } from "std/time"

// Create a NAMED broadcast channel (module/app level). The name makes the
// bus process-global: every worker and every hot-reload re-evaluation
// resolves the same bus (Design Review).
let metrics_bus = broadcast("metrics")

// Send a value to all current subscribers
send(metrics_bus, map { "cpu": 87.3, "ts": now_millis() })

// In a route handler: return the SSE response; the connection stays open
// and events stream as they arrive
get("/metrics/stream", fn(req) {
    return sse(subscribe(metrics_bus))
})
```

**Types:**
- `broadcast(name?: String) -> BroadcastHandle` — named: process-global; anonymous: per-worker
- `subscribe(BroadcastHandle) -> SSESubscription`
- `send(BroadcastHandle, Map) -> Bool` — reuses existing `send()` dispatch on handle type; returns Bool like channel `send()` (true if the bus had at least one subscriber)
- `sse(SSESubscription) -> Response` — ordinary response builder (like `json()`/`html()`)

### Per-Connection Streams (No Shared Bus)

For streams that are unique per connection — a clock, a personal notification feed, a progress bar — use the callback form:

```ntnt
import { sse_stream } from "std/sse"
import { now, format } from "std/time"
import { schedule, cancel_schedule } from "std/concurrent"
import { job_status } from "std/jobs"

get("/clock", fn(req) {
    return sse_stream(fn(push, on_close) {
        let sched = schedule(1000, fn() {
            push(map { "time": format(now(), "%H:%M:%S") })
        })
        on_close(fn() { cancel_schedule(sched) })
    })
})

get("/jobs/{id}/progress", fn(req) {
    let job_id = req.params.id
    return sse_stream(fn(push, on_close) {
        let poll = schedule(500, fn() {
            let status = unwrap(job_status(job_id))
            push(status)
            if includes(["completed", "failed", "dead"], status["status"]) {
                // Terminal event: the browser closes the EventSource on
                // "done", which fires on_close and cancels the poll
                push(map { "done": true })
            }
        })
        on_close(fn() { cancel_schedule(poll) })
    })
})
```

`sse_stream(fn(push, on_close))`:
- `push(value)` — send an event to this connection; returns `false` after the client disconnects so generators can stop
- `on_close(fn)` — register a cleanup callback for when the client disconnects
- Returns a `Response` immediately, like `sse()` — the handler exits and the connection streams until the client disconnects

> **Implementation note — on_close execution point:** the same constraint as
> filter predicates applies: an ntnt closure cannot run on the Rust write
> task. Disconnect is detected in the write task, which enqueues the
> registered `on_close` callback onto the scheduler's interpreter thread
> (the same thread that runs `schedule()` callbacks) for execution.

> **Implementation note — the generator runs in a scheduling-capable
> context, NOT the worker request scope.** A route handler executes on a
> worker interpreter, and Worker mode deliberately lacks
> `RuntimeCapability::Scheduling` — so a bare `schedule()` in a worker
> handler is silently skipped. That gate exists to stop *module-level*
> samplers from registering N times (once per worker) during program
> load; it must NOT disable per-connection timers, or the clock and
> progress-bar streams above would silently never tick under multi-worker.
> Resolution: `sse_stream` hands the generator off (like the `sse()`
> subscription handoff) to run in a per-connection execution scope that
> grants `Scheduling`. `schedule()`/`cancel_schedule()` inside the
> generator therefore register normally, and the cardinality is correct by
> construction — one timer per open connection, not one per worker. (A
> module-level `schedule()` at app top level is still skipped on workers,
> unchanged.)

### Phase 2: Named Events, IDs, Filtering

**Named events** — SSE supports `event:` fields; the browser can listen selectively:

```ntnt
// Server
send(metrics_bus, "cpu_update", map { "value": 87.3 })
send(metrics_bus, "mem_update", map { "value": 4096 })

// Browser
es.addEventListener('cpu_update', e => updateCpuGauge(JSON.parse(e.data)))
es.addEventListener('mem_update', e => updateMemChart(JSON.parse(e.data)))
```

`send(handle, event_name, value)` — two-arg form emits a named SSE event.

**Filtering** — create a derived subscription that only passes matching events:

```ntnt
import { subscribe, filter, sse } from "std/sse"

get("/stream/network/{iface}", fn(req) {
    let iface = req.params.iface
    return sse(filter(subscribe(metrics_bus), fn(m) { m["interface"] == iface }))
})
```

`filter(SSESubscription, fn(Map) -> Bool) -> SSESubscription`

Filtering happens server-side before writing to the socket — non-matching events are dropped, never sent over the wire. Execution point: the predicate is an ntnt closure, which the Rust write task cannot evaluate — so filtered subscriptions register their predicate with the bus and it runs at `send()` fan-out time, on the sending interpreter thread — outside the registry lock, against a snapshot of the subscriber list (see the `send()` algorithm in Implementation Notes), so a predicate may itself call `send()` safely. The write task stays interpreter-free (the zero-overhead property holds); the cost of filtering lands on the publisher, proportional to filtered subscribers.

**Event IDs and reconnect:**

```ntnt
// Server assigns IDs automatically when replay is enabled
let metrics_bus = broadcast("metrics", map { "replay_buffer": 100 })

// Browser automatically sends Last-Event-ID on reconnect
// ntnt replays the buffer from that ID forward
```

`broadcast(name?, opts?)`:
- `"replay_buffer": N` — keep the last N events in a ring buffer; replayed on reconnect
- `"drop_slow": false` — opt INTO briefly-blocking sends for correctness-critical streams. The default is `true` (drop-oldest from the subscriber's bounded queue, with a deduped warn) — per the Design Review, blocking on a stuck subscriber is the OOM/backpressure hazard

### Full API Surface

```ntnt
import {
    broadcast,          // create a broadcast channel
    subscribe,          // tap into a broadcast (per-connection)
    filter,             // filter a subscription by predicate
    sse,                // SSE response builder for a subscription (primary form)
    sse_stream,         // per-connection stream with push/on_close callbacks
    connection_count,   // how many active subscribers on a broadcast channel
} from "std/sse"
```

Response forms:
```ntnt
return sse(subscription)                    // push from shared bus
return sse_stream(fn(push, on_close) { })   // per-connection generator
```

---

## Full Stack Examples

### Compute Dashboard

```ntnt
import { broadcast, subscribe, sse } from "std/sse"
import { html } from "std/http/server"
import { schedule, send } from "std/concurrent"
import { now_millis } from "std/time"
import { read_file } from "std/fs"

let metrics = broadcast("metrics")

// One sampler loop, feeds all connected browser tabs
// (cpu_percent/mem_used_mb/load_avg are hypothetical samplers)
schedule(500, fn() {
    send(metrics, map {
        "cpu":    cpu_percent(),
        "mem_mb": mem_used_mb(),
        "load":   load_avg(),
        "ts":     now_millis(),
    })
})

get("/dashboard", fn(req) {
    return html(read_file("dashboard.html")?)
})

get("/stream/metrics", fn(req) {
    return sse(subscribe(metrics))
})
```

Frontend (40 lines of vanilla JS):
```js
const es = new EventSource('/stream/metrics')
es.onmessage = e => {
    const m = JSON.parse(e.data)
    document.getElementById('cpu').textContent = m.cpu.toFixed(1) + '%'
    document.getElementById('mem').textContent = m.mem_mb + ' MB'
    updateChart(m)
}
```

---

### Network Monitoring Dashboard

```ntnt
import { broadcast, subscribe, filter, sse } from "std/sse"
import { schedule, send } from "std/concurrent"
// std/events is planned, not shipped (DD-037 Phase 7: pub/sub fan-out
// over the job system) — shown here to illustrate how the layers compose
import { subscribe as event_subscribe, publish } from "std/events"

// Named buses: process-global, shared across workers and hot reloads
let net_metrics = broadcast("net_metrics")
let net_alerts  = broadcast("net_alerts")

// Interface sampler
schedule(1000, fn() {
    let ifaces = sample_interfaces()  // returns array of interface maps
    for iface in ifaces {
        send(net_metrics, iface)

        // Publish structured event for job-based alerting
        if iface["status"] == "down" {
            publish("interface.down", iface)
        }
        if iface["rx_errors"] > 100 {
            publish("interface.errors_high", iface)
        }
    }
})

// Alert sampler (separate cadence)
schedule(5000, fn() {
    let dropped = check_packet_drops()
    if dropped["rate"] > 0.01 {
        send(net_alerts, map { "type": "packet_loss", "rate": dropped["rate"] })
    }
})

// All interfaces — live feed
get("/stream/network", fn(req) {
    return sse(subscribe(net_metrics))
})

// Per-interface filtered stream
get("/stream/network/{iface}", fn(req) {
    let iface = req.params.iface
    return sse(filter(subscribe(net_metrics), fn(m) { m["interface"] == iface }))
})

// Alert stream
get("/stream/alerts", fn(req) {
    return sse(subscribe(net_alerts))
})

// Job-based alerting wired through std/events
event_subscribe("interface.down",      "PageOnCall")
event_subscribe("interface.down",      "LogIncident")
event_subscribe("interface.errors_high", "SendSlackAlert")
```

The monitoring stack: `schedule` samples → SSE streams live to browsers + `std/events` fires jobs for persistent alerting. Separate concerns, one coherent system.

---

### Job Progress Bar

```ntnt
import { sse_stream } from "std/sse"
import { job_status } from "std/jobs"
import { schedule, cancel_schedule } from "std/concurrent"

get("/jobs/{id}/progress", fn(req) {
    let job_id = req.params.id
    return sse_stream(fn(push, on_close) {
        let poll = schedule(500, fn() {
            let status = unwrap(job_status(job_id))
            push(status)
            if includes(["completed", "failed", "dead"], status["status"]) {
                // Terminal event: the browser closes the EventSource on
                // "done", which fires on_close and cancels the poll
                push(map { "done": true })
            }
        })
        on_close(fn() { cancel_schedule(poll) })
    })
})
```

---

### Log Tail

```ntnt
import { broadcast, subscribe, filter, sse } from "std/sse"
import { send } from "std/concurrent"

let log_bus = broadcast("logs")

// Hook into ntnt's log system (or tail a file)
// on_log() is illustrative — ntnt has no log-hook API yet; a real
// version of this example would need one (or a tail-file sampler on
// schedule())
on_log(fn(entry) {
    send(log_bus, entry)
})

get("/stream/logs", fn(req) {
    return sse(subscribe(log_bus))
})

get("/stream/logs/{level}", fn(req) {
    let level = req.params.level
    return sse(filter(subscribe(log_bus), fn(e) { e["level"] == level }))
})
```

---

## Implementation Notes

### BroadcastHandle and BroadcastRegistry

```rust
// Global registry of all broadcast channels — process-wide so every
// worker (and hot-reload re-evaluation) resolves the same named bus.
// Named buses key by name; anonymous buses key by generated id.
static BROADCAST_REGISTRY: LazyLock<Mutex<HashMap<BroadcastKey, BroadcastChannel>>> = ...;

enum BroadcastKey {
    Named(String),  // broadcast("metrics") — shared across workers/reloads
    Anon(u64),      // broadcast() — per-evaluation, single-worker semantics
}

struct BroadcastChannel {
    id: u64,
    senders: Vec<SubscriberEntry>,
    replay_buffer: Option<VecDeque<BroadcastMessage>>,
    replay_buffer_size: usize,
}

struct SubscriberEntry {
    sender_id: u64,
    // Per-subscriber bounded ring buffer. Drop-oldest is a SENDER-side
    // eviction (pop_front on overflow), which a flume/crossbeam channel
    // cannot do from the sending end — so the queue is an explicit
    // VecDeque under a Mutex, and `wake` is a coalescing signal that tells
    // the write task "drain me" without carrying the payload.
    queue: Arc<Mutex<VecDeque<BroadcastMessage>>>, // cap = queue_size (1024)
    wake: flume::Sender<()>,                        // bounded(1), coalescing
    queue_size: usize,
    // Phase 2: set by filter(sub, pred). The predicate is an ntnt closure;
    // it is evaluated at send() fan-out time on the publishing interpreter
    // thread (the write task never runs interpreter code).
    filter: Option<FilterPredicate>,
}

// Phase 2. Wraps the ntnt closure Value (and its captured environment)
// so send() fan-out can evaluate it on the publishing interpreter thread.
struct FilterPredicate {
    closure: Value, // Value::Function — predicate fn(Map) -> Bool
}

// Holds a subscription between the interpreter returning
// Value::SSESubscription(id) from the handler and the Axum layer taking
// ownership to drive the write task. take() consumes the entry, so a
// subscription streams to exactly one response.
static SUBSCRIPTION_REGISTRY: LazyLock<Mutex<HashMap<u64, SseSubscription>>> = ...;

struct SseSubscription {
    queue: Arc<Mutex<VecDeque<BroadcastMessage>>>, // SAME Arc as the entry
    wake_rx: flume::Receiver<()>,                  // write task awaits this
    channel_key: BroadcastKey, // for sender cleanup on disconnect
    sender_id: u64,
}

struct BroadcastMessage {
    id: Option<u64>,        // event ID for reconnect
    event: Option<String>,  // named event type
    value: SerializedValue, // serialized for thread safety
}
```

`send(bc, value)`:
1. Serialize `Value` → `SerializedValue`
2. Lock registry, look up the `BroadcastChannel` by the handle's
   `BroadcastKey`; append to replay buffer if configured; **snapshot the
   subscriber list** (clone each entry's `queue` Arc and `wake` Sender —
   both cheap, they're Arc-backed — plus `sender_id` and filter handle);
   drop the lock
3. Outside the lock, for each snapshot entry: if `filter` is set,
   evaluate the predicate against the value (we are on the publishing
   interpreter thread, so ntnt closures are evaluable here) and skip
   non-matching subscribers
4. For each remaining entry, enqueue with drop-oldest: lock the entry's
   `queue`; if `queue.len() == queue_size`, `pop_front()` (drop the oldest
   unsent event, warn deduped per subscriber) — unless the bus was created
   with `"drop_slow": false`, which instead waits briefly for the write
   task to drain (Phase 2); `push_back(message)`; unlock; then
   `wake.try_send(())` and ignore a `Full` result (a pending wake already
   means "drain"). A `Disconnected` wake receiver means the write task is
   gone → collect this `sender_id` for cleanup.
5. If any subscribers were dead, re-lock the registry briefly and remove
   those entries

Drop-oldest is a sender-side eviction, so the per-subscriber queue must be
an explicit `Mutex<VecDeque>`, not a channel: neither flume nor crossbeam
lets the sending end pop the oldest queued item (only the receiver drains).
The `wake` channel carries no payload — it is a coalescing "data ready"
signal (`bounded(1)`), so the write task can `recv_async()` on it and then
drain the deque.

Predicates and enqueues run OUTSIDE the registry lock deliberately:
`std::sync::Mutex` is non-reentrant, so a filter predicate that itself
calls `send()` (or `subscribe()`/`broadcast()`) would deadlock if the
registry lock were held across evaluation. With the snapshot approach,
re-entrant `send()` from a predicate simply takes the registry lock afresh
and works; recursion depth is bounded by user code. The cost is benign
staleness: a subscriber added or removed mid-send catches the next event.

`subscribe(bc)`:
1. Create the per-subscriber queue — `Arc<Mutex<VecDeque>>` capped at
   `queue_size` (default 1024) — and a `flume::bounded::<()>(1)` wake pair.
   Bounded per the Design Review: a stuck TCP connection with an unbounded
   queue is slow-motion OOM. The wake channel is flume so the Axum write
   task can `recv_async()` on it (crossbeam has no async recv). On a full
   queue the policy is drop-oldest with a deduped warn (unless the bus was
   created with `"drop_slow": false`, which opts into briefly-blocking
   sends — Phase 2).
2. Allocate a fresh `sender_id`; lock the broadcast registry, push
   `SubscriberEntry { sender_id, queue: queue.clone(), wake: wake_tx,
   queue_size, filter: None }` into `BroadcastChannel.senders`, drop the
   lock. (Phase 2's `filter(sub, pred)` later sets `filter` on this entry,
   located by `channel_key` + `sender_id`.)
3. Insert `SseSubscription { queue, wake_rx, channel_key, sender_id }` into
   `SUBSCRIPTION_REGISTRY` under a fresh `subscription_id` — this is the
   bridge handoff: the Axum handler later `take()`s this entry
4. Return `Value::SSESubscription(subscription_id)` to ntnt; the handler
   returns it through `sse()`, which carries the id to the HTTP layer as
   `BridgeBody::Sse { subscription_id }`

### SSE Response Handler (Rust HTTP Layer)

When the HTTP layer sees `BridgeBody::Sse { subscription_id }` on the bridge response (see Design Review):

```rust
async fn sse_handler(subscription_id: u64, req: Request) -> Response {
    let subscription = SUBSCRIPTION_REGISTRY.take(subscription_id); // consume
    let body = Body::new(async_stream::stream! {
        // Keep-alive ping
        let mut keepalive = tokio::time::interval(Duration::from_secs(15));

        loop {
            tokio::select! {
                _ = keepalive.tick() => {
                    yield Ok(Bytes::from(":\n\n"));  // SSE comment = keep-alive
                }
                woken = subscription.wake_rx.recv_async() => {
                    if woken.is_err() { break; } // all senders dropped
                    // Drain everything currently queued (drop-oldest means
                    // the deque already holds only the freshest queue_size
                    // events). Hold the lock only to move messages out.
                    let batch: Vec<BroadcastMessage> = {
                        let mut q = subscription.queue.lock();
                        q.drain(..).collect()
                    };
                    for m in batch {
                        yield Ok(format_sse_event(&m));
                    }
                }
                _ = req.closed() => break,  // client disconnected
            }
        }

        // Cleanup: remove this subscriber from the broadcast registry
        BROADCAST_REGISTRY.lock().remove_sender(&subscription.channel_key, subscription.sender_id);
    });

    Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("X-Accel-Buffering", "no")  // disable nginx buffering
        .body(body)
}
```

### SSE Event Format

```rust
fn format_sse_event(msg: &BroadcastMessage) -> Bytes {
    let mut out = String::new();
    if let Some(id) = msg.id {
        out.push_str(&format!("id: {}\n", id));
    }
    if let Some(event) = &msg.event {
        out.push_str(&format!("event: {}\n", event));
    }
    let json = serde_json::to_string(&msg.value).unwrap_or_default();
    out.push_str(&format!("data: {}\n\n", json));
    Bytes::from(out)
}
```

### New Value Variants

```rust
enum Value {
    // ... existing variants ...
    BroadcastHandle(BroadcastKey), // carries the registry key, so send()
                                   // can address named and anonymous
                                   // buses uniformly
    SSESubscription(u64),          // subscription_id into SUBSCRIPTION_REGISTRY
}
```

### Typechecker Signatures

```rust
// std/sse module
// name and opts are both optional: broadcast(), broadcast("metrics"),
// broadcast("metrics", map { "replay_buffer": 100 })
sig!("broadcast", ["name" => Type::String, "opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Named("BroadcastHandle".to_string()), required(0));
sig!("subscribe", ["handle" => Type::Named("BroadcastHandle".to_string())], Type::Named("SSESubscription".to_string()));
sig!("filter", ["sub" => Type::Named("SSESubscription".to_string()), "pred" => Type::Any], Type::Named("SSESubscription".to_string()));
sig!("connection_count", ["handle" => Type::Named("BroadcastHandle".to_string())], Type::Int);
// Both builders return Response, same as json()/html(), so middleware
// and enable_cors() compose without special cases
sig!("sse", ["sub" => Type::Named("SSESubscription".to_string())], Type::Named("Response".to_string()));
sig!("sse_stream", ["handler" => Type::Any], Type::Named("Response".to_string()));
```

### No Parser Integration Needed

The original draft planned a `respond sse(...)` keyword form, but there is
no `respond` keyword in the language (see Design Review). `sse()` and
`sse_stream()` are ordinary imported response builders, exactly like
`json()`/`html()`: the handler returns their value, the bridge recognizes
the SSE response (via the `BridgeBody::Sse` variant), and the parser is
untouched.

---

## Phase Details

### Phase 1: Core SSE 📋

**Estimated effort:** 3-4 days

- [ ] `Value::BroadcastHandle(BroadcastKey)` and `Value::SSESubscription(u64)` variants
- [ ] `BroadcastRegistry` global, keyed by name for named buses (Design Review: shared across workers and hot reloads) — LazyLock, same pattern as the pool registry
- [ ] `broadcast(name?) -> BroadcastHandle` — named form is the documented default; anonymous form is per-worker
- [ ] `subscribe(BroadcastHandle) -> SSESubscription` — per-subscriber bounded ring buffer (`Arc<Mutex<VecDeque>>`, cap 1024) + coalescing `flume::bounded(1)` wake; drop-oldest on overflow with deduped warn (the `"drop_slow": false` blocking opt-in arrives with `opts?` in Phase 2). Ring, not a channel: drop-oldest is a sender-side eviction channels can't do
- [ ] `send(BroadcastHandle, value) -> Bool` — dispatches on handle type (existing channel `send` is arity-2 and returns Bool; the broadcast arm keeps that contract. Phase 2's 3-arg named-event form widens `max_arity`)
- [ ] `sse(SSESubscription) -> Response` — response BUILDER returned from a normal handler (there is no `respond` keyword; zero parser work)
- [ ] `BridgeBody::Sse { subscription_id }` variant on the bridge response; Axum layer builds the streaming body when it sees it (Design Review item)
- [ ] Multi-worker integration test: sampler registers once on the primary (the existing `RuntimeCapability::Scheduling` gate — no new code), subscribers connect via workers, each event arrives exactly once per subscriber
- [ ] SSE write loop in Rust: format events, keep-alive pings, disconnect cleanup
- [ ] Sender cleanup on subscriber disconnect
- [ ] `sse_stream(fn(push, on_close))` — per-connection callback form; `push()` returns Bool (false after disconnect)
- [ ] `sse_stream` generator runs in a scheduling-capable per-connection scope (handed off from the worker), so `schedule()`/`cancel_schedule()` inside it register per connection instead of being skipped by the Worker capability gate — with a multi-worker test that a per-connection timer actually ticks when the request was served by a worker
- [ ] `connection_count(BroadcastHandle) -> Int`
- [ ] `enable_cors()` headers apply to SSE responses — headers are set before the stream body begins (see Security)
- [ ] Typechecker signatures
- [ ] `// @ntnt` doc blocks on all functions
- [ ] `ntnt docs --generate`
- [ ] Tests: subscribe + send + receive, multiple subscribers, disconnect cleanup, zero subscribers
- [ ] Example: `examples/sse_dashboard.tnt`
- [ ] `docs/AI_AGENT_GUIDE.md` — SSE section

### Phase 2: Named Events, Filtering, Replay 📋

**Estimated effort:** 2-3 days

- [ ] `send(BroadcastHandle, event_name, value)` — named SSE events
- [ ] `filter(SSESubscription, fn) -> SSESubscription` — server-side predicate filter
- [ ] `broadcast(name?, opts?)` — `replay_buffer: N`, `drop_slow: bool` (default true = drop-oldest; false opts into blocking)
- [ ] Replay buffer: ring buffer of last N events, sent on connect with `Last-Event-ID` header
- [ ] Event ID auto-increment when replay is enabled
- [ ] Tests: named events, filter, replay on reconnect, drop_slow behavior

### Phase 3: Auth, Limits, Observability 📋

**Estimated effort:** 1-2 days

- [ ] SSE-aware middleware — auth runs before SSE connection is established
- [ ] `max_connections: N` option on `broadcast()` — reject new subscribers over limit (429)
- [ ] Slow-consumer observability: expose per-subscriber drop counts (the drop policy itself ships in Phase 1: bounded queue, drop-oldest with deduped warn)
- [ ] `ntnt jobs`-style: `ntnt sse status server.tnt` — list active broadcast channels + connection counts

---

## Relationship to std/events and std/jobs

These three modules are complementary, not competing (`std/events` is
planned, not shipped — DD-037 Phase 7). Each operates at a different
layer:

| Module | Layer | Transport | Latency | Use for |
|--------|-------|-----------|---------|---------|
| `std/jobs` | Background work | KV (SQLite/Redis) | Seconds | Email, cleanup, processing |
| `std/events` (planned — DD-037 Phase 7) | Application events → jobs | In-process (or Redis pub/sub) | ~ms | "When X happens, run these jobs" |
| `std/sse` | Browser push | HTTP (SSE) | ~ms | Live metrics, dashboards, progress |

They compose naturally:

```ntnt
import { broadcast, sse } from "std/sse"
import { schedule, send } from "std/concurrent"
import { subscribe as event_subscribe, publish } from "std/events"  // planned (DD-037 Phase 7)

let metrics_bus = broadcast("metrics")

// Sampler fires jobs + pushes to SSE simultaneously
schedule(500, fn() {
    let m = sample_metrics()
    send(metrics_bus, m)                       // → SSE → browser (instant)

    if m["cpu"] > 95.0 {
        publish("cpu_critical", m)              // → std/events → jobs (durable)
    }
})

// Job handles the durable response (std/events subscribe imported as
// event_subscribe to avoid colliding with std/sse subscribe)
event_subscribe("cpu_critical", "SendPagerAlert")
event_subscribe("cpu_critical", "LogIncident")
```

Real-time display: `std/sse`. Durable alerting: `std/events` + `std/jobs`.

---

## Security

**SSE endpoints go through normal route middleware** — no special cases. Auth middleware added to a route group applies to SSE endpoints in that group:

```ntnt
import { broadcast, subscribe, sse } from "std/sse"
import { html, status } from "std/http/server"
import { starts_with } from "std/string"
import { read_file } from "std/fs"

let metrics_bus = broadcast("metrics")

// Middleware short-circuits before the SSE connection is established
// (is_admin is a hypothetical app-level helper)
use_middleware(fn(req) {
    if starts_with(req.path, "/dashboard") && !is_admin(req) {
        return status(403, "Forbidden")
    }
})

get("/dashboard", fn(req) { return html(read_file("dashboard.html")?) })
get("/dashboard/stream", fn(req) { return sse(subscribe(metrics_bus)) })
```

The middleware runs before the SSE connection is established. If the middleware rejects the request (401, 403), the SSE connection is never opened. This is correct — you don't want to establish the connection and then reject it.

**CORS:** If the SSE endpoint is consumed from a different origin (e.g. a static frontend), enable CORS the same way as for any other route — the existing `enable_cors()` server action (a global builtin, like the route functions) applies to SSE responses too:

```ntnt
import { broadcast, subscribe, sse } from "std/sse"

let metrics_bus = broadcast("metrics")

enable_cors(map { "origins": ["https://app.example.com"] })

get("/stream/metrics", fn(req) {
    return sse(subscribe(metrics_bus))
})
```

(Phase 1 must ensure the CORS layer applies its headers to `BridgeBody::Sse` responses — headers are set before the stream body begins.)

**Rate limiting:** Handled at the route level, same as any other route. `max_connections` on `broadcast()` caps total subscribers regardless of auth status.

---

## Competitive Analysis

| Feature | ntnt `std/sse` | Phoenix LiveView | Rails ActionCable | FastAPI SSE | Go net/http SSE |
|---------|---------------|-----------------|-------------------|-------------|-----------------|
| Setup | 3 lines | Full LiveView framework | ActionCable + Redis | Manual generator | Manual response writer |
| Broadcast to N clients | `broadcast()` + `subscribe()` | PubSub module | ActionCable channels | Manual (asyncio.Queue per client) | Manual (slice of channels) |
| Auth integration | Normal middleware | Normal plug | Normal middleware | Depends | Manual |
| Per-connection cleanup | Automatic (disconnect → drop) | Automatic | Automatic | Manual | Manual |
| Filter/transform | `filter(sub, fn)` | Custom handler | Custom handler | Manual | Manual |
| Keep-alive | Built-in (15s) | Built-in | Built-in | Manual | Manual |
| Named events | `send(bc, "name", val)` | Topic-based | Channel-based | Manual | Manual |
| Replay on reconnect | `broadcast("bus", map { "replay_buffer": N })` | ❌ | ❌ | Manual | Manual |
| Works with existing scheduler | `schedule()` → `send()` | Separate GenServer | Separate worker | Separate asyncio task | Separate goroutine |
| Lines for a metrics dashboard | ~15 | ~80 | ~100 | ~60 | ~70 |

ntnt's advantage: the sampler (`schedule`), the bus (`broadcast`), and the endpoint (`sse()` responses) are first-class primitives that compose with each other and with `std/jobs`/`std/events`. No framework, no adapter, no external process.

---

## Open Questions

All resolved as of the 2026-07-08 Design Review (resolutions below,
reconciled into the body):

| Question | Resolution |
|----------|------------|
| Broadcast backing implementation | Custom `Vec<SubscriberEntry>` per bus; each subscriber has a BOUNDED ring buffer (1024, drop-oldest) + a coalescing flume wake signal |
| `send()` overload on BroadcastHandle | Reuse existing `send()` dispatch on handle type |
| `drop_slow` default | Drop-oldest with deduped warn (default); `"drop_slow": false` opts into briefly-blocking sends |
| `sse_stream` cleanup model | `on_close(fn)` callback |
| Backpressure signal | `push()` returns Bool (false after disconnect) |
| WebSockets | Separate DD — SSE covers the dashboard cases and is simpler |
| `ntnt sse status` CLI | Phase 3 |
| CORS middleware built-in | Stdlib utility, not SSE-specific |

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-17 | Initial draft — vision, architecture, full API, three phases, implementation notes |
| 2026-07-08 | Design review against current main: response-builder API (no `respond` keyword), `BridgeBody::Sse` bridge variant, named process-global broadcasts; per-subscriber bounded ring buffer + wake signal (drop-oldest needs sender-side eviction, which a channel can't do); `sse_stream` generators run in a scheduling-capable per-connection scope so per-connection timers work under multi-worker; verified the module-level Worker `schedule()` gate already exists; body reconciled end to end |
