# DD-041: Real-Time Streaming — SSE & Broadcast Channels

**Status:** Draft — design review 2026-07-08 (see § Design Review below); awaiting maintainer go/no-go for Phase 1
**Author:** Larri
**Created:** 2026-03-17
**Branch:** TBD
**Depends on:** DD-037 (concurrency primitives — `schedule`, `channel`)

---

## Table of Contents

1. [Vision](#vision)
2. [Architecture](#architecture)
3. [API Design](#api-design)
4. [Full Stack Examples](#full-stack-examples)
5. [Implementation Notes](#implementation-notes)
6. [Phase Details](#phase-details)
7. [Relationship to std/events and std/jobs](#relationship-to-stdevents-and-stdjobs)
8. [Security](#security)
9. [Competitive Analysis](#competitive-analysis)
10. [Open Questions](#open-questions)

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
  per worker). A subscriber lands on whichever worker served its request;
  a sampler sends on whichever worker runs it. Subscribers on other
  workers receive nothing.
- `schedule()` spawns its loop unconditionally, so a module-level sampler
  runs N times — a pre-existing duplicate-background-work bug that SSE
  would surface as duplicate events.

Resolution, now part of Phase 1:

- **Broadcasts are named and process-global**: `broadcast("metrics")`
  returns the same bus for the same name from every worker (registry
  keyed by name, same `LazyLock<Mutex<HashMap>>` pattern as the pool
  registry). Named buses also survive hot reload — a re-eval re-attaches
  to the existing bus instead of orphaning open connections.
  Anonymous `broadcast()` remains for single-connection/`sse_stream`
  patterns and is documented as per-worker.
- **`schedule()` registers only outside Worker mode** (workers 1..N skip
  it, exactly as they skip `listen()`/`on_shutdown()`). This fixes the
  duplicate-sampler bug for ALL apps, not just SSE — flagged as its own
  release-note item since multi-worker apps today silently run N copies
  of every scheduled loop.

**Open questions resolved (recommendations):**

- Backing: custom `Vec<Sender>` per bus (crossbeam-consistent) — but with
  **bounded per-subscriber queues** (e.g. 1024) rather than unbounded: a
  stuck TCP connection with an unbounded queue is slow-motion OOM.
  Default policy drop-oldest with a deduped warn; `"drop_slow": false`
  opts into blocking sends for correctness-critical streams.
- `send()` reuses the existing dispatch on handle type — no new verb.
- `sse_stream` cleanup stays `on_close(fn)`; `push()` returns Bool so
  generators can stop on disconnect.
- WebSockets remain a separate DD; `ntnt sse status` stays Phase 3.

Phase 1 effort holds at 3-4 days with the two additions (BridgeBody
variant, worker-mode schedule gate). Awaiting go/no-go.

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

`channel()` creates a `ChannelHandle` backed by a crossbeam unbounded channel — one producer, one consumer (or multiple producers, one consumer via clone). Every `recv()` call removes the message from the queue. This is wrong for SSE: you want every connected browser to receive every metric sample, not one browser to receive each sample in round-robin.

### The New Primitive: Broadcast Channel

`broadcast()` creates a broadcast channel backed by a multi-sender, multi-receiver bus. Each subscriber gets their own crossbeam receiver that receives a copy of every message sent to the bus. Messages are not consumed — they're fanned out.

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
- Custom: Vec of crossbeam senders, new sender added per subscriber, clean up on disconnect

The custom approach (Vec of senders) is most consistent with ntnt's existing crossbeam usage.

### How `return sse(...)` Works

When a route handler returns `return sse(subscription)`:

1. The Rust HTTP layer recognizes the SSE response type.
2. It sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`, connection stays open.
3. A Rust task (not an interpreter thread) polls the subscription's crossbeam receiver in a loop, formatting each `Value` as an SSE event and writing to the HTTP response body.
4. When the client disconnects, the write fails, the loop exits, and the subscription is dropped — which removes this subscriber from the broadcast channel's sender list.
5. The interpreter thread is free after returning the SSE response token. It does not block.

This means SSE adds **zero interpreter-thread overhead** after the route handler completes.

### Keep-Alive

SSE connections die silently when passing through proxies and load balancers that close idle connections. The SSE writer task sends a keep-alive comment (`:\n\n`) every 15 seconds (configurable). Browsers ignore comments; proxies see activity and keep the connection open.

### Connection Registration Flow

```
Browser → GET /metrics/stream
  → Route handler runs (interpreter thread)
  → subscribe(metrics_bus) → creates crossbeam receiver, registers in BroadcastRegistry
  → return sse(subscription) → returns SSEResponse token to HTTP layer
  → HTTP layer spawns write task: poll receiver → write to socket
  → Route handler exits, interpreter thread free

schedule(500ms) loop (separate thread):
  → send(metrics_bus, value) → locks BroadcastRegistry, sends to all registered receivers
  → Each write task wakes, reads value, writes SSE event to socket
```

---

## API Design

### Phase 1: Core

```ntnt
import { broadcast, subscribe, sse } from "std/sse"

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
- `send(BroadcastHandle, Map) -> Unit` — reuses existing `send()` dispatch on handle type
- `sse(SSESubscription) -> Response` — ordinary response builder (like `json()`/`html()`)

### Per-Connection Streams (No Shared Bus)

For streams that are unique per connection — a clock, a personal notification feed, a progress bar — use the callback form:

```ntnt
import { sse_stream } from "std/sse"

get("/clock", fn(req) {
    return sse_stream(fn(push, on_close) {
        let sched = schedule(1000, fn() {
            push(map { "time": format_time(now()) })
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
            if status["status"] in ["completed", "failed", "dead"] {
                push(map { "done": true })
                // on_close fires automatically when the response ends
            }
        })
        on_close(fn() { cancel_schedule(poll) })
    })
})
```

`sse_stream(fn(push, on_close))`:
- `push(value)` — send an event to this connection
- `on_close(fn)` — register a cleanup callback for when the client disconnects
- Returns when the connection closes or the handler returns

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
import { filter } from "std/sse"

get("/stream/network/{iface}", fn(req) {
    let iface = req.params.iface
    return sse(filter(subscribe(metrics_bus), fn(m) { m["interface"] == iface }))
})
```

`filter(SSESubscription, fn(Map) -> Bool) -> SSESubscription`

Filtering happens server-side before writing to the socket — non-matching events are dropped, never sent over the wire.

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
import { broadcast, subscribe } from "std/sse"

let metrics = broadcast("metrics")

// One sampler loop, feeds all connected browser tabs
schedule(500, fn() {
    send(metrics, map {
        "cpu":    cpu_percent(),
        "mem_mb": mem_used_mb(),
        "load":   load_avg(),
        "ts":     unix_ms(),
    })
})

get("/dashboard", fn(req) {
    return file("dashboard.html")
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
import { broadcast, subscribe, filter } from "std/sse"
import { subscribe as event_subscribe, publish } from "std/events"

let net_metrics = broadcast()
let net_alerts  = broadcast()

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
    return sse(filter(subscribe(net_metrics), fn(m) { m["name"] == iface }))
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

get("/jobs/{id}/progress", fn(req) {
    let job_id = req.params.id
    return sse_stream(fn(push, on_close) {
        let poll = schedule(500, fn() {
            let status = unwrap(job_status(job_id))
            push(status)
        })
        on_close(fn() { cancel_schedule(poll) })
    })
})
```

---

### Log Tail

```ntnt
import { broadcast, subscribe, filter } from "std/sse"

let log_bus = broadcast()

// Hook into ntnt's log system (or tail a file)
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
    senders: Vec<crossbeam::channel::Sender<BroadcastMessage>>,
    replay_buffer: Option<VecDeque<BroadcastMessage>>,
    replay_buffer_size: usize,
}

struct BroadcastMessage {
    id: Option<u64>,        // event ID for reconnect
    event: Option<String>,  // named event type
    value: SerializedValue, // serialized for thread safety
}
```

`send(bc, value)`:
1. Serialize `Value` → `SerializedValue`
2. Lock registry, get `BroadcastChannel`
3. Append to replay buffer if configured
4. Send to each sender; on `SendError` (receiver dropped) → remove that sender from the list
5. Drop lock

`subscribe(bc)`:
1. Create a new `crossbeam::channel::bounded(1024)` pair — bounded per the
   Design Review: a stuck TCP connection with an unbounded queue is
   slow-motion OOM. On a full queue the send policy is drop-oldest with a
   deduped warn (unless the bus was created with `"drop_slow": false`,
   which opts into briefly-blocking sends)
2. Lock registry, push `sender` into `BroadcastChannel.senders`
3. Wrap `receiver` in `SSESubscription { receiver, channel_id, last_event_id: None }`
4. Drop lock
5. Return `Value::SSESubscription(id)` to ntnt

### SSE Response Handler (Rust HTTP Layer)

When the HTTP layer sees `Response::SSE(subscription_id)`:

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
                msg = subscription.recv_async() => {
                    match msg {
                        Ok(m) => yield Ok(format_sse_event(&m)),
                        Err(_) => break,  // broadcast channel dropped
                    }
                }
                _ = req.closed() => break,  // client disconnected
            }
        }

        // Cleanup: remove sender from broadcast registry
        BROADCAST_REGISTRY.lock().remove_sender(subscription.channel_id, &subscription.sender_id);
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
    BroadcastHandle(u64),
    SSESubscription(u64),
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
sig!("sse_stream", ["handler" => Type::Any], Type::Named("SSEResponse".to_string()));
```

### `return sse(...)` Parser Integration

`respond` already handles `return file(...)`, `return json(...)`, `return html(...)`. Add:
- `return sse(expr)` — where `expr` evaluates to `SSESubscription`
- `return sse_stream(expr)` — where `expr` evaluates to a closure

---

## Phase Details

### Phase 1: Core SSE 📋

**Estimated effort:** 3-4 days

- [ ] `Value::BroadcastHandle(u64)` and `Value::SSESubscription(u64)` variants
- [ ] `BroadcastRegistry` global, keyed by name for named buses (Design Review: shared across workers and hot reloads) — LazyLock, same pattern as the pool registry
- [ ] `broadcast(name?) -> BroadcastHandle` — named form is the documented default; anonymous form is per-worker
- [ ] `subscribe(BroadcastHandle) -> SSESubscription` — bounded per-subscriber queue (1024), drop-oldest with deduped warn; `"drop_slow": false` opts into blocking
- [ ] `send(BroadcastHandle, value)` — dispatches on handle type (reuse existing `send`)
- [ ] `sse(SSESubscription) -> Response` — response BUILDER returned from a normal handler (there is no `respond` keyword; zero parser work)
- [ ] `BridgeBody::Sse { subscription_id }` variant on the bridge response; Axum layer builds the streaming body when it sees it (Design Review item)
- [ ] `schedule()` registers only outside Worker mode, so module-level samplers run once, not once per worker (Design Review item; own release note)
- [ ] SSE write loop in Rust: format events, keep-alive pings, disconnect cleanup
- [ ] Sender cleanup on subscriber disconnect
- [ ] `sse_stream(fn(push, on_close))` — per-connection callback form; `push()` returns Bool (false after disconnect)
- [ ] `connection_count(BroadcastHandle) -> Int`
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
- [ ] Slow consumer detection: if a subscriber's buffer backs up > threshold, emit warning or drop
- [ ] `X-Accel-Buffering: no` header set automatically (nginx/Cloudflare compat)
- [ ] `ntnt jobs`-style: `ntnt sse status server.tnt` — list active broadcast channels + connection counts
- [ ] CORS headers for cross-origin SSE (configurable)

---

## Relationship to std/events and std/jobs

These three modules are complementary, not competing. Each operates at a different layer:

| Module | Layer | Transport | Latency | Use for |
|--------|-------|-----------|---------|---------|
| `std/jobs` | Background work | KV (SQLite/Redis) | Seconds | Email, cleanup, processing |
| `std/events` | Application events → jobs | In-process (or Redis pub/sub) | ~ms | "When X happens, run these jobs" |
| `std/sse` | Browser push | HTTP (SSE) | ~ms | Live metrics, dashboards, progress |

They compose naturally:

```ntnt
// Sampler fires jobs + pushes to SSE simultaneously
schedule(500, fn() {
    let m = sample_metrics()
    send(metrics_bus, m)                       // → SSE → browser (instant)

    if m["cpu"] > 95.0 {
        publish("cpu_critical", m)              // → std/events → jobs (durable)
    }
})

// Job handles the durable response
subscribe("cpu_critical", "SendPagerAlert")     // std/events wiring
subscribe("cpu_critical", "LogIncident")
```

Real-time display: `std/sse`. Durable alerting: `std/events` + `std/jobs`.

---

## Security

**SSE endpoints go through normal route middleware** — no special cases. Auth middleware added to a route group applies to SSE endpoints in that group:

```ntnt
// Middleware short-circuits before the SSE connection is established
use_middleware(fn(req) {
    if starts_with(req.path, "/dashboard") && !is_admin(req) {
        return status(403, "Forbidden")
    }
})

get("/dashboard", fn(req) { return file("dashboard.html") })
get("/dashboard/stream", fn(req) { return sse(subscribe(metrics_bus)) })
```

The middleware runs before the SSE connection is established. If the middleware rejects the request (401, 403), the SSE connection is never opened. This is correct — you don't want to establish the connection and then reject it.

**CORS:** If the SSE endpoint is consumed from a different origin (e.g. a static frontend), add CORS headers:

```ntnt
get("/stream/metrics", fn(req) {
    return sse(subscribe(metrics_bus))
})
```

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

All resolved by the 2026-07-08 Design Review (decisions recorded there and
reconciled into the body):

| Question | Resolution |
|----------|------------|
| Broadcast backing implementation | Custom `Vec<Sender>` per bus, crossbeam-consistent — with BOUNDED per-subscriber queues (1024) |
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
