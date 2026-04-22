# DD-061: Interpreter Performance Roadmap for ntnt

## Status: in_review

## Problem
ntnt has already captured major server/runtime wins by moving from a single request interpreter to a worker-pool architecture and by adding async PostgreSQL pooling. Those changes delivered large benchmark gains, especially for database-heavy workloads, but they did **not** fundamentally optimize the interpreter itself.

As long as ntnt remains a tree-walking Rust interpreter, performance is still constrained by interpreter-core costs such as:
- `Value` representation overhead
- environment / scope lookup churn
- repeated string-based name lookup
- generic property / method dispatch
- repeated dynamic branching in hot AST evaluation paths
- repeated per-render/per-call work that could be cached or specialized

This means that, after the server/runtime improvements from v0.4.2, the next ceiling is increasingly the interpreter core rather than the web server shell around it.

The open question is not whether ntnt should become a JIT or bytecode VM immediately. The nearer-term question is which **interpreter-native** techniques are high leverage for ntnt’s current architecture, and how to adopt them without losing the language’s clarity, debuggability, and correctness posture.

## Solution
Create a staged performance roadmap for the existing tree-walking ntnt interpreter that focuses on high-ROI architectural and runtime improvements **before** considering bytecode or JIT work.

The roadmap should prioritize techniques that:
- fit a dynamic tree-walking interpreter
- preserve ntnt’s debuggability and implementation clarity
- improve real workloads, not just synthetic benchmarks
- compose safely with ntnt’s worker-pool server model
- avoid premature complexity where simpler structural fixes will yield most of the gain

The core proposal is to split performance work into four layers:

1. **Hot-path simplification**
2. **Lookup and dispatch optimization**
3. **Runtime representation optimization**
4. **Execution model evolution**

This design doc proposes specific work in each layer and recommends an implementation order.

## Design

### Guiding Principles

#### 1. Optimize the interpreter we actually have
ntnt today is a tree-walking Rust interpreter with a compiled Rust runtime shell. The most valuable near-term work is the work that makes **that architecture** faster.

That means prioritizing:
- cheaper lookups
- cheaper values
- cheaper dispatch
- less repeated work

before jumping to:
- bytecode
- JIT
- speculative compilation

#### 2. Preserve debuggability
ntnt’s current implementation has a major advantage: it is comparatively easy to reason about. Performance work should not casually destroy this.

Each phase should therefore prefer:
- explicit data structures over magic
- localized specialization over invisible global behavior
- invalidation rules that are explainable
- performance wins that do not make correctness opaque

#### 3. Measure against real ntnt workloads
Performance work must be evaluated against workloads that reflect how ntnt is actually used:
- plaintext / JSON endpoints
- route dispatch
- auth/session flows
- template rendering
- single-query DB handlers
- multi-query DB handlers
- blog/page render workloads

Microbenchmarks are useful, but only as supplements.

#### 4. Avoid pretending all wins are equal
Some techniques are highly relevant to ntnt now; others are intellectually interesting but premature.

**Highly relevant now:**
- environment lookup optimization
- `Value` representation / allocation reduction
- field / method inline caches
- object / map model improvements
- AST-path specialization
- template caching

**Possibly later:**
- bytecode interpreter
- register VM
- JIT
- aggressive speculative optimization

---

### What We Already Did

Before this roadmap, ntnt already shipped significant server/runtime performance work:

#### Worker pool
- one interpreter per worker thread
- requests distributed over an MPMC queue
- removes the single-request funnel for HTTP workloads

#### Async PostgreSQL connection pooling
- moved from synchronous single-client posture to pooled async DB access
- allows concurrent DB operations across workers

#### Worker-mode runtime simplification
- workers skip hot-reload/dev-only overhead
- reduces per-request non-essential work in serving mode

These changes produced strong gains, especially on DB-heavy benchmarks. They should be understood as **runtime-shell performance improvements**, not interpreter-core performance work.

