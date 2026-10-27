# DD-041 implementation plan: native SSE, then WebSockets

**Status:** Independently reviewed plan. Gate 0 executed on 2026-10-26 (outcome recorded under §4); no stop condition was hit. Slice A (SSE) may proceed; Slice B remains gated on its own acceptance checks.
**Target baseline:** `58710b9b9b9468575257ccc141dcbaed639d468b` (DD-041 merged); runtime/Cargo remain identical to v0.5.6.
**Design:** [DD-041](dd-041-sse-streaming.md).
**Publication:** Proposed implementation plan for review. This document does not change DD-041's accepted status or implement its APIs. Keep the plan PR unmerged pending review; Gate 0 must establish feasibility before committing to public API/behavior or beginning the implementation slices.

## Outcome and limits

Ordinary NTNT applications can return SSE or WebSocket responses through existing routes, authentication, middleware and headers. An idle connection consumes native async transport state, not a thread/interpreter. HTTP behavior remains compatible. Fan-out, callbacks and retained payloads are bounded and have explicit failure semantics.

Deliver SSE independently before WebSockets, but validate the WebSocket callback seam early enough not to fossilize an incompatible SSE ownership API. No new syntax, `ExecutionMode`, general async interpreter, general actor framework, durable messaging/broker, rooms, replay, client SDK or generic native-extension dispatch redesign. Do not fix unrelated capability/GC problems inside this feature. The quality objective is a small coherent addition with demonstrable invariants—not an unprovable promise of zero future technical debt.

## 1. Concrete findings to settle before freezing implementation

### A. Separate JSON serialization from transport copies

Pinned Axum 0.7.9 `response/sse.rs:158–163` stores events in a `BytesMut`; `BytesMut::clone()` copies its contents (`bytes-1.12.1/src/bytes_mut.rs:1455–1458`). `Event::finalize()` is private (`sse.rs:375–378`). Naively publishing an Axum Event and cloning it per client does not provide shared immutable SSE frames. Wrapping an Event around each subscriber's data also allocates/formats once per recipient and does not preserve our payload-budget owner through Hyper.

**Proposed narrow DD clarification:** share pre-encoded immutable SSE frame bytes and use Axum's ordinary streaming-body adapter. A tiny bounded SSE formatter plus native heartbeat is justified here; a new HTTP server or SSE parser is not. Validate framing against WHATWG and pinned Axum behavior, converting invalid metadata to errors instead of calling panic-prone builder methods. Keep the same response-builder API.

Pinned Axum's WebSocket Message is `Text(String)` / `Binary(Vec<u8>)` (`extract/ws.rs:566–594`). It needs owned per-write data. **Serialize application JSON once, but allow bounded native WS message/codec copies.** Do not bypass the maintained codec or upgrade the HTTP stack solely to claim zero-copy WS delivery. Charge application-owned copies while pending, configure codec bounds, and measure the remaining codec allocation separately. Correct the DD's overly strong “encode transport once per protocol” wording to distinguish SSE frame caching from WebSocket framing.

### B. Opaque authority must have an owner independent of NTNT values

An NTNT response map is cloneable and may survive in a lexical cycle. A pending native subscription cannot be owned solely by an Arc hidden in that map, or a saved response alias can keep it alive after request failure. Request-owned pending state and an opaque weak/one-shot token are separate roles; see §2.

The same distinction applies to application scope: primary/root lifetime owns shutdown. Worker interpreters and scheduled task captures may reference that runtime, but dropping a worker must not cancel the entire application, and saving a handle in a cycle must not keep a stopped server accepting operations.

### C. A callback is not an ordinary HTTP bridge request

Current `HandlerRequest` is request+oneshot, and both worker loops process only HTTP (`http_bridge.rs:224–282`; `interpreter.rs:10464–10495,10816–10874`). Add a separate bounded callback queue; do not disguise a frame as a synthetic URL/request or pin a worker for a connection.

