# DD-065: The Agent Harness as a Versioned Standard Library (`std/harness`)

**Status:** Draft — for review
**Author:** Larri
**Created:** 2026-07-08
**Depends on:** DD-037 (concurrency & jobs), DD-041 (SSE streaming), DD-047 (module-as-namespace imports), DD-062 (secure compiled extension libraries), the Intent Assertion Language (IAL) + contract system, and the `RuntimeCapability` / execution-mode model
**Relates to:** DD-060 (AI-native developer experience)

> This is a design proposal. Except where a section is explicitly labelled
> **(existing)**, every API name, type, and module path below is
> **proposed** and subject to change through review. The point of this doc
> is to fix the *shape of the primitives*, the *versioning model*, and the
> *extension seams* before any implementation begins.

---

## Table of Contents

1. [Vision & Motivation](#1-vision--motivation)
2. [The Design Model](#2-the-design-model)
3. [Design Principles](#3-design-principles)
4. [Primitives](#4-primitives)
5. [Architecture: Substrate and Harness](#5-architecture-substrate-and-harness)
6. [Versioning: Harness Editions](#6-versioning-harness-editions)
7. [The Extension Model: A Ladder, Not a Plugin Zoo](#7-the-extension-model-a-ladder-not-a-plugin-zoo)
8. [Governance & Security](#8-governance--security)
9. [Verification, Testing, and Evals](#9-verification-testing-and-evals)
10. [Relationship to Existing Modules](#10-relationship-to-existing-modules)
11. [Competitive Analysis](#11-competitive-analysis)
12. [Phase Plan](#12-phase-plan)
13. [Open Questions](#13-open-questions)
14. [Version History](#14-version-history)

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

ntnt will build the agent harness **in** the language instead — as a
standard-library module whose core makes the agent a **first-class
participant in the program**. The bet is not "ntnt gets agent support." It
is that ntnt *already has*, for unrelated reasons, the exact machinery a
harness normally fakes:

| A harness needs… | ntnt already has… **(existing)** |
|---|---|
| Tool sandboxing / authorization | `RuntimeCapability` + capability-gated execution modes |
| Output verification & repair loops | IAL intent specs, `requires`/`ensures` contracts, the DD-063 repair loop |
| Orchestration (subagents, fan-out) | `std/concurrent` — `spawn`, `parallel`, `race`, channels |
| Durable long-running agents | `std/jobs` — enqueue, retries, workers, audit log (DD-042) |
| Token / event streaming | `std/sse` (DD-041) — broadcast bus, bounded queues, backpressure |
| Long-term memory | `std/kv`, `std/db` |
| Structured outputs | the type system, `std/validate` schemas, strict-mode boundary checks |
| Fast iteration on prompts/skills | hot reload |

Because these exist, the harness is **mostly composition** — and it can
offer guarantees a library harness structurally cannot: an agent call that
runs under a capability scope the runtime enforces, whose output is checked
by the same verification machinery as the rest of the program, composed
with the same concurrency and durability the rest of the program uses.

The field this design must survive is the fastest-moving one in software:
model APIs, tool protocols (MCP and its successors), context-management
practice, and prompt conventions all churn monthly. A standard library is a
stability commitment. §5 and §6 resolve that tension structurally rather
than by policy: the *unversioned* part of this design is one small native
substrate that the churn cannot reach, and everything the churn does reach
is versioned ntnt source.

### The two approaches, side by side

```
Traditional (harness WITH a language)        Proposed (harness IN the language)
────────────────────────────────────        ──────────────────────────────────
agent = opaque external service              agent = a callable value in the program
tool allowlist (hand-rolled)                 tool = capability-scoped function
output validator (bolted on)                 output gated by contracts/IAL + repair
graph engine (reinvented)                    orchestration = spawn/parallel/race
queue adapter (reinvented)                   durability = std/jobs
event bus (reinvented)                       streaming = std/sse
framework internals (opaque)                 harness = readable ntnt source
config = YAML outside the type system        config = typed, tested, hot-reloaded source
```

The differentiation is narrow but real: applications where agents and
deterministic code are **deeply interleaved and mutually governed** — the
agent invokes your capability-gated, contract-checked functions, and your
functions invoke agents whose outputs are verified — one program the
runtime can sandbox, hot-reload, test, and audit end to end.

---

## 2. The Design Model

Three ideas carry the whole design. Everything else in this document is a
consequence of them.

### 2.1 There is exactly one nondeterministic primitive

Strip any agent harness to its irreducible core and one thing remains that
cannot be built from an existing ntnt primitive: **the model call** — a
request goes out, a stream of events (text deltas, tool-call requests,
usage) comes back, and the mapping is nondeterministic.

This design gives that core a name — `infer(request) -> EventStream` — and
confines *all* nondeterminism and *all* provider churn behind it. Every
other part of the harness — the agentic loop, context assembly, tool
dispatch, verification, memory, sessions — is ordinary, deterministic ntnt
code that can be read, typed, tested, and hot-reloaded like any other code
in the application.

**Invariant 1: the harness contains exactly one nondeterministic function,
and that function is recordable.** (Recordability is what makes agent
testing deterministic — §9.)

### 2.2 An agent is a function whose body is a model

ntnt already has two kinds of callable: functions (body = code) and
closures (body = captured code). An **agent is the third: a callable whose
body is inference.** It has a typed input, a typed output, contracts, and
a capability scope — like any function. It differs only in its execution
substrate.

This single identification does most of the design work:

- **Agents are values.** `agent(…)` returns a callable value; you invoke it
  like any closure: `support(message)`. No `run()` verb is needed for the
  common case, no new syntax, no parser work.
- **Structured output is just the return schema.** An agent declared with
  `"returns": invoice_schema` (a `std/validate` schema — **existing**)
  produces a validated map or an `Err`, exactly like `validate()` at any
  other trust boundary. "Structured outputs" stops being a feature and
  becomes the type system doing its job at one more boundary.
- **Contracts stretch across the nondeterminism boundary.** `ensures` on a
  function is a runtime check; `ensures` on an agent is a runtime check
  *plus a bounded repair loop* (feed the violation back, retry, then `Err`).
  Same vocabulary, honestly weaker semantics — gated and repaired, never
  proven (§9).
- **Agents compose like functions** — which yields the second unification:

**An agent can be another agent's tool.** A tool is a callable the agent
may invoke; agents are callables; therefore `Tool ⊇ Agent`. Handoffs,
router agents, supervisor trees, specialist pools — every multi-agent
pattern collapses into "an agent whose tools include agents," orchestrated
by the concurrency the language already has. There is no separate
delegation concept to design, version, or teach. Capability grants narrow
automatically through the same nesting (§8).

### 2.3 Thin native substrate, harness as source

The implementation splits into two layers with opposite change rates:

- **The substrate** (native, tiny, **unversioned**): `infer` + provider
  adapter registration, capability-gate enforcement for tool dispatch,
  schema derivation from typed signatures, token accounting. Frozen because
  it is small enough to freeze.
- **The harness** (ntnt source, **versioned**): the agentic loop, context
  strategies, prompt construction, memory policy, session management,
  hooks, built-in skills. `std/harness/v1` is a module of readable ntnt
  code over the substrate; `std/harness/v2` is another.

This split is what makes a stdlib harness viable in a churning field. The
part that must stay stable is too small to be wrong for long; the part that
must evolve is cheap to evolve — a new harness version is *new ntnt source*,
not a runtime fork. It also means the harness is not magic: an app developer
can read exactly what their agent loop does, step through it, and — at the
top of the extension ladder (§7) — replace or fork it.

---

## 3. Design Principles

1. **One primitive, then composition.** Anything expressible as composition
   over `infer` + existing modules must not become a new primitive.
   Corollary: if a proposed feature can be written in userland ntnt in ten
   lines, ship it as documentation, not API.

2. **Agents are functions.** Callable values with types, contracts, and
   capability scopes. Every design question of the form "how do agents do
   X?" is first answered by "how do functions do X?" and only deviates with
   cause.

3. **Everything bounded.** Turns, tokens, cost, tool calls, delegation
   depth, context size: every resource an agent consumes has an explicit
   budget with an explicit overflow policy. This is the same design
   language as DD-041's bounded subscriber queues — an unbounded agent
   loop is the OOM of the agentic era, and "bounded with policy" is the
   house style.

4. **Verified, not guaranteed.** Contracts and IAL specs over
   nondeterministic output are runtime gates with bounded repair — never
   static proof, and never *presented* as proof. Preserving the trust
   semantics of `ensures` everywhere else in the language is more important
   than marketing symmetry.

5. **Governance at the effect, not the prompt.** The security boundary is
   the tool call, enforced by capability grant — not instructions to the
   model. We do not try to make the model trustworthy; we make it
   powerless outside its grant (§8).

6. **The churn stops at the adapter.** Providers (and future tool
   protocols like MCP) are adapters behind the substrate's one interface.
   New model, new provider API, new protocol → new adapter. A new
   *harness version* is reserved for changes in harness *design*.

7. **No new keywords, no new config format.** Agents, tools, and skills are
   ordinary declarations in ordinary modules: typed, testable
   (`ntnt test`), verifiable (`ntnt intent check`), hot-reloadable. The
   parser is untouched (the DD-041 lesson, applied from day one).

---

## 4. Primitives

Seven nouns. Two are unifications rather than new things (`Tool ⊇ Agent`;
`Session` = agent ⊗ persistent context), and one (`Provider`) lives below
the harness in the substrate. **All identifiers are proposed.**

| Noun | What it is | One-line definition |
|---|---|---|
| `Agent` | callable value | a function whose body is inference |
| `Tool` | callable + grant | any capability-scoped callable an agent may invoke — including another agent |
| `Skill` | module | instructions + tools + seed context, imported like any module |
| `Context` | bounded resource | the working set of a run: budgeted, with an overflow policy |
| `Memory` | store | durable recall across runs; `std/kv`/`std/db`-backed by default |
| `Session` | binding | an agent bound to a persistent context by key |
| `Provider` | substrate adapter | the one nondeterministic function, per backend |

### Declaring an agent

```ntnt
// PROPOSED
import { agent, tool } from "std/harness/v1"
import { schema, required } from "std/validate"

let reply_schema = schema(map {
    "answer":     [required],
    "escalate":   [required],
})

let support = agent(map {
    "provider": "anthropic",
    "model":    "claude-fable-5",
    "system":   "You are a support agent for an e-commerce store.",
    "tools":    [lookup_order, issue_refund],  // capability-scoped (§8)
    "returns":  reply_schema,                  // structured output = std/validate
    "ensures":  fn(r) { r["answer"] != "" },   // gate + bounded repair (§9)
    "budget":   map { "max_turns": 8, "max_depth": 2, "repair": 2 },
    "context":  map { "budget": 32000, "overflow": "summarize-oldest" },
    "memory":   "support",                     // std/kv namespace
})
```

### Calling an agent — it's a function

```ntnt
get("/support", fn(req) {
    let data = parse_json(req) otherwise { return status(400, "Bad JSON: #{err}") }
    // An agent is a callable value. The call runs the loop, applies the
    // returns-schema and ensures gate (with repair), and yields Ok/Err.
    let reply = support(data["message"])?
    return json(reply)
})
```

Streaming, durability, sessions, and fan-out are the same agent value
passed through existing subsystems — not new agent kinds:

```ntnt
import { sse } from "std/sse"
import { stream, session } from "std/harness/v1"
import { enqueue } from "std/jobs"
import { parallel } from "std/concurrent"

// STREAM — tokens/tool events as SSE (DD-041 bus underneath)
get("/support/stream", fn(req) {
    return sse(stream(support, req.params.message))
})

// SESSION — the agent bound to a persistent, named context. Named like
// DD-041's named buses: survives hot reload, shared across workers.
post("/chat/{user_id}", fn(req) {
    let chat = session(support, req.params.user_id)
    let reply = chat(parse_json(req)?["message"])?
    return json(reply)
})

// DURABLE — a long-running agent is just a job (retries + DD-042 audit)
enqueue("run_agent", map { "agent": "researcher", "input": topic })

// FAN-OUT — the language's concurrency IS the orchestration engine
let [plan, risks, cost] = parallel([
    fn() { planner(brief) },
    fn() { risk_analyst(brief) },
    fn() { estimator(brief) },
])
```

### Agents as tools — multi-agent for free

```ntnt
// A researcher that may only fetch; a writer that may only file reports —
// and may delegate to the researcher. No handoff API: Tool ⊇ Agent.
let researcher = agent(map {
    …,
    "capabilities": ["net.fetch"],
})

let writer = agent(map {
    …,
    "tools":        [researcher, file_report],
    "capabilities": ["net.fetch", "reports.write"],
    "budget":       map { "max_depth": 2, … },   // bounds delegation recursion
})
```

The delegated agent runs under the **intersection** of its own grant and
its caller's — grants only ever narrow through delegation (§8). `max_depth`
bounds recursion, because agents-that-call-agents is now ordinary calling
and must be budgeted like any other resource (Principle 3).

---

## 5. Architecture: Substrate and Harness

```
┌──────────────────────────────────────────────────────────────┐
│  Application (ntnt)                                          │
│    agents, tools, skills, hooks — ordinary app modules       │
├──────────────────────────────────────────────────────────────┤
│  Harness (ntnt source, VERSIONED: std/harness/v1, v2, …)     │
│    the agentic loop · context strategies · prompt assembly   │
│    sessions · memory policy · hooks · built-in skills        │
├──────────────────────────────────────────────────────────────┤
│  Substrate (native, UNVERSIONED, deliberately tiny)          │
│    infer(request) -> EventStream   · provider registration   │
│    capability-gate for tool dispatch                          │
│    schema derivation from typed signatures · token counting  │
├──────────────────────────────────────────────────────────────┤
│  Providers (adapters; independent release cadence)           │
│    anthropic · openai · local · replay (for tests, §9) · …   │
└──────────────────────────────────────────────────────────────┘
```

### The loop is readable source

The default agentic loop ships as ntnt source in the harness module —
abridged here to show the shape (every helper is also harness source):

```ntnt
// std/harness/v1 — default loop, ABRIDGED. Ordinary ntnt: readable,
// hot-reloadable, replaceable per agent (§7, rung 4).
fn default_loop(cfg, input, ctx) {
    let mut turns = 0
    while turns < cfg["budget"]["max_turns"] {
        let request  = assemble(cfg, ctx, input)      // context strategy (§7)
        let response = infer(cfg["provider"], request) // THE primitive (§2.1)

        match interpret(response) {
            ToolCall(call) => {
                // capability check happens HERE, in the substrate gate —
                // not in the prompt (§8)
                let result = invoke_tool(cfg, call)
                ctx = append(ctx, call, result)
            }
            Final(output) => {
                return gate_and_repair(cfg, output, ctx)  // §9
            }
        }
        turns = turns + 1
    }
    return Err("agent budget exhausted: max_turns=#{cfg["budget"]["max_turns"]}")
}
```

Three properties fall out of "the harness is source":

1. **Nothing is magic.** The exact behaviour of your agent — what enters
   context, when compaction fires, how repair re-prompts — is code you can
   open, not a framework internal you reverse-engineer from docs.
2. **Versioning is cheap** (§6). A new harness generation is a new source
   module sharing the frozen substrate — not a compiler release, not a
   runtime fork.
3. **The substrate is a platform.** `std/harness` is the *blessed* harness,
   not the only possible one. A team (or the community) can write a
   different harness against the same substrate and get the same
   governance guarantees, because enforcement lives below the source line.

### What must be native, and why

| Substrate piece | Why it cannot be harness source |
|---|---|
| `infer` + provider adapters | network + streaming + secrets; the nondeterminism boundary itself |
| capability gate on tool dispatch | security enforcement must sit below replaceable code — a harness (or a prompt-injected agent) must not be able to skip it |
| schema derivation | needs the typechecker's view of signatures/contracts |
| token accounting | needs provider-accurate tokenizers; feeds budget enforcement |

Everything not on this list defaults to harness source. When in doubt,
source.

---

## 6. Versioning: Harness Editions

The versioning model is the module path — and the substrate/harness split
is what makes it honest:

```ntnt
import { agent } from "std/harness/v1"   // this app pins v1
import { agent } from "std/harness/v2"   // a later app pins v2
```

Think **editions** (in the Rust sense), not semver: coexisting generations
of the same feature, selected per import, sharing one runtime substrate.

### 6.1 What a version owns

The **vocabulary is held stable across versions** so call sites survive a
migration; each version owns **behaviour**:

| Stable vocabulary (all versions) | Versioned behaviour (per edition) |
|---|---|
| The seven nouns (§4) and their meaning | default context strategy & budget defaults |
| Call shapes: `agent(cfg)`, direct call, `stream()`, `session()` | prompt construction & system conventions |
| Extension interfaces: provider, context strategy, memory store, hook points | default tool-protocol dialect (e.g. v2 = MCP-native) |
| Capability semantics: grants, intersection-narrowing, budgets | memory policy defaults |
| The gate-and-repair policy *shape* | bundled skills and their prompts |

The ideal `v1 → v2` migration: change the import line, run
`ntnt intent check`, adjust config only where a default genuinely changed.
`support(message)` does not move.

### 6.2 Shipped versions are immutable

Once `std/harness/v1` ships, it receives **bug fixes only** — never changed
defaults, never revised prompts. Behaviour someone's app depends on is
behaviour someone's app depends on; "improvements" go in `v2`. This is the
strongest promise a stdlib can make in a churning field, and it is only
affordable because a version is a source module, not a runtime (§2.3).

### 6.3 Coexistence and incremental migration

Versions are distinct namespaces (DD-047 module resolution — **existing**),
so an app imports both and migrates one agent at a time:

```ntnt
import { agent as agent_v1 } from "std/harness/v1"
import { agent as agent_v2 } from "std/harness/v2"

let legacy_support = agent_v1(map { … })   // untouched
let new_research   = agent_v2(map { … })   // on the new approach
```

### 6.4 Providers version independently — the churn sink

New model, new provider API, protocol revision → **new adapter**, on its
own cadence, usable from every harness version. A new harness edition is
reserved for a change in *design*: a different context model, a different
default tool protocol, a different loop. The roster of models is never a
reason to cut `v2`.

### 6.5 Policy

- Within an edition: additive only (new optional config keys, new hooks).
- Across editions: breaking changes allowed, each with a migration note.
- `ntnt lint` warns on imports of an edition past its published
  support window (window length: open question, §13).

---

## 7. The Extension Model: A Ladder, Not a Plugin Zoo

Extensibility is designed as **progressive disclosure of power**: five
rungs, each strictly more capable and strictly more responsibility, each a
stable seam. Most apps never leave rung 1.

**Rung 1 — Configure.** The `agent(map { … })` surface: model, system,
tools, skills, budgets, context policy, memory namespace, gates. Typed,
validated config — no code.

**Rung 2 — Hook.** Cross-cutting interception at fixed points, global or
per-agent — the middleware pattern ntnt already uses for HTTP:

```ntnt
// PROPOSED hook points (observe / mutate / short-circuit):
use_agent_hook("before_request", fn(c) { … })  // redact, inject, cache-check
use_agent_hook("on_tool_call",   fn(c) { … })  // authorize, rate-limit, log
use_agent_hook("on_token",       fn(t) { … })  // stream side-channel
use_agent_hook("after_response", fn(r) { … })  // validate, cache-store
use_agent_hook("on_error",       fn(e) { … })  // fallback, escalate
```

**Rung 3 — Swap a strategy.** Named implementations behind the three
behavioural interfaces, registered and then referenced from config:

```ntnt
// Context strategy: how the working set is assembled and compacted.
register_context_strategy("retrieval", map {
    "assemble": fn(history, budget, memory) { … },
    "compact":  fn(ctx, budget) { … },
})

// Memory store: durable recall (default: std/kv; here: a vector DB).
register_memory_store("pgvector", map {
    "write": fn(ns, record) { … },
    "query": fn(ns, query, k) { … },
    "prune": fn(ns, policy) { … },
})

// Provider: the substrate adapter (native providers arrive via DD-062).
register_provider("my-llm", fn(request) { … })
```

**Rung 4 — Replace the loop.** An agent config may name its loop function
(`"loop": my_loop`). The default loop is harness source (§5); a custom loop
is app source with the same signature. Governance does not weaken: the
capability gate and budgets are enforced in the substrate, *below* any
loop.

**Rung 5 — Fork the harness.** Because an edition is ntnt source, a team
can vendor `std/harness/v1` into their app, modify it, and still run on
the same substrate with the same enforcement. The escape hatch is total,
and it still cannot escape governance.

**Skills** cut across the ladder: a skill is a module exporting
instructions + tools + seed context, mounted at rung 1 —

```ntnt
import { refund_handling } from "skills/refunds"
let support = agent(map { …, "skills": [refund_handling] })
```

— and because skills are modules, they inherit the entire toolchain:
imports and versioning (DD-047), type checking, `ntnt test`,
`ntnt intent check`, hot reload. A skill is to an agent what an import is
to a function.

---

## 8. Governance & Security

Agents are a governance problem before they are an intelligence problem,
and governance is where a language-level harness earns its keep. The stance
in one line: **we don't try to make the model trustworthy; we make it
powerless outside its grant.**

- **Tool authorization = capability scoping.** An agent is declared with a
  capability grant. Every tool declares the capabilities it requires
  (derived from the modules it touches where possible, declared where not).
  The substrate refuses any tool dispatch whose requirements exceed the
  live grant — the same mechanism family as the **existing**
  `RuntimeCapability` gate that silently no-ops `listen()` in a Worker,
  with two deliberate differences: the namespace is **open** (app-defined
  capability names like `"payments.refund"`, not only the built-in enum —
  new substrate work), and violations are **loud** (`Err`, logged), never
  silent — an agent must not "succeed" by having its effect skipped.

- **Prompt injection meets a hard floor.** Everything a model emits and
  ingests is untrusted. The boundary that matters is the *effect*, and the
  effect boundary is the capability check — enforced below harness source
  (§5), so neither a hostile prompt nor a buggy custom loop can lift it. A
  fully compromised agent can at worst exercise exactly the grant you
  wrote next to its definition, which is a line of code a human reviews.

- **Grants only narrow.** A delegated agent (an agent used as a tool, §4)
  runs under `own_grant ∩ caller_grant`. Skills cannot widen the mounting
  agent's grant. There is no escalation path by construction.

- **Everything bounded** (Principle 3). `max_turns`, `max_depth`,
  `max_tool_calls`, token and cost ceilings, context budgets with overflow
  policy. Exhaustion is a loud `Err` carrying what was exhausted. The
  runaway agent is not a monitoring problem; it is a type of bug the
  runtime refuses to run.

- **Secrets stay below the source line.** Provider keys live in adapters
  (loaded via `std/env`, held natively — DD-062 fits here), never in
  context windows, never in tool arguments. A tool receives only what its
  typed signature declares.

- **Audit is inherited, not added.** Durable runs get the DD-042 job audit
  log; every tool call flows through `on_tool_call` (rung 2) for policy
  and logging; sessions and memory are inspectable state in `std/kv`.

---

## 9. Verification, Testing, and Evals

### The gate-and-repair contract

Verification is the existing contract system stretched across the
nondeterminism boundary, with semantics stated honestly:

| On a function **(existing)** | On an agent **(proposed)** |
|---|---|
| `requires` checked at call | config/schema validated at call |
| `ensures` checked at return; violation = error | `returns` schema + `ensures` predicates checked at return; violation → **bounded repair** (violation fed back, ≤ `repair` retries) → then `Err` |
| IAL spec via `@implements`, checked by `ntnt intent check` | IAL spec bindable to an agent's observable behaviour, same tooling |

The gate **never proves**; it gates and repairs. Documentation and tooling
must keep that distinction sharp (Principle 4) — the trust users place in
`ensures` elsewhere in the language is not spendable here.

Where stronger guarantees are needed, compose them the ordinary way — a
deterministic wrapper owns the real contract:

```ntnt
// The wrapper's contract is a REAL contract; the agent inside is gated.
fn triage(ticket: String) -> String
    ensures result != ""
{
    let reply = triage_agent(ticket)?
    return reply["answer"]
}
```

### Deterministic tests — the replay provider

Because all nondeterminism passes through `infer` (Invariant 1), recording
at that seam makes every agent test deterministic:

```ntnt
// PROPOSED: the replay provider is just another provider.
// Record once against the live model; replay forever in ntnt test.
let support_test = agent(map { …, "provider": "replay:fixtures/support" })
```

Consequences worth stating:

- **Agent tests are ordinary `ntnt test` functions** — agents are callable
  values, so they are called, asserted on, and CI'd like everything else.
- **Evals are tests with a scoring assertion** — a fixture set, a replay or
  live provider, and a threshold. No separate eval framework; `ntnt test`
  with a budget.
- **Hot reload closes the iteration loop** — edit a prompt, a skill, or a
  strategy; the next request runs it. The agent development loop becomes
  the ntnt development loop.

---

## 10. Relationship to Existing Modules

The harness is a composition layer. Each capability maps onto a module
that exists or is in design — the strongest feasibility argument there is:

| Harness capability | Backed by | Status |
|---|---|---|
| Subagents, fan-out, races | `std/concurrent` (`spawn`, `parallel`, `race`) | **existing** (DD-037/049) |
| Durable / background agents | `std/jobs` (enqueue, retries, workers, audit) | **existing** (DD-037/042) |
| Token & event streaming | `std/sse` (bus, bounded queues, backpressure) | **in design** (DD-041) |
| Memory / recall | `std/kv`, `std/db` | **existing** |
| Structured output | `std/validate` schemas | **existing** (DD-063 era) |
| Tool sandboxing | capability gate (open-namespace extension of `RuntimeCapability`) | **existing + new substrate work** |
| Output verification | IAL + `requires`/`ensures` + DD-063 repair pattern | **existing** |
| Native provider adapters | DD-062 secure compiled extension libraries | **in design** |
| Prompt/skill iteration | hot reload | **existing** |

---

## 11. Competitive Analysis

| Capability | `std/harness` (proposed) | LangGraph | CrewAI | OpenAI Agents SDK | Vercel AI SDK |
|---|---|---|---|---|---|
| Agent representation | callable value in the language | graph node | crew member | SDK object | SDK call |
| Tool sandboxing | runtime capability gate, below user code | app allowlist | app-level | app-level | app-level |
| Multi-agent | `Tool ⊇ Agent` + language concurrency | graph engine | crew abstraction | handoffs API | manual |
| Output verification | contracts/IAL + bounded repair, native | validators | custom | structured outputs | structured outputs |
| Deterministic testing | replay provider at the one seam | bolt-on mocks | bolt-on | bolt-on | bolt-on |
| Durability | `std/jobs`, native | external | external | external | external |
| Streaming | `std/sse`, native | callbacks | manual | streaming API | strong |
| Harness internals | readable, forkable ntnt source | framework internals | framework internals | SDK internals | SDK internals |
| Versioning | coexisting editions, per-import | semver package | semver | semver | semver |
| Ecosystem breadth | narrow at launch | broad | broad | OpenAI-centric | broad |

Honest read: libraries win on day-one breadth — every integration is a
package away, and providers are many. This design wins on **cohesion,
governance, and permanence**: enforcement below user code, verification
and testing that are the language's own, internals that are source, and an
edition model no package manager offers. The narrow-breadth risk is
mitigated structurally: one small provider interface is the entire
integration surface, and DD-062 gives native adapters a delivery path.

---

## 12. Phase Plan

**Phase 0 — Substrate spike.** The thinnest slice that tests the whole
thesis: `infer` with one hard-coded provider; `agent()` returning a
callable value; tool dispatch through the capability gate with one
app-defined capability name; `returns`-schema gate with **one** repair
retry; `max_turns` budget. ~A week of work, and it answers the only
question that matters: does "agent = function under governance" *feel*
native in real code, the way `sse()`-as-response-builder does in DD-041?

**Phase 1 — Substrate + `std/harness/v1` core.** Freeze the substrate
surface (`infer`, provider registration, capability gate, schema
derivation, token accounting). Ship v1 as harness source: default loop,
one context strategy (`sliding-window` + summarize-overflow), sessions,
memory over `std/kv`, hooks (rung 2), Anthropic adapter + **replay
provider**, `stream()` over `std/sse`. Editions mechanics land here
(module-path resolution, immutability policy).

**Phase 2 — Composition surface.** Skills as modules; agents-as-tools with
grant intersection + `max_depth`; durable agents over `std/jobs`;
strategy registration (rung 3); OpenAI + local adapters.

**Phase 3 — Verification depth.** IAL binding for agent behaviour; richer
repair strategies; retrieval context strategy over memory stores;
eval-style scoring in `ntnt test`; intent-studio supervision of agent runs
(DD-060).

`std/harness/v2` is cut only when the *design* changes — a new default
tool protocol (e.g. MCP-native), a new context model, a new loop — per
§6.4.

---

## 13. Open Questions

1. **The provider event vocabulary.** The normalized `infer` event set
   (text delta, tool-call, usage, done, error) — how small can it stay
   while covering thinking/multimodal streams, and what passes through as
   provider-opaque extensions?
2. **App-defined capability names.** Namespacing and declaration point —
   in code at the tool site, or declared in the app's `.intent` file so
   the grant surface is part of the intent spec a human approves? (The
   latter is philosophically attractive: IDD for governance.)
3. **Agent callability mechanics.** Builder returns a native closure
   (zero interpreter change) vs a distinct `Value::Agent` (introspectable,
   better errors, dot-property reads). Leaning closure for Phase 0,
   decide by Phase 1.
4. **IAL over behaviour, not just output.** Binding a spec to *the run* —
   "must call `lookup_order` before `issue_refund`" — is a temporal
   assertion over the tool-call trace. Powerful; how much of IAL as it
   exists covers it?
5. **Streaming ⊗ structured output.** When a `returns` schema is set, what
   streams — raw deltas plus a final validated event? How does repair
   interact with an already-streamed partial answer?
6. **Memory portability across editions.** A version-neutral store
   contract vs per-edition migration of memory namespaces.
7. **Cost accounting.** Where do token/cost budgets meet `std/jobs` (a job
   with a spend ceiling) and the audit log — per run, per session, per
   app?
8. **Support-window policy.** How long does an edition receive bug fixes
   after its successor ships, and what does `ntnt lint` say at each stage?

---

## 14. Version History

| Date | Change |
|------|--------|
| 2026-07-08 | Initial draft — vision, principles, primitives, module-path versioning, extension seams, security model, phase plan |
| 2026-07-08 | Design pass — reduced to the one-primitive model (`infer`); agent-as-function (callable values, contracts across the boundary, structured output = `std/validate` schema); `Tool ⊇ Agent` collapses multi-agent; substrate/harness split (thin unversioned native core, harness as versioned ntnt source); editions framing with immutable shipped versions; extension ladder (configure → hook → swap → replace loop → fork); everything-bounded budgets; replay provider for deterministic tests/evals |
