# IAL Vision V2: Beyond Unit Testing

**Status:** Planning
**Date:** January 2026
**Prerequisites:** V1 Complete (Phases 0-5)

---

## Executive Summary

V1 delivered the foundation: glossary-based term rewriting, function calls, invariants, corpus testing, property checks, and verbosity levels. This document captures the remaining vision from our design conversations — the features that transform NTNT from "a working IDD system" into "a governance layer for autonomous development."

---

## Part I: The Remaining Vision

### What V1 Achieved

- Humans write natural language scenarios
- Glossary compiles terms to technical checks
- System runs hundreds of assertions invisibly
- Progressive disclosure via `-v` / `-vv`
- Coverage tracking and traceability

### What V2 Must Achieve

1. **Intent Studio Enhancement** - Expandable obligation views, not just pass/fail
2. **Formal Obligation Model** - Named obligations as first-class entities
3. **Agent Protocol** - Structured API for intent manipulation
4. **Advanced System Vocabulary** - Round-trips, conservation, authorization
5. **Structural Enforcement** - System refuses ambiguity, not just warns

---

## Part II: Intent Studio Enhancement

### Current State

Intent Studio shows live test results with checkmarks. It's a test dashboard.

### Target State

Intent Studio becomes an **obligation observability layer**:

- **Feature-level summary** - "Homepage: 3 scenarios, 47 checks ✓"
- **Expandable scenarios** - Click to see When/Then structure
- **Expandable assertions** - Click to see glossary term resolution chain
- **Failure drill-down** - Click to see counterexamples and expected vs actual
- **Coverage visualization** - Which code satisfies which intent

### UI Principles

1. **Progressive disclosure** - Start collapsed, expand on demand
2. **Green bar ≠ tests passed** - Green bar = obligations satisfied
3. **No mandatory reading** - Human confirms intent in 5 seconds, drills down only when curious

### Proposed Features

- [ ] Collapsible feature cards showing pass/fail summary
- [ ] Scenario expansion showing When/Then with resolution traces
- [ ] Assertion expansion showing term → primitive chain
- [ ] Failure cards with counterexamples highlighted
- [ ] Coverage heat map linking code lines to obligations
- [ ] Search/filter by feature, scenario, or assertion text
- [ ] Real-time updates as code changes (existing hot-reload)

---

## Part III: Formal Obligation Model

### Current State

Features contain Scenarios contain Assertions. But "obligation" is implicit.

### Target State

Obligations are first-class entities:

```
Obligation {
  id: "obligation.homepage.loads"
  name: "Homepage loads successfully"
  source: Feature("feature.homepage") → Scenario("visitor discovers site")
  status: Proven | Unproven | Partial | Stale
  evidence: [TestRun, PropertyCheck, RuntimeContract]
  linked_code: ["server.tnt:42", "server.tnt:58"]
}
```

### Why This Matters

- **Countable**: "7 obligations declared, 6 proven, 1 unimplemented"
- **Traceable**: Each obligation links to code and evidence
- **Staleness detection**: Code changed → obligation evidence is stale
- **Agent-queryable**: Agents can ask "what obligations exist?"

### Implementation Notes

- Obligations derived from Feature → Scenario → Assertion structure
- Each glossary-expanded assertion bundle = one obligation
- Invariant references create obligation dependencies
- Store in `.intent.lock` or similar artifact

---

## Part IV: Agent Protocol

### The Problem

Agents currently interact with intent via text editing. This leads to:
- Format drift
- Inconsistent structure
- No way to query "what obligations exist?"
- No atomic operations (add term, refine bundle)

### The Solution

A formal protocol for intent manipulation:

