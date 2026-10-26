# DD-041: Native SSE and WebSockets

**Status:** Accepted design direction — implementation not started; feasibility gates pending.
**Author:** Larri
**Created:** 2026-03-17
**Revised:** 2026-10-25
**Baseline:** NTNT v0.5.6, `e9dadc715fc84dbe4a7b80289e5d21f9e3dca734`.
**Scope:** Replaces the previous SSE-only DD-041 and supersedes the WebSocket sketches in DD-058, `http_updates.md`, and ROADMAP Phase 10.5. The filename is retained for stable links; prior drafts remain in Git history.

Approval accepts the two-slice design direction, not a claim of shipped APIs or authorization to implement/deploy. API details and numerical defaults remain subject to the feasibility and measurement gates below.

The [proposed implementation plan](dd-041-implementation-plan.md) details ownership contracts, dependency-ordered slices and the runtime experiments required before implementation. It remains subject to review; the feasibility gates have not been executed.

## 1. Decision

Support **both protocols**, delivered in two independently useful slices: SSE first, WebSockets second. Share connection ownership, resource accounting, native fan-out, response handoff, and shutdown—not protocol semantics or a new application framework.

Use SSE for dashboards, notifications, progress, live logs, and server-to-browser updates. Existing authenticated HTTP requests remain the simplest client-to-server command path. WebSockets earn their place for frequent two-way messages, interactive sessions, collaborative applications, and binary messages. A full chat product does not inherently require WebSockets; many work well with SSE plus POST. SSE is nevertheless not a substitute for a general bidirectional socket protocol.

This is a **medium runtime/server feature, not a server rewrite**. Axum already supplies both transports. SSE is the smaller portion; safe WebSocket callbacks and lifecycle integration are the consequential work. Tentatively allow roughly one to two focused engineering weeks for both, including tests, subject to the integration spikes below. This is a planning estimate, not a delivery commitment or an estimate derived from the old DD's line count.

## 2. What already exists