Callback invocation must use ordinary `call_function`/caller-frame cleanup and the worker's local registered function, while keeping explicit connection state outside its environment. Normalize both a runtime error and a returned `Err(...)` as callback failure—not a successful state value. Never automatically retry application callbacks after a worker panic/disconnect.

### D. Worker capability and setup semantics need an explicit check

`call_function`'s native arm has no generic `&mut Interpreter` native-function argument; imported `ws_endpoint` therefore needs a narrow interpreter-aware registration hook (`interpreter.rs:9480–9584`). Resolve the actual native callee first and recognize the canonical stdlib function identity, not a raw identifier spelling. Named import, module-qualified call and alias must agree; a user function with the same name must remain a user function. Do not add a name-only global action shortcut that steals shadowed calls.

Registration occurs before serving in primary and Worker initialization. `http_worker_request_tasks` currently distinguishes task availability after worker initialization (`interpreter.rs:1218–1225,10850–10864`); a Worker mode test alone does not prove a call is setup-time. Freeze the endpoint registry explicitly at the setup/serving transition and reject later registration, including aliases, callbacks and task-created interpreters. Do not return Unit on denial.

**Startup/eligibility protocol for applications registering realtime resources:** root setup establishes a provisional generation and freezes its definitions; each configured worker evaluates setup, freezes definitions and reports compatibility before entering either work queue. Commit the generation and emit public readiness/admit traffic only after every configured worker passes within a bounded startup deadline. Binding a socket is not readiness. The current ordering starts the server before secondary workers (`interpreter.rs:10380–10461`); moving/gating that ordering is an explicit implementation task, not something the existing bridge already provides. Conflicting definitions are a fatal startup error, not an infinite retry; transient failures may retry only within the startup deadline. Buffered-only applications retain their existing startup path.

Program identity must cover the actual loaded main and transitive application-module source bytes/resolved identities, runtime build and frozen endpoint configuration, not just the endpoint's own AST or a file timestamp. A respawned worker remains quarantined from work until it matches the committed generation. Source/helper changes cannot be silently blessed as the old generation: close old realtime connections and coordinate a fresh generation through the existing reload/restart boundary before admitting new realtime work. Gate 0 must establish that boundary; if it needs a general loader/hot-reload rewrite, stop and rescope rather than concealing it here.

Callback capability behavior must be the same on worker 0 and secondary workers. Source inspection shows JobEnqueue is absent from Worker's default capabilities while enqueue declares that requirement; this is a pre-existing behavior risk, not a verified runtime reproduction. The callback spike must exercise task spawning, scheduling, job enqueue, config/lifecycle denial and native-test rejection through the real dispatcher. Use a narrowly scoped invocation policy/guard if needed, restore it on success/error/unwind, and do not silently switch to Normal mode or broaden all existing HTTP workers. If an existing defect needs correction, propose it separately and reuse its resulting tests.

### E. Stream cancellation is not socket-write preemption

The normal timeout wraps response creation, not body lifetime. A stalled Hyper write can stop polling an SSE body; a timer inside that body cannot then force a TCP teardown. Prove what cancellation/drop actually releases on the existing Axum server before specifying a hard SSE stall/shutdown guarantee. Keep a connection permit until the transport owner really ends. Do not declare the connection gone, release its capacity, and leave the socket alive.

## 2. Small ownership model

Prefer one cohesive private native module, initially `src/stdlib/realtime.rs`, with internal submodules only when needed. Export public helpers from the existing concurrent and HTTP-server modules, not a new framework. Add small private interpreter helpers rather than another large inline server subsystem.

### Owners and handles