```
Intent Protocol v1

Queries:
  list_features() → [Feature]
  list_obligations(feature_id?) → [Obligation]
  get_coverage() → CoverageReport
  explain_term(term) → ExpansionChain
  show_failures() → [FailureDetail]

Mutations:
  add_glossary_term(pattern, means) → Result
  add_scenario(feature_id, scenario) → Result
  refine_obligation(obligation_id, refinement) → Result
  mark_assumption(obligation_id, reason) → Result
  deprecate_obligation(obligation_id, reason) → Result

Validation:
  validate_intent() → [Error | Warning]
  check_completeness() → CompletenessReport
```

### Protocol Principles

1. **Structured operations** - Not text surgery
2. **Validation on mutation** - Reject ambiguous changes
3. **Audit trail** - Every change has reason and timestamp
4. **Same API for Studio and agents** - One source of truth

### Implementation Options

- JSON-RPC over stdio (like LSP)
- HTTP API from `ntnt intent serve`
- MCP tool definitions for Claude

---

## Part V: Advanced System Vocabulary

### Currently Implemented

- Equality: `is`, `equals`, `result is`
- Containment: `contains`, `matches`, `starts with`, `ends with`
- Stability: `is deterministic`, `is idempotent`
- Bounds: `is at least`, `is at most`
- HTTP: status, body, headers

### V2 Additions

#### Errors and Rejection

| Term | Compiles To | Example |
|------|-------------|---------|
| `fails with {error}` | Check function raises error type | "rejects bad input" |
| `fails with message containing {text}` | Check error message | "explains what's wrong" |
| `rejects {input}` | Check function returns error/None | "refuses invalid data" |
| `never crashes` | Invariant: no panic on any corpus input | "handles errors gracefully" |

#### Round-trips and Inverses

| Term | Compiles To | Example |
|------|-------------|---------|
| `round-trips` | f(g(x)) == x for inverse pair | "can be reversed" |
| `round-trips with {inverse}` | inverse(f(x)) == x | "parse and format are consistent" |
| `is reversible with {g}` | g(f(x)) == x | "encoding can be decoded" |

#### Conservation

| Term | Compiles To | Example |
|------|-------------|---------|
| `does not change {field}` | before.field == after.field | "preserves the original" |
| `preserves {property}` | prop(before) == prop(after) | "keeps data intact" |
| `length is preserved` | len(result) == len(input) | "no data loss" |

#### Authorization (Future)

| Term | Compiles To | Example |
|------|-------------|---------|
| `requires {permission}` | Check context has permission | "needs login" |
| `cannot {action} without {condition}` | Check forbidden | "blocked without auth" |

#### Algebraic Laws (Future)

| Term | Compiles To | Example |
|------|-------------|---------|
| `is commutative` | op(a,b) == op(b,a) | "order doesn't matter" |
| `is associative` | op(op(a,b),c) == op(a,op(b,c)) | "grouping doesn't matter" |
| `has identity {e}` | op(x,e) == x | "zero element exists" |

---

## Part VI: Structural Enforcement

### Current State

System warns about issues but allows proceeding.

### Target State

System **refuses** to proceed when intent is ambiguous:

```
Error: Undefined glossary term "user is authenticated"
  Used in: Feature homepage, Scenario "logged in user sees dashboard"

  Fix: Define this term in the glossary section.

  Cannot proceed until intent is unambiguous.
```

### Enforcement Points

1. **Undefined terms** - All glossary references must resolve
2. **Orphan code** - Code without `@implements` generates warning (error in strict mode)
3. **Stale evidence** - Code changed but tests not re-run
4. **Incomplete coverage** - Feature has scenarios but no test data
5. **Circular definitions** - Already implemented in V1

### Configuration

```toml
# ntnt.toml
[intent]
strict = true           # Errors instead of warnings
require_implements = true   # All exported functions need @implements
require_test_data = true    # All scenarios need test cases
```

---

## Part VII: Intent Diff and Watch

### Intent Diff

Show semantic changes to intent, not just text changes:

```bash
$ ntnt intent diff HEAD~1

Feature: Homepage
  Scenario: Visitor discovers site
    + → they see "Get Started"     # New assertion added

Feature: Blog [NEW]
  + Scenario: Blog shows posts
  + Scenario: Blog post has author

Obligation summary:
  Added: 3 obligations
  Removed: 0 obligations
  Modified: 1 obligation
```

### Intent Watch

Continuous verification as files change:

```bash
$ ntnt intent watch server.tnt

Watching server.tnt and server.intent...

[12:34:56] Code changed: server.tnt
[12:34:57] Re-running intent check...
[12:34:58] ✓ 12 features, 47 scenarios, 210 checks passed

[12:35:12] Intent changed: server.intent
[12:35:12] New scenario added: "Blog post has tags"
[12:35:13] Warning: No test data for new scenario
```

---

## Part VIII: Implementation Phases

### Phase 6: Intent Studio Enhancement (2-3 weeks)

- [ ] Collapsible feature cards
- [ ] Scenario expansion with When/Then
- [ ] Assertion expansion with resolution chain
- [ ] Failure detail view with counterexamples
- [ ] Real-time status indicators

### Phase 7: Formal Obligation Model (1-2 weeks)

- [ ] Define Obligation struct
- [ ] Derive obligations from parsed intent
- [ ] Track obligation status (Proven/Unproven/Stale)
- [ ] Generate `.intent.lock` artifact
- [ ] Add `ntnt intent obligations` command

### Phase 8: Agent Protocol (2-3 weeks)

- [ ] Define protocol schema (JSON-RPC or HTTP)
- [ ] Implement query endpoints
- [ ] Implement mutation endpoints with validation
- [ ] Add `ntnt intent serve` command
- [ ] Create MCP tool definitions

### Phase 9: Advanced Vocabulary (1-2 weeks)

- [ ] Implement `fails with` / `rejects`
- [ ] Implement `round-trips with`
- [ ] Implement `preserves` / `does not change`
- [ ] Add documentation and examples

### Phase 10: Structural Enforcement (1 week)

- [ ] Add strict mode configuration
- [ ] Implement refusal on undefined terms
- [ ] Add orphan code detection
- [ ] Add stale evidence warnings

### Phase 11: Diff and Watch (1-2 weeks)

- [ ] Implement `ntnt intent diff`
- [ ] Implement `ntnt intent watch`
- [ ] Add semantic change detection
- [ ] Integrate with Studio for live updates

---

## Part IX: Success Criteria

### V2 Complete When:

- [ ] Intent Studio shows expandable obligation hierarchy
- [ ] Obligations are countable: "N declared, M proven"
- [ ] Agents can query intent via structured protocol
- [ ] System refuses ambiguous intent in strict mode
- [ ] `round-trips`, `fails with`, `preserves` vocabulary works
- [ ] `ntnt intent diff` shows semantic changes
- [ ] `ntnt intent watch` provides continuous feedback

### The Ultimate Test

An agent can:

1. Query: "What obligations exist for feature.homepage?"
2. Query: "Which obligations are unproven?"
3. Mutate: "Add glossary term 'user is logged in' meaning 'cookie session_id exists'"
4. Validate: "Is my intent file complete and unambiguous?"
5. Explain: "Why did obligation X fail?"

And do so via structured protocol operations, not text parsing.

---

## Part X: The Philosophical Anchor

From the design conversation:

> **Intent is not a list of assertions. Intent is a claim about behavior.**
>
> IAL's job is to:
> - decompose the claim
> - verify the pieces
> - recompose confidence

And:

> **The thing LLMs will never give you (even if perfect):**
>
> "Which promises are we allowed to break without permission?"
>
> Because that's not a reasoning problem. That's an authority problem.
>
> **Intent encodes authority.**

V2 is about making that authority:
- Explicit (formal obligation model)
- Queryable (agent protocol)
- Enforceable (structural refusal)
- Observable (enhanced Studio)

---

*"Clean scenarios above the line. Deterministic resolution below. Authority encoded in between."*