This roadmap begins where those gains stop.

---

### Performance Layers

## Layer 1: Hot-Path Simplification

These are the lowest-risk, highest-clarity improvements. They keep the current architecture intact while reducing obvious waste.

### 1.1 Reduce repeated string-driven dispatch in hot interpreter paths
Current dynamic language interpreters often pay too much for repeated string lookup in places like:
- variable access
- object field access
- method dispatch
- builtin/operator dispatch

#### Proposal
Audit hot AST evaluation paths and replace repeated string-driven or generic dispatch with more direct specialized paths where the parser/runtime already knows enough structure.

Examples:
- direct operator fast paths instead of generic name-based dispatch where possible
- specialized evaluation paths for common AST node categories
- reduced repeated normalization / map probing in common route/template/auth code paths

#### Why this matters
Tree-walking interpreters frequently lose large amounts of time in generic “do everything” code paths rather than in the AST walk itself.

#### Implementation Checklist
- [ ] Identify top interpreter hot paths using perf/flamegraph or benchmark-guided sampling
- [ ] Inventory string-driven dispatch in arithmetic, comparisons, property access, and calls
- [ ] Replace obviously static operator paths with direct evaluation helpers where safe
- [ ] Reduce redundant normalization / repeated path-building in HTTP/auth/template hot paths
- [ ] Add microbenchmarks for each changed hot path

---

### 1.2 Cache compiled / prepared template state
The benchmark post explicitly called out template re-parse overhead as remaining headroom.

#### Proposal
Introduce template compilation/parsing caches with safe invalidation in development and stable reuse in worker/prod mode.

#### Why this matters
Template rendering is exactly the sort of repeated structured work that should not be re-done on every request if the source has not changed.

#### Implementation Checklist
- [ ] Measure current template parse vs render cost separately
- [ ] Add compiled template cache keyed by canonical path + invalidation metadata
- [ ] Support dev invalidation via mtime / watched change detection
- [ ] Support worker/prod stable reuse without per-request stat churn
- [ ] Benchmark template-heavy routes before/after

---

### 1.3 Reduce allocation / cloning churn in common request paths
#### Proposal
Audit repeated `String`, `Vec`, `HashMap`, and `Value` clones in request handling, template rendering, and route dispatch.

#### Why this matters
Interpreters often bleed performance through allocation churn even when algorithms are fine.

#### Implementation Checklist
- [ ] Profile allocation-heavy request paths
- [ ] Reduce avoidable cloning in route dispatch and request/response helpers
- [ ] Reuse buffers or interned/static strings where appropriate
- [ ] Add regression benches focused on allocation-sensitive endpoints

---

## Layer 2: Lookup and Dispatch Optimization

This is likely the biggest ROI zone for ntnt’s actual interpreter.

### 2.1 Environment / scope lookup optimization
The current interpreter model likely pays repeated cost for:
- lexical scope chain walking
- repeated map lookups by string name
- nested closure / environment traversal

#### Proposal
Add lookup specialization for variable resolution.

Possible strategies:
- symbol IDs instead of repeated raw string comparison
- resolved lexical slots captured during parse/bind/typecheck phases
- per-node cached lookup metadata pointing to environment depth + slot index

#### Why this matters
For a tree-walking interpreter, variable lookup is often one of the most frequent operations in the system.

#### Design direction
Prefer a staged approach:
1. symbolization / name interning
2. binder-produced metadata for resolved locals/upvalues
3. slot-based lookup where environment layout is stable enough

#### Implementation Checklist
- [ ] Measure variable/scope lookup share in representative benchmarks
- [ ] Add symbol / interned-name abstraction for identifiers
- [ ] Add a binding/resolution pass that records scope depth and binding identity per node
- [ ] Introduce slot-based or index-based access for stable local frames
- [ ] Fall back safely for dynamic/global/module cases
- [ ] Benchmark closure-heavy and local-variable-heavy workloads

---