- **Application owner:** root interpreter owns an explicit closable runtime scope. Its shared native state contains namespace/generation, admission/payload limits, weak bus lookup, active native connections and cancellation. Attach the same runtime reference to secondary workers before source evaluation. In-process task capture carries the necessary runtime reference/bus authority, never an active HTTP request lease. A worker/task reference is not the root shutdown owner.
- **Worker-local endpoint definition:** stable application-scoped name plus immutable configuration/definition fingerprint within the complete program-generation check in §1.D; callback Values remain in that worker's ServerState. No `Rc`, captured environment or Function crosses a native channel. Compare worker registrations before committing startup; inconsistent or missing definitions fail initialization. A respawned worker becomes eligible only after it reconstructs the same program generation. Reuse this eligibility state for callback capacity; configured worker count is not live capacity.
- **Request lease:** created around `process_request` before middleware, bound to server generation and request identity, invalidated when request ownership ends. Native transport preparations register in this lease. The opaque value in a response map is a weak, request-bound, one-shot reference to one preparation. Map cloning cannot transfer ownership to another request.
- **Pending transport preparation:** holds provisional subscription/admission/configuration. Only the one selected final response may move it out of the request lease into a typed bridge reply. All unselected preparations are dropped at scope exit. Reply delivery failure/receiver cancellation drops the moved preparation; the token cannot reopen it.
- **Active native owner:** after transport commitment, owns connection permit, cancellation, subscription/inbox and callbacks/state. A WebSocket send handle is weak authority with generation/nonce checks, not lifetime ownership. Late aliases cannot keep a socket or queue alive.
- **Budgeted immutable bytes:** payload storage owns its byte-budget permit. The final Bytes reference retained by the body/transport retains that allocation's owner; ring eviction and local variable drop must not release its charge early. `Bytes::from_owner` is an available primitive, not a new refcounting implementation or proof that downstream codec copies share the owner.

Request/transport transition:

```text
prepare under request lease
 -> validate final response and take exactly one preparation
 -> move into oneshot BridgeResponse
 -> validate async request still owns upgrade/response authority
 -> attach SSE body or await WS upgrade
 -> active native owner
 -> closing (reject new work, invalidate reply sequence)
 -> transport/queues dropped, then release permits
```

Dropping any arrow must release the resources owned at that arrow. A middleware replacement, 4xx/5xx/redirect final response, bodyless HEAD/status, failed header build, failed oneshot send and invalid upgrade must not leave a live subscription. A normal JSON/text response takes exactly the current path, apart from a cheap inactive realtime-context branch.

`BridgeBody` becomes `Buffered(String) | Sse(PendingSse) | WebSocket(PendingWs)`. Do not retain an unconditional Clone derive on a one-shot transport owner. Preserve duplicate headers and response-map metadata, but validate protocol-fixed status/framing headers rather than letting application headers override upgrade or stream correctness. The async request retains OnUpgrade before conversion consumes the original body/extensions.

### Fan-out and accounting

- Use the existing Tokio broadcast primitive and a shared encoded event, not a Vec of per-subscriber NTNT values/queues.
- Bound serialization during traversal/output, including depth, encoded metadata and bytes. Do not create an unlimited JSON String and check its length afterward. Reject nested Secret values, handles and non-JSON values even with zero subscribers.
- Reserve enough budget before allocating/growing encoded output; release reservations on every rejection/no-subscriber path. Pre-encode SSE frame bytes once, and retain original JSON bytes for WS forwarding without re-running JSON serialization.
- Tokio may round channel capacity; validate/document the effective retained-message bound rather than promising an exact arbitrary capacity that the library doesn't provide. Byte budgets remain independently enforced.
- Named lookup is namespace+generation, with finite topic count and dead-entry pruning. Weak maps alone do not remove String keys. Runtime close clears the registry/resources regardless of outstanding dead aliases.
- A slow consumer loses its connection on lag, not other consumers or the publisher. A control event is best effort. Browser/application reconciliation remains required; there is no replay promise.
- SSE source Bytes held by Hyper remain charged until their final reference drops. WS application-owned message copies stay charged through flush or socket disposal; do not drop a charge just because an enqueue succeeded. HTTP/SSE **and** WS codec copies have a separately bounded, measured transport allowance outside that application quota. Pinned h2 copies small DATA into codec buffers (`h2-0.4.20/src/codec/framed_write.rs:240–261`); Hyper HTTP/1's Flatten strategy copies too (`hyper-1.8.1/src/proto/h1/io.rs:538–562`). Those copies can outlive the charged source reference. Bound physical connections, multiplexed streams and per-stream/per-connection buffering where supported; report a conservative aggregate allowance without pretending a per-stream Drop counter measures a shared HTTP/2 connection's memory. Gate 0 must verify which paths/protocols the real listener enables and their actual bounds; do not replace Hyper to hide this distinction.