- The [previous DD-041: Real-Time Streaming — SSE & Broadcast Channels](https://github.com/ntntlang/ntnt/blob/v0.5.6/design-docs/dd-041-sse-streaming.md) was an unimplemented SSE draft, including a later multi-worker design review. This revision replaces it.
- [DD-058](https://github.com/ntntlang/ntnt/blob/v0.5.6/design-docs/dd-058-stdlib-gaps.md), [http_updates.md](https://github.com/ntntlang/ntnt/blob/v0.5.6/design-docs/http_updates.md), and ROADMAP Phase 10.5 contain WebSocket sketches. They do not establish a coherent shipped API.
- The current Axum/Tokio server bridges to synchronous interpreter workers through a bounded flume queue and oneshot replies. `BridgeResponse.body` is a String; response conversion buffers normal application bodies. Static-file serving already uses a streaming body.
- Workers have independent, non-Send `Rc<RefCell<...>>` environments. Application functions can be used in tasks in 0.5.6, but an arbitrary request-local closure is not automatically portable to another existing HTTP worker.
- Current `max_connections` configuration is not enforced admission control. Do not count it as an existing safety mechanism.

Source starting points: `src/stdlib/http_bridge.rs:145–282`, `src/stdlib/http_server_async.rs:402–468,521–537,986–1098`, `src/interpreter.rs:10464–10520,10657–10769,10816–10874`, and `src/stdlib/concurrent.rs:505–600,2042–2084,2420–2488,2653–2663`. Lines refer to the baseline above.

### Retained from the previous DD-041

Ordinary response builders, normal middleware before streaming, named buses shared across workers, native transport delivery, bounded memory, automatic disconnect cleanup, and keep-alives.

### Replace or defer

- Replace hand-built per-subscriber ring/wake plumbing with Tokio's bounded broadcast channel unless measurements demonstrate a concrete missing invariant.
- Replace silent drop-oldest delivery with explicit lag handling: close the affected stream and require reconciliation. Do not silently corrupt a delta stream.
- Do not make publishers wait for slow sockets, even as an opt-in correctness mode. Blocking a publisher does not make delivery durable.
- Remove per-connection generators, scheduler ownership, arbitrary predicate callbacks, replay history, and a status CLI from the initial feature.
- Authentication, real admission limits, ownership cleanup, and shutdown belong in the first useful slice—not a later hardening phase.

## 3. Public surface: small and additive

All APIs below are **proposed**, not available in v0.5.6. No parser syntax, async language syntax, or new route DSL is required.

- `std/concurrent.broadcast(name, options?) -> Broadcast`: a named, runtime-scoped native fan-out source.
- `std/concurrent.publish(bus, value, event_options?) -> Result<Unit, String>`: validate and JSON-encode once, then publish without waiting for any client. Zero subscribers is a successful no-op, not replay storage. Optional `event` and `id` fields are SSE metadata; neither is an authorization grant.
- `std/http/server.sse(bus, options?) -> Response`: streaming response builder for an ordinary GET handler. It attaches its own subscription; a separate public `subscribe()` object is unnecessary initially.
- `std/http/server.ws_endpoint(name, on_message, options?) -> WebSocketEndpoint`: application-initialization registration, in each worker, of the callback definition. Optional `on_open` and `on_close` hooks use the same registry and worker mechanism.
- `std/http/server.websocket(endpoint, options?) -> Response`: upgrade response builder, returned by an ordinary authenticated GET handler. Options contain explicit serializable `state`, optionally a broadcast `source`, and endpoint-narrowing limits/origin policy.
- `std/http/server.ws_send(conn, data) -> Result<Unit, String>`: enqueue a text String or checked byte array as one WebSocket message. Success means admitted to the bounded queue, never confirmed delivery.
- `std/http/server.ws_close(conn, code?, reason?) -> Result<Unit, String>`: validated, idempotent close. No raw socket or numeric-ID API.

Keep existing `send()` behavior unchanged. The explicit `publish()` verb avoids changing channel return semantics or overloading event metadata onto every channel operation. Do not introduce separate `std/sse`, `std/ws`, and generic event-framework APIs for this slice.

### SSE example — proposed API

```ntnt
import { broadcast, publish } from "std/concurrent"
import { sse } from "std/http/server"

let metrics = broadcast("metrics")

get("/events/metrics", fn(req) {
    // Existing middleware/application authorization runs first.
    return sse(metrics)
})

// An existing sampler/request/task can call:
// publish(metrics, value, map { "event": "metrics", "id": sample_id })?
```

The browser uses `new EventSource('/events/metrics')`. Each SSE `data:` contains the JSON encoding of the published value. A String is therefore a JSON string, not raw SSE markup. Event names and IDs are validated against control-character injection and bounded independently.

### WebSocket example — proposed API

```ntnt
import { ws_endpoint, websocket, ws_send } from "std/http/server"

fn echo_message(conn, message, state) {
    ws_send(conn, message.data)?
    return state
}

let echo = ws_endpoint("echo", echo_message)

get("/socket/echo", fn(req) {
    // Authorize using the actual post-middleware request here.
    return websocket(echo, map { "state": map {} })
})
```

`message` is `{ kind: "text" | "binary", data: String | Array<Int> }`. UTF-8 text, fragmented-message assembly, masking validation, ping/pong, and close frames are handled by the maintained protocol library, not NTNT code. Byte arrays use the existing 0–255 representation; no new Bytes language type is required. JSON parsing for incoming application messages is explicit.

`on_message(conn, message, state)` returns the next state. Optional `on_open(conn, state)` runs first and returns state; `on_close(state, info)` is bounded best effort. A callback error closes the connection; there is no automatic retry. Effects already performed by a callback, including accepted sends and database writes, are not rolled back by a later failure.

An optional broadcast `source` forwards each published JSON value as a WebSocket text message. SSE-only `event`/`id` metadata is not implicitly wrapped into a WebSocket application protocol; applications needing message types/cursors on both transports include them in the payload. No implicit rooms, subscription messages, topic guessing, or client-controlled channel joins.

## 4. Architecture and interpreter boundary

```text
HTTP request / upgrade attempt
  -> existing route matching, middleware and application authorization
  -> ordinary response envelope with an opaque transport capability
  -> typed bridge reply; HTTP interpreter worker is released
  -> native async transport owner
       SSE: broadcast receiver -> body stream
       WS: read/write/control frames + bounded application-event admission
                -> short worker callback -> next serialized state
```

### Typed response handoff

Change the bridge body to a Rust enum with ordinary buffered, SSE, and WebSocket reply variants. Keep status/header metadata map-shaped so CORS, CSP, error handlers and cookie helpers continue to compose; the transport authority inside that envelope is a real opaque native value, not a magic String tag or integer ID.

Bind the response capability to the originating request/server generation; attachment is one-shot. Validate before committing headers. Reuse, stale aliases, wrong-server attachment and serialization into JSON/DB/KV/jobs must fail. Pending subscriptions/admission reservations need request-owned cleanup when middleware replaces a response, the request times out, a client disappears, reply delivery fails, or upgrade construction fails. Do not create an immortal global pending-token registry.

Retain Axum's WebSocket upgrade capability on the async side **before** current request conversion consumes `req.into_body()` and discards extensions. Only perform the upgrade after the normal interpreter pipeline succeeds. Reconstructed headers are not upgrade authority. HEAD must not attach an SSE body; non-GET or malformed WebSocket upgrades fail without opening a connection.

### Callback identity and explicit state

`ws_endpoint` is setup-only, like route registration: register definitions in every worker before serving. Resolve them through a stable endpoint identity and program generation, not a worker-local numeric slot. Conflicting definitions/configuration fail startup. Keep callback closures in their owning worker environments; do not send `Value::Function` or `Rc` across threads.

A callback work item contains endpoint/generation/sequence, an opaque connection send capability, explicit bounded state and message data. Any initialized worker may execute it using its local endpoint definition. Module globals retain existing worker-local semantics; state that must follow a connection is passed explicitly and returned explicitly. State is JSON-compatible, secret-free, size-limited data—not an implicit snapshot of cookies, headers, the entire request, handles, or lexical environments.

One callback may be in flight per connection. Apply its returned state only if the connection generation and sequence are still current. Closing/reloading invalidates late replies. Arbitrary closures created inside an opening request are deliberately unsupported; this restriction avoids portable closure snapshots, worker affinity, and a fresh interpreter per frame.

### Worker fairness and cancellation

Use a separately bounded callback admission queue integrated with the existing workers. Admission is nonblocking or has a short explicit deadline; do not create unbounded tasks waiting to enqueue into a bounded channel. Start with weighted service between HTTP and callback work, and a callback execution limit leaving one HTTP worker available when more than one worker exists. The one-worker case retains the existing limitation that any blocking application invocation delays all other invocations; do not claim otherwise.

Check connection cancellation and admission deadlines before starting queued callbacks. No callback worker may wait for socket capacity or the next client message. Native ping/pong, reading control frames, close handling and transport cancellation continue independently while a callback is in flight. A client flooding data while a callback is busy must hit a finite message/byte/rate limit.

Timeout is not CPU preemption. Existing NTNT loops and some native calls cannot be forcibly interrupted; this feature does not add interpreter suspension or promise rollback of an in-flight callback's side effects. Keep callback guidance explicit, reject stale results, and benchmark the reservation/fairness policy. CPU budgets would be a separate interpreter concern if real applications demonstrate that need.

## 5. Fan-out, ownership and resource bounds

Use `tokio::sync::broadcast<Arc<EncodedEvent>>`. Publishing is synchronous and nonblocking; receivers await asynchronously. A shared immutable encoded payload avoids cloning the NTNT value or re-serializing JSON for each subscriber. A transport frame may be encoded once per protocol, not once per recipient. Use Axum's SSE event/keep-alive implementation and WebSocket codec rather than handwritten wire parsers.

Bus names resolve inside the same application/server generation across all interpreter workers and in-process tasks. They are **not** shared between different applications or OS processes. Initialization must propagate the namespace to workers/task captures; do not use a global bare-name registry that merges unrelated servers. A live native handle grants publish/subscribe authority; a browser-supplied String does not. Bound topic creation, reject incompatible redefinitions, and use weak registry entries/reference ownership so transient names do not accumulate forever. Capturing a Broadcast into an in-process task is supported explicitly; peers/endpoints/response capabilities are not general task/channel/job payloads.

Suggested conservative defaults, to confirm through the spike/load measurements:

- 1,024 live realtime connections per server; 256 per endpoint; enforce actual permits before opening.
- 64 retained events per bus; 64 KiB maximum encoded event/message.
- Direct WS outgoing queue: at most 16 messages and 256 KiB; inbound pending data: at most 4 messages and 256 KiB; connection state: at most 16 KiB encoded.
- 64 admitted callback jobs globally, plus the bounded in-flight execution limit.
- A 32 MiB application-owned realtime payload budget covering retained/in-flight native payload allocations, with permits released on the last shared reference—not merely on ring eviction. Bound topic and pending-response counts too.
- SSE comments every 15 seconds; WS ping every 30 seconds with a 10-second pong deadline; WS stalled-write and close-drain deadlines of 10 and 5 seconds respectively.
- Maximum application connection lease of 5 minutes by default, reconnecting through normal authentication; trusted configuration may choose another finite limit. Expiry stops admitting callbacks/producing protected data. Native connections hold no persistent interpreter or OS thread.

A body-stream timer alone does **not** prove a hard SSE socket-write deadline: Hyper may be waiting on an unresponsive peer without polling the body again. The lifecycle spike must establish how the current server cancels an active body/connection, including HTTP/2 streams. Do not release a live-connection permit while the actual transport remains owned, claim instantaneous TCP teardown, or replace Axum's server driver speculatively to manufacture that guarantee. If hard write-stall/teardown requirements require a larger connection-driver change, surface that scope decision before implementing; meanwhile ring/payload/connection admission bounds must remain enforceable even for a stalled peer.

These are application-owned bounds, not an RSS formula: kernel sockets, codec buffers, TLS/proxy state, expanded NTNT values and worker memory must be measured separately. Configure codec message/write-buffer limits explicitly rather than inheriting large defaults. Validate encoded size during bounded serialization, not after first allocating an unlimited String. A single server-level `realtime` map under the existing `listen(port, options)` form supplies limits; endpoints can narrow them. Invalid/unknown values fail early; normal existing `listen` calls are unchanged.

On broadcast lag, close that consumer rather than silently continuing with a gap. SSE may send a bounded best-effort control event before close, but correctness must not rely on that event reaching an already slow socket. The client reconciles authoritative state on every initial connection and reconnect. WS uses an appropriate overload close (for example 1013); protocol/oversize errors use the corresponding protocol close code. No global backpressure from one consumer, unbounded queue, or lossless-delivery claim.

Native Drop/cancellation guards release subscriptions, byte permits, connection permits and callback ownership on every exit, including upgrade failure/body drop/panic. Saved weak connection-handle aliases cannot keep a closed socket or its queues alive. Cleanup must not depend on application `on_close` or another successfully admitted callback.

## 6. HTTP, auth and deployment contract

- Run existing middleware and application authorization before SSE 200 or WS 101. Preserve denial responses, redirects and repeated Set-Cookie headers. Cookies can be set only during admission, not in later frames.
- The auth stack can return a refresh 307 plus cookies. Specify and test this admission path for browsers; do not assume a WebSocket client follows it. Prefer refreshing the authenticated HTTP session before opening a socket when refresh is needed. Never turn a rejected/refresh response into a successful upgrade.
- SSE uses normal HTTP/CORS rules. Browser EventSource supports cookie sessions but not arbitrary Authorization headers; do not recommend bearer tokens in URLs. A fetch-based SSE client can be used when explicit headers are required.
- WS needs its own exact-Origin policy; CORS does not authorize a WebSocket handshake. Default to the canonical configured public origin, reject null/foreign origins, and require an explicit trusted setting for origin-less non-browser clients or cross-origin clients. Never derive trust from arbitrary Host/X-Forwarded-* headers; missing trusted public-origin configuration must fail closed in production rather than guess.
- Handshake authentication is not permanent authorization. Finite connection age bounds stale session access but is not immediate revocation. Sensitive WS commands must revalidate current application/session authorization per command; copied `state.user_id` is not proof of a still-valid session. Applications requiring immediate revocation need an explicit verified revocation/close integration before adopting protected streams. Do not market handshake-only auth as live revocation.
- No secrets in frames, state serialization, debug output or error rendering. Reject nested Secret values before publication/send. Ordinary sensitive application data still requires correct topic/recipient authorization.
- SSE emits UTF-8 `text/event-stream`, `Cache-Control: no-store, no-transform`, and `X-Accel-Buffering: no`, and omits Content-Length. Leave ordinary HTTP timeout protections in place: the pinned tower timeout wraps response creation, not streaming-body lifetime. Its compression predicate already excludes SSE; retain and test that behavior.
- Support HTTP/1.1 WebSocket upgrade initially. Do not claim HTTP/2 extended CONNECT or HTTP/3 WebSockets. SSE can use the HTTP versions supported by the current server/front proxy; do not add an HTTP-stack upgrade to this feature.
- On shutdown: stop admission, cancel native connections, close WS with bounded draining, release subscriptions and only then wind down bridge/workers. Native cleanup must precede the user shutdown hooks that currently run after joins. Bound native connection teardown; do not promise forced interruption of arbitrary user/native callbacks.
- Development reload closes affected connections with a generation change. Do not silently keep old callbacks alive against new code. Existing multi-worker development restart limitations remain explicit.

## 7. Delivery semantics and non-goals

A publish/send acknowledgment is queue admission, not browser receipt, durable acceptance, or exactly-once execution. Disconnects and callbacks can have partial/unknown external outcomes; the runtime never retries an application message automatically. Ordering is per connection and a bus's accepted publication order, not a total order across all buses/producers/processes.

SSE `id` and the request's `Last-Event-ID` header are supported wire fields, **not** built-in history. No automatic replay across reconnects/restarts. Reconciliation/snapshot/durable cursor history belongs to the application; subscribe/snapshot races must be addressed there if using deltas. For the simplest dashboard, treat events as invalidations and refetch authoritative state. Do not claim that GET-snapshot-then-subscribe is race-free.

No Redis/broker requirement, cross-process fan-out, durable pub/sub, `std/events` dependency, room DSL, wildcard channels, per-subscriber interpreter filters, per-connection scheduler, Socket.IO compatibility, transport negotiation/fallback package, client SDK, outbound WebSocket client, native outbound SSE reader, generic streaming HTTP client, or new CLI. A separate jobs process cannot publish to this in-memory bus; it must use an explicit application handoff. These can be justified separately by actual consumers.

## 8. Implementation slices and checks

### Slice A: shared ownership seam plus useful SSE

Add typed response handoff, native broadcast/publish handles and serialization rules, server-scoped limits, body Drop cleanup, heartbeat, auth/header behavior, shutdown and one small dashboard fixture. Ordinary HTTP remains unchanged. Do not include WS callback dispatch or generator APIs merely to anticipate later needs.

### Slice B: small bidirectional WebSockets

Enable Axum's `ws` feature, assess the resulting dependency/advisory delta, preserve upgrade authority, add endpoint registration, native actor/send/close, explicit connection state, fair bounded callback dispatch, Origin policy, text/binary support and lifecycle hooks. Use a small echo plus authorized messaging fixture; no chat framework.

### Focused feasibility gates before implementation commitments

1. On pinned Axum 0.7.9, prove streamed-body Drop and failed-upgrade cleanup, cookie-bearing/rejected handshake behavior, and live-stream shutdown. Confirm behavior through the actual NTNT bridge, not only a standalone Axum demo.
2. Prove the setup-registered callback identity/state handoff with multiple real interpreters, including imported helpers, cancellation, worker restart and generation mismatch. Do not substitute a new interpreter per message if this seam is awkward; resolve the seam first.
3. Measure idle connection memory/thread count and callback-vs-HTTP fairness. If the design requires a language-wide async/closure refactor, stop and rescope rather than hiding it inside this feature.

### Acceptance checks

- Real multi-worker SSE delivery: one in-process producer reaches clients admitted through different workers; no duplicated sampler registration or worker-local bus isolation.
- Real WS handshake, text/binary round trips, fragmented message limits, per-connection message ordering and state, errors and native ping/pong while callback work is pending.
- Unauthorized/foreign-origin/invalid-upgrade attempts open no transport and leak no permits; auth refresh/cookies and CORS work as specified. Secret/capability forgery, cross-server handles and stale-generation aliases are rejected.
- A slow or flooding client cannot grow memory beyond admitted bounds or block a publisher/healthy client. Assert serialization count and shared payload identity; exercise the actual parser/worker API, not only synthetic native data.
- Disconnect/body drop, middleware replacement, reply timeout, upgrade failure, callback failure/panic, shutdown and reload release native resources. Application cleanup hooks failing or being unable to enqueue cannot prevent this.
- Compare ordinary HTTP baseline before/after on the same build profile/machine, with and without many idle streams; record throughput, tail latency, CPU, OS threads and idle per-connection memory. Thousands of idle sockets must not create thousands of interpreter threads. Check broadcast fan-out scaling and actual codec buffers rather than asserting universal RPS/RSS numbers.
- Native Rust integration clients should drive the protocol tests. Existing synchronous `ntnt test` mode must fail clearly for unsupported streaming/upgrade execution instead of buffering forever or silently passing; ordinary existing tests remain intact. Do not count documentary `.intent` scenarios as executed streaming assertions.
- Relevant existing tests, platform CI, generated stdlib docs/type signatures, API examples, release notes and dependency security checks are updated for each shipped slice.

## 9. Evidence from this design pass

The repository/current dependencies were inspected; no NTNT server implementation was changed. A disposable offline Rust probe compiled against exact Tokio 1.53.3 and passed assertions for shared immutable fan-out payloads, bounded oldest eviction with an explicit lag error, synchronous worker publication to async receive, and the no-receiver condition. This removes the justification for a handwritten ring solely to obtain drop-oldest behavior. It does **not** prove the future NTNT bridge, callback dispatcher, browser/proxy integration, or memory targets.

Additional reference behavior was checked in the pinned tower-http 0.5.2 timeout/compression source and Axum 0.7.9 upgrade source. Protocol references: [WHATWG SSE](https://html.spec.whatwg.org/multipage/server-sent-events.html), [RFC 6455](https://www.rfc-editor.org/rfc/rfc6455), and [Tokio broadcast semantics](https://docs.rs/tokio/1.53.3/tokio/sync/broadcast/index.html).

## 10. Accepted direction and implementation gate

The maintainer has accepted the two-slice direction: process-local live delivery and explicit state, without replay/brokers/rooms/generators. Implementation has not started; proposed API details and suggested limits still require the feasibility and measurement checks above. The first implementation task is the narrow bridge/upgrade/callback feasibility probe, not coding the entire design at once. If first-release requirements include cross-process job push, durable replay, arbitrary request-local closures, or immediate session revocation, revise that scope explicitly before implementation; those materially change the work.