### 2.2 Property / method inline caches
#### Proposal
Introduce inline caches for repeated property and method lookup.

Candidate targets:
- object field access
- map/object property reads
- method resolution on stable receiver shapes/types

#### Why this matters
If the same AST node repeatedly accesses the same logical property on similarly shaped values, a node-local cache can collapse repeated hash/string lookup into a fast check.

#### Design direction
Start with monomorphic inline caches:
- cache receiver kind/shape + resolved slot/index/lookup result
- on mismatch, fall back to slow path and refresh cache

Only add polymorphic caches if monomorphic results prove insufficient.

#### Invalidation
If shape/layout can change, add invalidation/watchpoint-style versioning rather than complex mutation hooks everywhere.

#### Implementation Checklist
- [ ] Define a shape/version model for cachable object/map lookups
- [ ] Add monomorphic inline cache to field-get path
- [ ] Add equivalent cache to method lookup path
- [ ] Add invalidation/version bump rules for mutating operations
- [ ] Benchmark repeated field/method access workloads

---

### 2.3 Truthful route / helper dispatch specialization
#### Proposal
Where route resolution or helper lookup repeatedly traverses generic maps/registries, add cached fast-path lookup keyed by normalized route/helper identity.

#### Why this matters
Web-heavy ntnt apps likely spend time in repeated structural dispatch that could be specialized safely.

#### Implementation Checklist
- [ ] Measure route dispatch and helper lookup overhead in HTTP-heavy benchmarks
- [ ] Cache route matcher results where safe
- [ ] Cache helper/builtin resolution where semantics are stable
- [ ] Ensure dev-mode invalidation remains correct

---

## Layer 3: Runtime Representation Optimization

### 3.1 Revisit `Value` representation
#### Proposal
Review the current `Value` representation for:
- size
- copy cost
- boxing frequency
- integer/float fast paths
- string/object indirection cost

#### Why this matters
Every interpreter operation touches `Value`. Small inefficiencies here multiply everywhere.

#### Possible directions
- reduce enum size / indirection costs
- keep cheap scalar cases fast
- avoid unnecessary heap allocation for common small values
- tighten common-case operator fast paths

This does **not** imply rewriting the whole runtime around clever tagging immediately. It means auditing whether the current representation is leaving obvious wins on the table.

#### Implementation Checklist
- [ ] Measure `Value` size and copy behavior in hot paths
- [ ] Inventory boxing/allocation patterns for common scalar operations
- [ ] Prototype low-risk representation improvements
- [ ] Verify impact on arithmetic, JSON, template, and dispatch benchmarks

---

### 3.2 Object / map model specialization
#### Proposal
Move toward a more explicit object model for frequently used structured values.

Potential ingredients:
- stable shapes / hidden-class-like metadata
- slot-based storage for stable object layouts
- faster property lookup than generic hash maps in the common case

#### Why this matters
Dynamic-language object access often dominates runtime costs. A better object model is usually prerequisite for effective inline caches.

#### Implementation Checklist
- [ ] Classify current object/map-heavy runtime paths
- [ ] Design a shape/version abstraction suitable for ntnt objects/maps/struct-like values
- [ ] Prototype slot-backed fast lookup for stable shapes
- [ ] Integrate with inline cache invalidation strategy
- [ ] Benchmark object/property-heavy workloads

---

## Layer 4: Execution Model Evolution

These are meaningful, but should come after Layers 1–3 unless measurements prove otherwise.

### 4.1 Bytecode / lowered IR exploration
#### Proposal
Investigate lowering AST to a compact intermediate representation or bytecode while preserving current semantics and diagnostics.

#### Why this matters
Tree walking has inherent dispatch overhead. Bytecode can reduce interpreter dispatch cost dramatically.

#### Why this is not first
ntnt still appears to have substantial wins available from simpler structural fixes. Moving to bytecode too early risks complexity before we understand the actual hot costs.