## 3. Callback contract to prove, not improvise

Registration fixes the callbacks for an endpoint. Per-connection state is bounded JSON-compatible data. Allow a plain state return or `Ok(state)`; a returned `Err` or interpreter error closes the connection. No implicit Result-as-state serialization. The same normalization applies to on_open.

The native actor submits at most one callback at a time per connection and carries connection epoch+sequence on the reply. The worker takes an execution credit without blocking a worker waiting for credit, rechecks connection/deadline before invoking, then returns the next serialized state. Apply only a matching current reply. State/capability validation failure closes that connection. Native queue locks must never be held while calling NTNT code, serializing user values or waiting for the worker.

- Preserve FIFO application-message order and commit state after successful callback normalization. Accepted sends/external effects before a later failure are not rolled back.
- On open: validate initial state, authorization, origins and upgrade first. After a successful physical upgrade, on_open is the first callback; inbound data can wait only in a bounded native queue. Its failure closes an already-upgraded socket; it is not an HTTP error response.
- On close: native cleanup is unconditional. A bounded best-effort callback uses the last committed state and small redacted close information, at most once. It must not retain send authority, reopen the connection or block shutdown if the callback queue is full.
- While a callback runs, native ping/pong and close processing continue. Data reads are capped/rate-limited; one-in-flight does not permit an unbounded reader backlog.
- Work admission uses a finite shared callback queue and nonblocking reserve/try-enqueue. There are no unbounded futures waiting to enter it. Start with weighted fair selection between existing HTTP work and callback work. Limit simultaneous callbacks using **live eligible** membership: leave one HTTP execution slot when at least two eligible workers exist; one-worker operation may execute a callback but has no preemption/reserved-capacity guarantee; zero eligible workers starts no callback. Update membership/credits on failure and rejoin, releasing failed in-flight work without replay. A worker loss cannot preempt callbacks already executing: stop new callback starts above the reduced limit, prefer queued HTTP as work completes, and explicitly exclude that degraded transition from the steady-state reservation guarantee. Do not promise uninterrupted HTTP latency while the sole survivor is inside non-preemptible user code. Test failure/recovery during callback saturation, not just a healthy pool. No second interpreter pool.
- Extract one worker service helper used by worker 0 and other workers without removing worker 0's existing reload duties. Do not implement two subtly different callback execution paths or two interpreters per worker.
- Cooperative cancellation may stop queued/future work and existing yield-point natives. It does not stop an infinite loop or roll back an external API call. No automatic message retry; retry/idempotency is application protocol policy.

## 4. Dependency-ordered implementation slices

These are coherent implementation boundaries, not a requirement for one micro-PR per bullet or a reviewer per slice.

### Gate 0 — prove the two risky seams before public API code

**Ownership probe:** in an isolated worktree, exercise the current NTNT bridge with a minimal private streaming reply and native Drop counters. Cover normal disconnect, final-response replacement, oneshot receiver timeout, invalid/abandoned upgrade, slow peer, active-stream SIGTERM and retained aliases. Add many small SSE events to a stalled HTTP/2 writer if enabled at the NTNT listener, plus the applicable HTTP/1 write strategy; prove protocol/feature reachability rather than counting an unavailable protocol as tested. Measure/bound downstream transport buffers separately from source-owner Drops, including HTTP/2's shared connection/stream allowance. Determine whether the current server driver can release active transports to the required shutdown bound; do not infer it from a standalone Axum demo. No new public stdlib functions yet.

