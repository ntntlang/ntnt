# DD-065: Agent Harness as a Versioned Standard Library (`std/harness`)

**Status:** Draft — for review
**Author:** Larri
**Created:** 2026-07-08
**Depends on:** DD-037 (concurrency & jobs), DD-041 (SSE streaming), DD-047 (module-as-namespace imports), DD-062 (secure compiled extension libraries), the Intent Assertion Language (IAL) + contract system, and the `RuntimeCapability` / execution-mode model
**Relates to:** DD-060 (AI-native developer experience)

> This is a design proposal. Except where a section is explicitly labelled
> **(existing)**, every API name, type, and module path below is
> **proposed** and subject to change through review. The point of the doc
> is to fix the *shape of the primitives* and the *versioning and
> extension model* before any implementation begins.

---

## Table of Contents

1. [Vision & Motivation](#1-vision--motivation)
2. [Design Principles](#2-design-principles)
3. [Core Primitives](#3-core-primitives)
4. [Versioning Strategy](#4-versioning-strategy)
5. [Extension & Integration Points](#5-extension--integration-points)
6. [Relationship to Existing Modules](#6-relationship-to-existing-modules)
7. [Security Model](#7-security-model)
8. [Competitive Analysis](#8-competitive-analysis)
9. [Phase Plan](#9-phase-plan)
10. [Open Questions](#10-open-questions)
11. [Version History](#11-version-history)

---

## 1. Vision & Motivation

Every agent framework today is built **with** a language: LangChain/LangGraph,
CrewAI, the OpenAI Agents SDK, Mastra, the Vercel AI SDK. The agent is a
*service the application talks to* — an opaque call that returns text or
JSON, invisible to the host language's type system, sandbox, and verifier.
The framework then reinvents, in application code, the things a language
normally provides: orchestration (a graph engine), sandboxing (a tool
allowlist), verification (output validators), durability (a queue), and
streaming (an event bus).

ntnt is positioned to build the agent harness **in** the language instead —
as a standard-library module (`std/harness`) whose core makes the agent a
**first-class participant in the program** rather than an external box the
program calls. The bet is not "ntnt gets agent support." It is that ntnt
*already has*, for unrelated reasons, the exact primitives a harness
normally fakes:

| A harness needs… | ntnt already has… **(existing)** |
|---|---|
| Tool sandboxing / authorization | `RuntimeCapability` + capability-gated execution modes |
| Output verification & repair loops | IAL intent specs, `requires`/`ensures` contracts, the DD-063 repair loop |
| Orchestration (subagents, fan-out) | `std/concurrent` — `spawn`, `parallel`, `race`, channels |
| Durable long-running agents | `std/jobs` — enqueue, retries, workers, audit log (DD-042) |
| Token / event streaming | `std/sse` (DD-041) — broadcast bus, bounded queues, backpressure |
| Long-term memory | `std/kv`, `std/db` |
| Structured outputs | the type system + strict-mode boundary validation |
| Fast iteration on prompts/skills | hot reload |

Because these exist, an ntnt harness is both **unusually cheap to build**
(it is mostly composition over primitives already shipping) and **able to
offer guarantees a library harness structurally cannot**: an agent call
that runs under a capability scope the runtime enforces, whose output is
checked against an intent spec the runtime already knows how to evaluate,
composed with concurrency and durability the runtime already provides.

**Why version it.** The AI substrate — model APIs, tool-calling protocols
(e.g. MCP), context-management best practices, prompt conventions — changes
monthly. A library can rev independently; a standard library is a stability
commitment. We resolve that tension by **versioning the harness at the
module path** (`std/harness/v1`, `std/harness/v2`, …). An application pins a
version by its import, keeps the *primitive vocabulary* stable across
versions, and opts into a new *approach* (new default strategies, a new
provider protocol) by changing one import line — not by rewriting the app.
This is the mechanism that lets a stdlib feature track a fast-moving field
without breaking existing apps.

### The two approaches, side by side

```
Traditional (harness WITH a language)        Proposed (harness IN the language)
────────────────────────────────────        ──────────────────────────────────
agent = opaque external service              agent = language-level call
tool allowlist (hand-rolled)                 tool = capability-scoped function
output validator (bolted on)                 output verified by IAL/contracts
graph engine (reinvented)                    orchestration = spawn/parallel
queue adapter (reinvented)                   durability = std/jobs
event bus (reinvented)                       streaming = std/sse
config = YAML/code outside the type system   config = typed, tested, hot-reloaded source
```

The differentiation is narrow but real: apps where the agent and the
deterministic code are **deeply interleaved and mutually governed** — the
agent invokes your capability-gated, contract-checked functions, and your
functions invoke agents whose outputs are intent-checked — all one program
the runtime can sandbox, hot-reload, and verify end to end.

---

## 2. Design Principles

1. **Primitives over framework.** `std/harness` ships a small, sharp
   vocabulary (Agent, Tool, Skill, Context, Memory, Provider) and the verbs
   to run them. Opinionated orchestration patterns live in userland or in
   optional companion modules, not in the core.

2. **Providers and strategies are adapters; primitives are stdlib.** The
   *churny* parts — model-provider protocols, prompt construction, default
   context-compaction algorithm — are swappable adapters behind stable
   interfaces. The *stable* parts — the primitive types, the capability
   scoping, the verification hook, the call verbs — are the stdlib surface.
   The version boundary is drawn exactly along this seam (§4).

3. **Verified, not guaranteed.** Agents are non-deterministic; ntnt sells
   verification. We reconcile this honestly: an `ensures`/IAL policy on an
   agent result is a **runtime gate plus bounded repair**, never a static
   proof. The harness must never let a contract over an agent *read* like a
   guarantee. (This mirrors the DD-041 discipline of labelling
   "verified-not-guaranteed" behaviour precisely.)

4. **Tool authorization is capability scoping.** An agent runs under a
   capability set fixed at definition time. A tool declares the
   capabilities it needs. An agent may invoke a tool only if the tool's
   requirements are a subset of the agent's grant. This reuses the
   **existing** `RuntimeCapability` mechanism (the same one that stops a
   Worker from calling `listen()`); it is the security spine of the design
   (§7).

5. **Compose over existing primitives.** Subagents are `spawn`; fan-out is
   `parallel`; a durable background agent is a `std/jobs` job; streaming is
   `std/sse`; memory is `std/kv`/`std/db`. The harness does not re-implement
   any of these — it wires them together.

6. **No new keywords.** Consistent with the SSE lesson (DD-041): agents are
   *values* and agent runs are *function calls*. Verification is a policy
   passed as configuration, not new syntax. Nothing here touches the parser.

7. **Configuration is source.** An agent, its tools, and its skills are
   ordinary ntnt declarations — typed, testable (`ntnt test`), verifiable
   (`ntnt intent check`), and hot-reloadable. There is no separate YAML/JSON
   agent-config format outside the language's reach.

---

## 3. Core Primitives

The vocabulary is deliberately small. **All identifiers below are proposed.**

### Nouns

- **`Agent`** — a configured agent: provider, model, system persona, granted
  tools, mounted skills, context strategy, memory handle, and verification
  policy. Declared as part of the app.
- **`Tool`** — a capability-scoped ntnt function exposed to an agent. The
  tool's JSON schema is *derived from the function's typed signature and
  doc/contract* rather than hand-written (a native advantage — see §5.2).
- **`Skill`** — a bundle of instructions + tools + optional seed context,
  distributed as an importable module (§5.5).
- **`Context`** — the managed working set for a run: a resource with a token
  budget and a pluggable assembly/compaction strategy (§5.3).
- **`Memory`** — durable state spanning runs, backed by `std/kv`, `std/db`,
  or a custom store (§5.4).
- **`Session`** — a stateful, multi-turn conversation handle (a `Run` is a
  single invocation; a `Session` threads context and memory across runs).
- **`Provider`** — a model-backend adapter (Anthropic, OpenAI, local, …)
  behind a stable interface (§5.1). The primary churn sink.

### Verbs (the call surface)

The same agent can be invoked in three shapes, each reusing an existing
subsystem rather than inventing one:

```ntnt
// PROPOSED. Declaring an agent as part of the app.
import { agent, tool } from "std/harness/v1"

let support = agent(map {
    "provider": "anthropic",
    "model":    "claude-fable-5",
    "system":   "You are a support agent for an e-commerce store.",
    "tools":    [lookup_order, issue_refund],   // capability-scoped fns
    "memory":   "support-sessions",             // std/kv namespace
    "context":  "sliding-window",               // named context strategy
    "verify":   fn(reply) { len(reply) > 0 },   // runtime gate (§2.3)
    "repair":   2,                              // bounded re-prompt attempts
})
```

**One-shot** (returns a typed, verified value):

```ntnt
get("/support", fn(req) {
    let data = parse_json(req) otherwise { return status(400, "Bad JSON: #{err}") }
    // run() applies the agent's verify policy and repair budget, then
    // returns the result (or an Err the caller handles).
    let reply = run(support, data["message"])?
    return json(map { "reply": reply })
})
```

**Streaming** (reuses `std/sse` — DD-041 — for token/event fan-out):

```ntnt
import { sse } from "std/sse"
import { stream } from "std/harness/v1"

get("/support/stream", fn(req) {
    // stream() returns an SSESubscription-compatible source; each token or
    // tool-call event is one SSE event. The SSE write task drains it.
    return sse(stream(support, req.params.message))
})
```

**Durable** (reuses `std/jobs` — DD-037 — for long-running/background work):

```ntnt
import { enqueue } from "std/jobs"

// A long research agent runs under the job system: retries, worker
// isolation, and the DD-042 audit log come for free.
enqueue("run_agent", map { "agent": "researcher", "input": topic })
```

**Subagents** (reuse `std/concurrent` — DD-049 — for orchestration):

```ntnt
import { parallel } from "std/concurrent"

// Fan out to three specialist agents; the language's concurrency IS the
// orchestration engine — no separate graph runtime.
let [plan, risks, budget] = parallel([
    fn() { run(planner, brief) },
    fn() { run(risk_analyst, brief) },
    fn() { run(estimator, brief) },
])
```

### Verification & repair

Verification is a **policy on the agent or the run**, never new syntax
(§2.6). Three sources, in increasing power:

- a predicate closure (`"verify": fn(result) { … }`),
- a `requires`/`ensures` **contract (existing)** on the ntnt function that
  post-processes the agent output,
- an **IAL intent spec (existing)** bound to the agent, checked with the
  same machinery as `ntnt intent check`.

On failure the runtime re-prompts up to `repair` times (feeding the
violation back as context — the DD-063 repair-loop pattern), then surfaces
an `Err`. The contract *gates and repairs*; it does not *prove* (§2.3).

---

## 4. Versioning Strategy

The defining constraint: **track a fast-moving field from inside a stable
standard library.** The mechanism is module-path versioning.

### 4.1 Module-path versioning

```ntnt
import { agent, run } from "std/harness/v1"   // an app pins v1
import { agent, run } from "std/harness/v2"   // a later app pins v2
```

Both versions are installable and resolvable simultaneously (the module
system already resolves distinct namespaces — DD-047). An application
selects an *approach* by its import, not by upgrading a dependency in place.

### 4.2 Stable vocabulary vs versioned behaviour

The design goal the maintainer set: **keep the API surface consistent
across versions**, so most of an app's call sites survive a version bump.
We achieve that by splitting what a version owns:

| Held stable across versions (the *vocabulary*) | Owned by each version (the *behaviour*) |
|---|---|
| Primitive types: `Agent`, `Tool`, `Skill`, `Context`, `Memory`, `Session` | Default **context strategy** (sliding-window vs summarize vs retrieval) |
| Call verbs: `agent()`, `run()`, `stream()`, `session()` | Default **provider protocol** (e.g. v1 = classic tool-calling; v2 = MCP-native) |
| Extension interfaces: `Provider`, `ContextStrategy`, `MemoryStore`, `Tool` shape | **Prompt construction** & system-prompt conventions |
| Capability-scoping semantics for tools | Default **memory policy** & context-budget defaults |
| The verification/repair policy shape | The bundled **built-in skills** and their prompts |

Ideal migration from `v1` to `v2` is: **change the import line**, re-check
`ntnt intent check`, and adjust config only where a default genuinely
changed. Call sites (`run(agent, input)`) should not move.

### 4.3 Coexistence & incremental migration

Because versions are distinct namespaces, an app can import **both** and
migrate one agent at a time:

```ntnt
import { agent as agent_v1 } from "std/harness/v1"
import { agent as agent_v2 } from "std/harness/v2"

let legacy_support = agent_v1(map { … })   // unchanged
let new_research   = agent_v2(map { … })   // on the new approach
```

### 4.4 Providers version independently (the churn sink)

Model providers change faster than harness *design*. Provider adapters are
therefore versioned **separately** from the harness core (§5.1): a new model
or a provider-API change ships as a new adapter, not a new `std/harness`
version. `std/harness/v1` can gain support for a new model without a v2.
A new `std/harness` version is reserved for a change in the *harness design*
(context model, orchestration defaults, tool protocol), not the roster of
models.

### 4.5 Compatibility policy

- Within a version: **additive only** (new optional config keys, new
  extension hooks). No breaking changes to a shipped version.
- Across versions: breaking changes are allowed only at the version
  boundary, and each is documented with a migration note.
- Support window: a version is supported for a defined window after its
  successor ships (exact policy is an open question — §10).
- `ntnt lint` should warn when an app imports a version past its
  deprecation window.

---

## 5. Extension & Integration Points

This is the heart of the design: the **seams** where app developers extend
or replace harness behaviour. Each seam is a stable interface with a default
implementation the app can override. **All interfaces below are proposed.**

### 5.1 Providers (model backends) — the primary seam

A `Provider` turns a normalized request (messages, tool schemas, params)
into a normalized response stream (text deltas, tool-call events, usage).
Everything provider-specific lives here; the rest of the harness is
provider-agnostic.

```ntnt
// PROPOSED interface shape (conceptual).
// register a custom or self-hosted backend
register_provider("my-llm", fn(request) {
    // request: { messages, tools, params }
    // returns a stream of { type: "text"|"tool_call"|"done", ... } events
    …
})

let a = agent(map { "provider": "my-llm", "model": "…", … })
```

Built-in adapters (Anthropic, OpenAI, local) ship as versioned adapters
(§4.4). Native, sandboxed adapters can be delivered via **DD-062 secure
compiled extension libraries** — a provider that holds an API key and makes
network calls is exactly the "trusted, capability-bounded native extension"
DD-062 describes.

### 5.2 Tools (capability-scoped functions)

A tool is an ordinary ntnt function exposed to an agent. Two native
advantages over library harnesses:

1. **Schema derivation.** The tool's parameter schema is derived from the
   function's typed signature and its doc/`requires` contract — not
   hand-authored and kept in sync manually.
2. **Capability scoping.** The tool declares the capabilities it needs; the
   agent can invoke it only if granted (§7).

```ntnt
// PROPOSED. An ntnt function with a contract…
fn issue_refund(order_id: String, amount: Float) -> Bool
    requires amount > 0.0
{ … }

// …exposed as a capability-scoped tool. Schema comes from the signature.
tool(issue_refund, map {
    "description":  "Refund an order by id",
    "capabilities": ["db.write", "payments.refund"],
})
```

### 5.3 Context strategies

How the working context is assembled, budgeted, and compacted is pluggable.
The default is versioned (§4.2); an app can supply its own.

```ntnt
// PROPOSED interface shape.
register_context_strategy("summarize-oldest", map {
    "assemble": fn(history, budget) { … },   // build the working set
    "compact":  fn(context) { … },           // when over budget
})

let a = agent(map { …, "context": "summarize-oldest" })
```

Built-ins to ship: `sliding-window`, `summarize-oldest`, and
`retrieval-augmented` (backed by a `MemoryStore`, §5.4).

### 5.4 Memory backends

Long-term memory spans runs and sessions. The default backs onto `std/kv`;
an app can bind `std/db` or a custom store implementing the `MemoryStore`
interface (write, query, prune).

```ntnt
// PROPOSED.
register_memory_store("pgvector", map {
    "write": fn(namespace, record) { … },
    "query": fn(namespace, query, k) { … },   // semantic recall
    "prune": fn(namespace, policy) { … },
})

let a = agent(map { …, "memory": memory("pgvector", "support-sessions") })
```

### 5.5 Skills (and skill sources)

A skill is a versioned, typed, testable module bundling instructions +
tools + optional seed context — the language-level analogue of the
markdown-file-loaded-by-convention pattern. Skills are `import`-ed like any
module and mounted on an agent.

```ntnt
// PROPOSED. A skill module exports a Skill value.
import { refund_handling } from "skills/refunds"

let support = agent(map { …, "skills": [refund_handling] })
```

Because skills are modules, they get module resolution, type checking,
`ntnt test`, `ntnt intent check`, and hot reload. A `SkillSource` extension
lets teams load skills from a registry, a directory, or a remote catalog.

### 5.6 Interceptors / hooks (middleware)

Cross-cutting concerns wrap the agent run like HTTP middleware wraps a
request. Hooks fire at defined points; each can observe, mutate, short-
circuit, or annotate.

```ntnt
// PROPOSED hook points.
use_agent_hook("before_request", fn(ctx) { … })   // redact, inject, cache-check
use_agent_hook("on_tool_call",   fn(call) { … })  // authorize, rate-limit, log
use_agent_hook("on_token",       fn(tok) { … })   // stream side-channel
use_agent_hook("after_response", fn(res) { … })   // validate, cache-store
use_agent_hook("on_error",       fn(err) { … })   // fallback, escalate
```

This is where logging, redaction, rate-limiting, caching, and cost tracking
plug in without touching the agent definition.

### 5.7 Verification & repair strategies

The verification policy (§3) is itself an extension seam: an app can supply
a custom validator, bind an IAL spec, or replace the repair strategy (how a
violation is fed back and how many attempts are allowed). This lets a team
encode domain-specific "did the agent actually do the job" checks as
first-class, testable artifacts.

---

## 6. Relationship to Existing Modules

The harness is a **composition layer**, not a greenfield subsystem. Each
capability maps to a module that already exists or is already being built:

| Harness capability | Backed by | Status |
|---|---|---|
| Subagents, fan-out, races | `std/concurrent` (`spawn`, `parallel`, `race`) | **existing** (DD-037/049) |
| Durable / background agents | `std/jobs` (enqueue, retries, workers, audit) | **existing** (DD-037/042) |
| Token & event streaming | `std/sse` (broadcast bus, backpressure) | **in design** (DD-041) |
| Memory / recall | `std/kv`, `std/db` | **existing** |
| Tool sandboxing | `RuntimeCapability` + execution modes | **existing** |
| Output verification | IAL intent specs + `requires`/`ensures` | **existing** |
| Prompt/skill iteration | hot reload | **existing** |
| Native provider adapters | DD-062 secure compiled extension libraries | **in design** |

That mapping is the strongest argument for feasibility: the harness is
mostly *glue* if the primitives are the right shape — which is exactly why
this DD prioritizes primitive shape over feature breadth.

---

## 7. Security Model

Agents are a governance problem, and governance is where the native
approach earns its keep.

- **Tool authorization = capability scoping.** An agent is defined with a
  capability grant. Each tool declares required capabilities. The runtime
  refuses any tool call outside the agent's grant — enforced by the
  **existing** capability mechanism, not an ad hoc allowlist. An agent
  cannot invoke effects it was not granted, no matter what the model emits.

- **The prompt-injection trust boundary.** Agent output (and any content the
  agent ingests) is **untrusted**. The boundary that matters is the *tool
  effect*, and that boundary is the capability check. A prompt-injected
  agent still cannot exceed its capability grant. This is a stronger,
  clearer boundary than "the model was told not to."

- **Secrets.** Provider API keys live in the provider adapter (loaded via
  `std/env`), never in the agent's context window or tool arguments. Tools
  receive only what their typed signature declares.

- **No capability escalation.** A subagent's grant is a subset of its
  parent's; a skill cannot widen the agent's grant. Grants only narrow.

- **Auditability.** Durable agent runs inherit the DD-042 job audit log;
  tool calls are natural `on_tool_call` hook points (§5.6) for logging and
  policy enforcement.

---

## 8. Competitive Analysis

| Capability | `std/harness` (proposed) | LangGraph | CrewAI | OpenAI Agents SDK | Vercel AI SDK |
|---|---|---|---|---|---|
| Tool sandboxing | Language capability scoping | App-level allowlist | App-level | App-level | App-level |
| Output verification | Native (IAL + contracts) | Custom validators | Custom | Structured outputs | Structured outputs |
| Orchestration | Language concurrency (`parallel`) | Graph engine | Crew abstraction | Handoffs | Manual |
| Durability | `std/jobs` (native) | External | External | External | External |
| Streaming | `std/sse` (native) | Callbacks | Manual | Streaming API | Streaming (strong) |
| Config surface | Typed, tested, hot-reloaded source | Python | Python/YAML | Python | TS |
| Versioned surface | Module-path versions (`/v1`, `/v2`) | Semver package | Semver | Semver | Semver |
| Provider breadth | Adapters (narrower initially) | Broad | Broad | OpenAI-centric | Broad |

Honest read: library harnesses win on **ecosystem breadth** (every
integration is a package away) and **provider coverage** on day one. The
native harness wins on **cohesion and governance** — sandboxing,
verification, durability, and streaming that are *the same primitives the
rest of the app uses*, plus a config surface that is real, typed, testable
source. The versioned module path is a differentiator no package-based
harness offers: two harness generations coexisting, selected per-import.

---

## 9. Phase Plan

**Phase 0 — Primitives spike.** The thinnest possible slice to validate the
core thesis: `agent()` as an ordinary builtin returning a typed value,
running under a capability scope, with **one** `verify`-triggered repair
attempt and a single hard-coded provider. Goal: confirm the
capability/verification synergy feels as native in practice as on paper —
exactly how DD-041 treats `sse()` as an ordinary response builder before
building the rest.

**Phase 1 — `std/harness/v1` core.** `Agent`, `Tool` (with schema
derivation + capability scoping), the `Provider` interface with an Anthropic
adapter, one-shot `run()` and streaming `stream()` (over `std/sse`), memory
over `std/kv`, one default context strategy, and the hook system (§5.6).
**The versioning mechanics land here** (module-path resolution, the
stable-vocabulary boundary).

**Phase 2 — Composition & skills.** Skills as modules (§5.5), durable agents
over `std/jobs`, subagent orchestration helpers over `std/concurrent`, and
additional providers as independently-versioned adapters.

**Phase 3 — Verification & advanced context.** IAL-spec binding for agent
outputs, richer repair strategies, additional context strategies
(summarize, retrieval), and integration with intent studio for supervising
agentic behaviour (ties to DD-060).

Later versions (`std/harness/v2`+) are reserved for changes in harness
*design* — a new tool protocol, a new default context model — per §4.4.

---

## 10. Open Questions

1. **Provider interface shape.** What exactly is the normalized
   request/response event stream, and how much of tool-calling is
   normalized vs passed through per-provider?
2. **Context strategy interface.** What is the minimal `assemble`/`compact`
   contract, and how does a strategy see the token budget and memory store?
3. **IAL binding to agent output.** How does an intent spec attach to an
   agent result — by return type, by a named spec, or inline — and how does
   a violation feed the repair loop?
4. **Tool schema derivation.** How much schema can be derived from a typed
   signature + contract, and what must remain hand-annotated
   (descriptions, enums)?
5. **Streaming ⊗ tool-calling.** How do interleaved token deltas and
   tool-call events map onto the SSE event model (DD-041), and where do tool
   results re-enter the stream?
6. **Memory portability across versions.** If v2 changes the memory policy,
   how does a v1 memory namespace migrate — or do versions share a
   version-neutral store contract?
7. **How much orchestration is stdlib vs userland?** Where is the line
   between "primitives + `std/concurrent`" and bundled orchestration
   patterns (supervisor, tournament, loop-until)?
8. **Deprecation window.** Concretely, how long is a `std/harness` version
   supported after its successor ships, and how is that enforced in `lint`?

---

## 11. Version History

| Date | Change |
|------|--------|
| 2026-07-08 | Initial draft — vision, principles, core primitives, module-path versioning strategy, extension/integration seams, security model, phase plan |