#### Implementation Checklist
- [ ] Document current AST-walk dispatch overhead with profiling evidence
- [ ] Sketch a minimal lowered IR / bytecode for a subset of the language
- [ ] Compare implementation/debug complexity vs expected wins
- [ ] Decide whether to pursue after Layers 1–3 data

---

### 4.2 Parallel query execution / higher-level DB batching
#### Proposal
Add first-class support for batched or parallel query execution in handlers where it preserves semantics.

#### Why this matters
The benchmark post explicitly identified the sequential-query bottleneck in multi-query workloads.

This sits partly above the interpreter layer, but is still a major real-world performance lever.

#### Implementation Checklist
- [ ] Design batched/parallel query API surface
- [ ] Define ordering/error semantics clearly
- [ ] Benchmark 20-query and template-style workloads

---

## Relevance Matrix

| Technique | Relevance to ntnt now | Why |
|---|---:|---|
| Worker pool | Already done | Runtime-shell improvement, not interpreter-core |
| Async DB pooling | Already done | Major DB concurrency win already shipped |
| Template caching | High | Called out directly by current benchmark headroom |
| Scope/env lookup optimization | Very high | Core tree-walking interpreter cost |
| Inline caching | Very high | Fits dynamic tree-walker, large lookup/dispatch ROI |
| Object model specialization | High | Enables faster property/method lookup |
| Value representation tuning | High | Touches every operation |
| General hot-path cleanup | Very high | Low risk, immediate wins |
| Bytecode VM | Medium later | Powerful, but likely premature today |
| JIT | Low now | Too much complexity for current stage |
| GC innovation | Low now | Not current dominant bottleneck |

---

## Proposed Implementation Order

### Phase 1: Interpreter-core performance foundation
- [ ] Add profiling for representative ntnt benchmark workloads (plaintext, JSON, route params, DB single, DB multi, template) and capture where interpreter time is actually going
- [ ] Audit and reduce obvious hot-path waste: repeated string dispatch, repeated normalization, unnecessary cloning/allocation, and other generic interpreter overhead that shows up in profiles
- [ ] Add template parse/compile caching with correct development invalidation and stable worker/prod reuse
- [ ] Design identifier symbolization plus binder/resolution metadata for locals/upvalues so repeated scope lookup can move toward depth/slot-based access
- [ ] Prototype monomorphic inline caches for the highest-value repeated lookup path (field access or method resolution), including explicit invalidation/versioning rules
- [ ] Evaluate current `Value` representation and identify low-risk changes that reduce copy/allocation overhead in the hottest interpreter paths
- [ ] Re-run benchmarks after each sub-step and document which wins came from runtime-shell work vs interpreter-core work

## Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Premature complexity outruns measured value | Medium | Gate each phase with profiling + benchmark deltas |
| Caching introduces stale/incorrect behavior | High | Start monomorphic, explicit invalidation/versioning, add targeted tests |
| Performance work harms debuggability | Medium | Prefer localized specialization and explicit structures over opaque magic |
| Object-model changes ripple widely through stdlib/runtime | Medium | Stage behind compatibility layers and benchmark each step |
| Bytecode temptation derails simpler wins | High | Treat bytecode as Phase 5 decision, not default path |

## Alternatives Considered

### Jump straight to bytecode
Rejected for now because simpler interpreter-native wins are likely still abundant.

### Jump straight to JIT
Rejected as far too complex relative to ntnt’s current stage and bottlenecks.

### Keep focusing only on server/runtime shell
Rejected because the benchmark post suggests the next ceiling increasingly sits in interpreter behavior and repeated work, not only in the outer HTTP shell.

## Definition of Done
- [ ] Profiling exists for representative ntnt benchmark workloads
- [ ] A prioritized roadmap with staged interpreter-core work is accepted
- [ ] Phase 1 implementation work is split into actionable follow-up tasks or DDs
- [ ] Benchmark methodology for future interpreter-core changes is documented
- [ ] This design doc status is updated from `draft` to the appropriate next state