**Callback probe:** using real primary/secondary interpreters, register a private endpoint definition and execute successive event work items with state. Cover imported/transitive helpers, plain/Ok/Err/runtime-error returns, unwind -> supervised worker restart, stale epoch replies, setup rejection, canonical native-call forms and capability consistency. Delay a secondary worker and assert no public readiness/traffic admission before compatibility commits; conflicting definitions must produce a bounded fatal startup result, not a retry loop. Change only an imported callback helper before respawn and assert quarantine until a coordinated generation transition. Saturate callbacks, fail another worker and keep HTTP traffic queued: verify reduced live-capacity admission, the stated degraded-transition limitation, HTTP preference once finite callbacks complete and recovery without stale credits or replay. Use the same call-frame machinery as normal function calls, not a portable closure snapshot or new interpreter per message.

**Gate outcome:** a short recorded result in the plan/DD, viable signatures/ownership decisions and a small regression seed. Delete discarded scratch code. No public stub API, temporary global registry or parallel transport implementation lands. Stop and ask if satisfying lifecycle bounds needs a new connection driver, whole-interpreter refactor, unrestricted closure transport or significant authority change. A blocked gate is not permission to skip an invariant or to promise a smaller feature still meets it.

### Gate 0 outcome (executed 2026-10-26)

Probes ran against the real `ntnt run` server and real Worker-mode interpreters, using disposable scratch patches that have since been deleted. No public API, stub or registry landed. Pre-existing defects that Gate 0 surfaced are tracked separately so this feature does not carry them: #289 and #290 are merged; the builtin-shadowing defect is open as #293 (#291 was a superseded stopgap).

**Ownership/transport (real bridge + Axum server, HTTP/1.1 and HTTP/2):**

- A streaming body's native owner is dropped as soon as the client disconnects (0 ms observed). A stalled client does not block other requests.
- Graceful shutdown waited forever for any open stream or slow response. This was a pre-existing defect: a throttled static download also hung SIGTERM. Fixed in #289: drain bounded by the request timeout, then explicit runtime shutdown. Streams additionally need to observe a process shutdown signal and end cooperatively. A scratch implementation of that ended an active stream at ~0 s and a stalled one at the drain bound, with the owner dropped in both cases.
- HTTP/2 prior knowledge is accepted by the listener, so h2 is a reachable path and must be tested and bounded. Stalled-client buffering measured outside the kernel send queue: HTTP/1.1 ≈115 KB; one h2 stream ≈0.5 MB; 8 h2 streams on one connection ≈8 MB (≈1 MB per stream inside hyper/h2). Slice A must therefore (a) produce frames only when the body is polled, from the bounded broadcast receiver, never pre-buffering per subscriber, and (b) set explicit h2 per-stream send-buffer and concurrent-stream limits on the server builder. This is configuration of the existing stack, not a new connection driver.

**Callbacks and worker semantics (real `Interpreter`, Worker mode):**

- A callback invoked through ordinary `call_function` with explicit state in/out behaves as §3 requires. Plain value and `Ok(state)` commit; returned `Err` and runtime errors (e.g. a failed `assert`) are failures and do not commit. Task spawning inside a callback works. `call_depth`, the environment frame and the deferred list are restored after every call, including failures. Transitive helper imports resolve.
- Canonical identity: an interpreter-aware native keyed on the resolved `NativeFunction` (the `sort_by` path) agrees across named import, alias and module-qualified calls. The spelling-keyed fast paths in call evaluation, and server-action dispatch by raw identifier, do **not** let user definitions shadow builtins. This is pre-existing and unfixed; the replacement design is tracked in #293. The realtime registration hook must use the value-based (resolved callee) path only and must not rely on spelling-keyed dispatch.
- Capability parity: secondary workers silently dropped `enqueue` (capability gate returned Unit): with 4 workers, 30 of 40 request-time enqueues were lost. Pre-existing; fixed in #290. Callback execution must reuse the request-phase capability set, and tests must assert real return values, because missing capabilities become silent Unit.
- Two work queues: `flume::Selector` over the HTTP and callback receivers blocks without busy-waiting and wakes on either. HTTP preference after a callback is a `try_recv` policy in the shared worker service, with no second interpreter pool.
- Source identity: a worker's tracked imports include nested imports, so a fingerprint over main + tracked import bytes changes when only a transitive helper changes. This establishes only the source-bytes input to §1.D's generation identity. Resolved import identities, runtime build identity and frozen endpoint configuration, plus the generation barrier and respawn quarantine themselves, were not exercised and remain Slice B acceptance checks.
- Readiness ordering: confirmed by source that the async server binds and emits readiness before secondary workers start, and workers retry startup indefinitely. The realtime readiness barrier (§1.D) must gate the existing startup handshake on worker setup/compatibility and bound startup retries. This is ordering work in `run_async_http_server`, not a loader rewrite.

**Not yet exercised:** WebSocket upgrade with a held `OnUpgrade` (needs Axum's `ws` feature; Slice B), respawned-worker quarantine end-to-end, and fairness under worker loss with saturated callbacks. These remain Slice B acceptance checks, not assumed results.

### Slice A — useful SSE, complete lifecycle and caps

Likely files: `src/stdlib/realtime.rs` (new private core), `src/stdlib/http_bridge.rs`, `src/stdlib/http_server_async.rs`, `src/stdlib/http_server.rs`, narrow interpreter/context/capture hooks, `src/stdlib/concurrent.rs`, `src/stdlib/mod.rs`, `src/typechecker.rs`, serializers/boundary consumers, and tests.

1. Implement application namespace ownership, generation/readiness membership barrier and request lease; upgrade bridge body without changing the buffered path. Establish the shared source generation before workers can publish/subscribe, even though endpoint callback parity is added in B. Install explicit cancellation/ownership checks around final response, rather than an ever-growing global token map.
2. Implement broadcast/publish, bounded event encoding and handles; add only the explicit Broadcast task-capture case. Opaque response/endpoint/peer handles remain rejected by generic serialization. Audit all Value consumers, equality/debug/type-name handling and JSON/task/channel/job/cache boundaries; reject nested handles consistently.
3. Implement `sse(bus, options?)` returning the opaque native token in the existing response envelope. Materialize a shared-byte stream with heartbeat on the async side; preserve headers/auth and enforce connection/topic/payload limits from first release.
4. Register shutdown/reload cancellation before bridges/workers are joined, and release resource ownership before application hooks. Test actual cancellation behavior established at Gate 0. Do not introduce a dedicated thread per stream.
5. Add one small native example and focused native protocol fixture; update public signatures/docs from the real implementation. Keep the WebSocket response enum variant internal until Slice B exists, or add it with B if Rust exhaustiveness is cleaner. No public unreachable placeholder.

SSE slice done means a production-style multi-worker process sends live data to clients admitted through different workers, denial paths open no stream, slow clients remain bounded, shutdown satisfies the measured contract, and ordinary buffered responses regress neither semantics nor material performance. SSE remains independently shippable if B is delayed.

### Slice B — WebSocket upgrade and bounded callback dispatch

1. Enable Axum's `ws` feature, inspect the resolved dependency/advisory delta, and add only direct dependencies actually imported. No speculative HTTP-stack upgrade. Capture OnUpgrade before request consumption; preserve ordinary admission denials, headers and cookies.
2. Extend A's generation/readiness barrier with per-worker endpoint registration/freeze, callback compatibility checks and quarantined rejoin from §1.D. Distinguish fatal parity failure from transient initialization retry, and bound initial retries. Implement the single interpreter-aware canonical native registration hook, application namespace attachment and scoped invocation context; cover aliases/shadowing and execution modes. Preserve buffered-only startup behavior.
3. Extend worker service to select bounded callback jobs fairly using live eligible capacity, the normal user-call cleanup and explicit capability contract. Test error -> success, callback saturation with worker loss/recovery and restart -> new connection; never re-execute an uncertain prior message. Document the non-preemptible degraded-transition exception rather than claiming a hard reservation across an arbitrary crash.
4. Implement one native WS owner with limited owned send buffers, bounded input, per-connection state/reply sequence, native ping/pong, close/lease and idempotent weak send handles. Separate outboxes from actor state locks so a worker calling ws_send cannot deadlock its own actor.
5. Connect optional broadcast source without copying/serializing NTNT state per subscriber; bounded owned WS payload copies are allowed. Complete exact-Origin policy, finite lease behavior, message/metadata validation and callback hooks.
6. Add echo plus a small authorized message fixture, not a room/chat framework. Regenerate public signatures/docs and migration guidance. Protect established-session semantics explicitly; handshake authentication alone does not provide immediate revocation.

### Release closure — measured, not aspirational

Choose final default limits and any lease policy from the measurements/consumer needs, without expanding the public option surface. Replace the DD's provisional numbers/stronger copy claims with actual supported semantics. Document process-local publication, reconnect reconciliation, supported HTTP versions, bounded vs non-preemptible work and auth refresh behavior. Do not label a deferred gate solved by documentation alone.

## 5. Focused verification and reuse

**Reuse:** `tests/support/system_fixture.rs` supplies temporary workspaces, actual compiled CLI path, port-zero JSON readiness, bounded output and child cleanup; `tests/http_listen_options_tests.rs` already tests async production workers and sync CLI paths. Extend only the protocol client/lifecycle helpers needed. Do not create another web-app test framework or use fixed ports/blind readiness sleeps. The fixture's Drop kills a child; graceful shutdown tests must signal/wait and assert graceful exit separately before that fallback can run.

Suggested new native integration targets: `tests/realtime_sse_tests.rs` and `tests/realtime_websocket_tests.rs`. These names are planned files, not existing executed tests. Place pure encoding/ownership tests with the native core. Retain existing tests in `tests/http_listen_options_tests.rs`, `tests/concurrency_tests.rs`, `src/interpreter/frame_restore_tests.rs`, auth tests, language/type-checker tests and existing CLI/native-test rejection checks.

Each slice needs these relevant assertions—not a Cartesian test matrix:

- Real parser/import/route path and multi-worker capture/namespace correctness; consistent alias/type-mode diagnostics and named native function registration.
- First/next message, Unicode/multiline JSON, binary exact bytes, metadata control injection/oversize rejection, optional IDs without a replay claim, zero-subscriber validation and backpressure error handling.
- Budget retained through final byte clone and through WS flush/socket-drop, simultaneous different payloads, effective broadcast capacity, slow receiver and many healthy receivers. Test small-event stalled HTTP/2 writes where supported and applicable HTTP/1 copying; bound/measure transport copies independently of payload-owner Drop counters. Count serializer calls on the real API path.
- Permit/topic/queue counts return to baseline after all rejected/cancelled/failed paths; retained aliases and failing hooks do not defeat cleanup. Late callbacks cannot mutate a reopened generation or continue sending.
- No overlapping per-connection callbacks, next state observed by the next message, on_open ordering and callback-failure normalization, stale replies after close, panic/restart without automatic replay, queue saturation and HTTP fairness under active callback load. Combine worker loss/reinitialization with callback saturation and continuing HTTP traffic; distinguish steady-state reservation from non-preemptible degraded behavior, then assert fair recovery. Also cover delayed/conflicting initialization, bounded failure without readiness and imported-helper-only changes before worker respawn.
- 401/403/redirect/Set-Cookie, auth refresh, WS Origin/canonical-public-origin configuration, schema/secret/handle rejection, session expiry/command reauthorization boundaries. Native test/sync CLI rejection is explicit; zero executed assertions cannot count as passing streaming coverage.
- Cross-platform socket fixtures and process isolation use the existing CI/nextest model. Test mode/generation changes without teaching the synchronous HTTP server a second streaming stack.

Relevant commands verified to exist in this checkout/toolchain; run only after the corresponding code/targets exist:

```sh
cargo fmt -- --check
cargo build --profile dev-release --locked
cargo test --locked --lib realtime
cargo test --locked --test realtime_sse_tests
# After B exists:
cargo test --locked --test realtime_websocket_tests
# Fresh optimized binary before generation:
./target/dev-release/ntnt docs --generate
# Final implementation integration: checkout-local target, process-per-test isolation
RUST_MIN_STACK=8388608 cargo nextest run --locked
```

The planning environment used nextest 0.9.137; hosted CI installs nextest independently. Do not claim tool-version parity. For authoritative final integration, leave CARGO_TARGET_DIR unset so CLI-inclusive tests can find the built binary. Use a bounded documented worker/thread count appropriate to available resources. Read the then-current CI invocation rather than treating this plan's command list as a replacement for repo policy.

For ordinary HTTP/CLI/template baseline comparison, reuse `scripts/bench/run-benchmarks.py --quick --skip-build --ntnt-bin <exact-built-binary> --output-dir <scratch-output>`. Its help/options were checked. Compare base and candidate on the same machine/profile. Add only small realtime-specific load clients/fixtures; keep large wrk laboratories in ntnt-benchmarks. Record ordinary HTTP throughput/p95/p99, CPU, thread count, idle memory and stalled-client/active-callback impact. A standalone channel throughput number is not a server performance result. Set an agreed regression threshold after observing baseline noise, before accepting candidate results.

## 6. Maintainability acceptance

- Native lifecycle state and ownership are explicit and local; no cyclic strong registry or externally forgeable marker/ID.
- Application serialization once; transport-copy costs truthful and bounded. No home-grown WS/HTTP codec or dependency upgrade justified only by cosmetic zero-copy claims.
- One callback invocation implementation, one connection-state authority, existing frame restoration, no worker affinity/new interpreter per frame, no generic continuation/binder/actor machinery.
- No hardcoded application integrations, mandatory Redis, room routing protocol or background scheduler per connection. Cross-process publication remains an application handoff, not a hidden promise.
- Existing buffered HTTP behavior and runtime mode/capture behavior remain compatible; any necessary pre-existing defect fix is separately scoped rather than buried.
- Tests assert the actual fast/ownership path, cleanup and inverse denials. Metrics/counters needed for tests remain private/test-only unless a clear operator need justifies a stable public API.
- One focused independent implementation review can cover meaningful concurrency/runtime changes; additional review is for new material uncertainty, not each commit or a score. Fix concrete findings together and rerun affected checks plus the existing release/integration gates.

## 7. Evidence and remaining uncertainty

**Actually observed (pre-Gate-0 design pass):** source/locked-dependency inspection at the stated baseline; exact Tokio 1.53.3 channel probe from the design pass; exact bytes 1.12.1 ownership probe in this pass. The latter verified shared backing across Bytes clones/slices and that a budget-owner Drop happens once only after the final byte reference. Benchmark runner help and local nextest version were queried. At that point no streaming server, WS callback dispatcher, native cross-worker prototype, performance benchmark, or feature regression suite had been executed.

**Actually observed (Gate 0):** disposable probes against the real `ntnt run` server and Worker-mode interpreters, recorded under §4 "Gate 0 outcome". These were feasibility probes, not the feature implementation or its regression suite.

**Still to prove after Gate 0:** request-lease teardown through the final-response path (Slice A), WS codec-buffer allowance and upgrade ownership, generation/readiness barrier implementation and quarantined respawn, and HTTP fairness/RSS under live load and worker loss (Slice B). See the Gate 0 outcome under §4 for what was measured. Suggested DD defaults, the tentative implementation estimate and a documentation bot's confidence do not settle them.

**Independent review disposition:** the reviewer found no blocker to proceeding with Gate 0 and three medium-severity acceptance gaps, not proof that the final feature is feasible. Source inspection confirmed the startup-before-workers ordering, supervised workers rereading source, and the downstream HTTP codec copy paths. This revision addresses them with (1) explicit readiness/generation/quarantine steps and delay/conflict/helper-change cases, (2) a separate HTTP/SSE as well as WS transport-buffer allowance and stalled-writer cases, and (3) live-eligibility admission plus a truthful degraded-capacity contract and combined failure/load test. These are plan corrections, not executed feature regressions or a second reviewer verdict. Numeric limits, final lease policy and any requirement for immediate authorization revocation remain decisions to settle before public API/behavior freeze. No additional design merge or feature implementation has occurred.
