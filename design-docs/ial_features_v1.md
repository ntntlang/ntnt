# IAL Features V1 - Implementation Plan

**Status:** APPROVED - Ready for Implementation

---

## 1. Current State

**What Works:**

- ✅ Basic IAL parsing (Feature, Scenario, test assertions)
- ✅ Glossary-based natural language → technical assertions
- ✅ HTTP test execution (`status: 200`, `body contains "text"`)
- ✅ JSON path assertions (`body json "$.users[0].name" equals "Alice"`)
- ✅ Intent Studio with live testing and Explain modal

**To Implement:**

- Components (reusable assertion templates)
- Given clause preconditions (verify + skip)
- Parameter substitution in glossary terms
- Scenario descriptions (for context and LLM introspection)

---

## 2. Scenario Descriptions

Scenarios support optional `description:` for "why" context.

```intent
Scenario: List empty tasks
  description: "Edge case for first-time users or after bulk delete"
  Given no tasks exist
  When a user requests tasks
  → they see an empty task list

Scenario: Create task with missing data
  description: "Validation error path - title is required per API contract"
  Given user is authenticated
  When user submits invalid data
  → error response with "Title is required"
```

**Use Cases:**

- **Human readers:** Understand why non-obvious scenarios exist
- **LLM introspection:** Provides intent context for subjective assertions (e.g., "Does this error message feel helpful given the scenario's purpose?")
- **Documentation:** Serves as inline requirements explanation

**Intent Studio Display:**

- Default: Hidden (scenario name + pass/fail only)
- Hover on scenario name → tooltip shows description
- Expand scenario → description shown below name

---

## 3. Preconditions: Verify + Skip

Given clauses are VERIFIED against current state. If precondition fails, the test is SKIPPED (not failed).

**Result Types:**

- ✅ **PASS** - precondition met, test passed
- ⏭️ **SKIP** - precondition not met, test not applicable
- ❌ **FAIL** - precondition met, test failed

**Execution Flow:**

1. Parse Given clause through glossary → HTTP assertion
2. Execute precondition check
3. If PASS → continue to main test; if FAIL → SKIP test

---

## 3. Components: Assertion Templates

Components reduce duplication. They are tested inline when referenced.

```intent
Component: Success Response
  id: component.success_response
  parameters: [message]
  Inherent Behavior:
    → status is 2xx
    → content-type is "application/json"
    → they see "ok"
    → they see {message}
```

**Testing Model:**

- Component assertions execute as part of the referencing scenario
- All component assertions must pass for the component reference to pass
- No standalone component testing

---

## 4. Glossary Design

### 4.1 Explicit Endpoint Specification

```intent
## Glossary
| Term | Means |
|------|-------|
| no tasks exist | GET /api/tasks returns body contains "\"tasks\":[]" |
| user submits valid data | POST /api/tasks with {"title": "Buy milk"} |
| success response with {text} | component.success_response(message: {text}) |
```

### 4.2 No Markup Required

Parser uses longest-match greedy tokenization. No special markup in scenarios.

### 4.3 Parameter Syntax: {param}

**Decision:** Use `{param}` for glossary term parameters.

**Example:**

```intent
## Glossary
| Term | Means |
|------|-------|
| success response with {text} | component.success_response(message: {text}) |
| user {name} sees {count} tasks | GET /api/users/{name}/tasks returns body contains "\"count\":{count}" |
```

**Usage:**

```intent
→ success response with "Task created"
→ user Alice sees 5 tasks
```

**Parsing:** Greedy match with parameter capture.

---

## 5. Implementation Details

### 5.1 Glossary Term Matching

Longest-match greedy tokenization. Convert patterns to regex for parameter extraction:

- `user {name} sees {count} tasks` → regex `user (.+?) sees (\d+) tasks`
- Capture groups extract parameters

### 5.2 Component Expansion

1. Parse assertion → match glossary term with parameters
2. Resolve to component reference: `component.id(param: value)`
3. Look up component by ID
4. Substitute `{param}` in inherent behavior
5. Execute all expanded assertions

### 5.3 Data Structures

```rust
pub struct Component {
    pub id: String,
    pub name: String,
    pub parameters: Vec<String>,
    pub inherent_behavior: Vec<String>,
    pub used_in: Vec<ComponentUsage>,
}

pub struct ComponentUsage {
    pub feature_id: String,
    pub feature_name: String,
    pub scenario_name: String,
}
```

---

## 6. Intent Studio Display

**Collapsed (default):**

```
📁 Feature: Task Management
  ✅ Create task
  ⏭️ Delete task (precondition not met)
```

**Expanded component:**

```
✅ Create task
  → ✅ success response with "Task created"  [component.success_response]
      ✅ status is 2xx
      ✅ content-type is "application/json"
      ✅ body contains "ok"
      ✅ body contains "Task created"
```

---

## 7. Full Example

```intent
# Task Management API

## Components

Component: Success Response
  id: component.success_response
  parameters: [message]
  Inherent Behavior:
    → status is 2xx
    → content-type is "application/json"
    → they see "ok"
    → they see {message}

Component: Error Response
  id: component.error_response
  parameters: [message]
  Inherent Behavior:
    → status is 4xx or 5xx
    → they see "error"
    → they see {message}

---

## Glossary

| Term | Means |
|------|-------|
| no tasks exist | GET /api/tasks returns body contains "\"tasks\":[]" |
| tasks exist | GET /api/tasks returns body not contains "\"tasks\":[]" |
| an empty task list | body contains "\"tasks\":[]" |
| user is authenticated | GET /api/me returns status is 2xx |
| user submits valid data | POST /api/tasks with {"title": "Buy milk"} |
| success response with {text} | component.success_response(message: {text}) |
| error response with {text} | component.error_response(message: {text}) |

---

Feature: Task Management
  id: feature.task_management

  Scenario: List empty tasks
    description: "Edge case for new users or after clearing all tasks"
    Given no tasks exist
    When a user requests tasks
    → they see an empty task list

  Scenario: Create task successfully
    description: "Happy path for task creation"
    Given user is authenticated
    When user submits valid data
    → success response with "Task created"

  Scenario: Create task with missing data
    description: "Validation error - title field is required per API contract"
    Given user is authenticated
    When user submits invalid data
    → error response with "Title is required"
```

---

## 8. Implementation Roadmap

### Phase 2: Preconditions + Components (THIS PHASE)

1. **Glossary Enhancements**
   - Explicit endpoint specification: `GET /api/tasks returns ...`
   - Parameter syntax: `{param}` extraction and substitution
   - Longest-match greedy tokenization
   - Component reference resolution: `component.id(param: value)`

2. **Component System**
   - Parse `Component:` blocks (id, parameters, inherent behavior)
   - Component expansion when referenced via glossary
   - Track component usage across scenarios
   - Display collapsed/expanded in Intent Studio

3. **Precondition System**
   - Parse `Given` clauses in scenarios
   - Resolve through glossary to HTTP assertions
   - Execute precondition check before main test
   - SKIP test if precondition fails

4. **Scenario Descriptions**
   - Parse optional `description:` field in scenarios
   - Pass to Intent Studio for hover/expand display
   - Include in LLM context for introspection features

5. **Intent Studio Updates**
   - Hover tooltips for glossary terms
   - Expandable component assertions
   - Precondition display with skip status

### Phase 3: Polish & Testing

- Comprehensive test suite for parser
- Edge case testing
- Documentation and examples

### Phase 4: Database Fixtures (FUTURE)

- Database setup actions for Given clauses
- Production safety checks (multi-signal detection)
- `--enable-fixtures` flag requirement

---

## 9. Success Criteria

- ✅ Parser handles all glossary/component/precondition syntax
- ✅ Components expand correctly with parameter substitution
- ✅ Preconditions execute and can skip tests
- ✅ Intent Studio displays all features cleanly
- ✅ Full component usage tracking

---

## 10. Architecture: Term Rewriting Engine

### 10.1 The Core Insight

IAL is a **term rewriting system**. Every natural language phrase rewrites to simpler terms until we reach primitives that actually execute. This is the same model as Lisp evaluation or mathematical substitution - proven, simple, infinitely powerful.

```
"they see success response with 'Created'"
    ↓ glossary lookup
"component.success_response(message: 'Created')"
    ↓ component expansion
["status is 2xx", "body contains 'ok'", "body contains 'Created'"]
    ↓ standard term resolution
[Check(status, in_range, 200-299), Check(body, contains, "ok"), Check(body, contains, "Created")]
    ↓ execution
[✓, ✓, ✓]
```

**Everything is just recursive term substitution.**

### 10.2 The Complete Data Model

```rust
/// The entire vocabulary - glossary, components, and standard terms unified
struct Vocabulary {
    terms: HashMap<Pattern, Definition>,  // All terms in one place
}

/// A pattern that matches natural language (with parameter capture)
struct Pattern {
    text: String,           // "success response with {message}"
    params: Vec<String>,    // ["message"]
}

/// What a term resolves to
enum Definition {
    /// Resolves to a sequence of other terms (recursion)
    Terms(Vec<Term>),
    /// Resolves to a primitive (base case)
    Primitive(Primitive),
}

/// A term reference with captured parameters
struct Term {
    text: String,                         // "body contains 'hello'"
    params: HashMap<String, Value>,       // captured parameter values
}

/// The primitives - the ONLY things that actually DO anything
/// This is a small, fixed set. New capabilities come from vocabulary, not new primitives.
enum Primitive {
    // === Actions (produce results) ===
    Http { method: String, path: String, body: Option<String> },
    Cli { command: String, args: Vec<String> },
    Sql { query: String },
    ReadFile { path: String },

    // === Checks (verify results) ===
    Check { op: CheckOp, path: String, expected: Value },
}

/// Check operations - the universal set of comparisons
enum CheckOp {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    Matches,        // regex
    Exists,
    NotExists,
    LessThan,
    GreaterThan,
    InRange,        // for status 2xx, timing assertions
}

/// Values in the system
enum Value {
    String(String),
    Number(f64),
    Bool(bool),
    Range(f64, f64),    // for "status is 2xx" → InRange(200, 299)
    Regex(String),
}
```

### 10.3 The Engine (Three Functions)

The entire IAL engine is **three pure functions**:

```rust
/// 1. PARSE: Text → Terms
fn parse(intent_file: &str) -> (Vocabulary, Vec<Scenario>);

/// 2. RESOLVE: Terms → Primitives (recursive rewriting)
fn resolve(term: &Term, vocab: &Vocabulary) -> Vec<Primitive> {
    // Look up term in vocabulary
    match vocab.lookup(&term.text) {
        Some(Definition::Primitive(p)) => {
            // Base case: substitute params and return
            vec![substitute_params(p, &term.params)]
        }
        Some(Definition::Terms(terms)) => {
            // Recursive case: resolve each sub-term
            terms.iter()
                .map(|t| substitute_params(t, &term.params))
                .flat_map(|t| resolve(&t, vocab))
                .collect()
        }
        None => {
            // Unknown term - error
            panic!("Unknown term: {}", term.text)
        }
    }
}

/// 3. EXECUTE: Primitives → Results
fn execute(primitives: &[Primitive], ctx: &mut Context) -> Vec<CheckResult> {
    primitives.iter().map(|p| match p {
        Primitive::Http { .. } => { ctx.response = do_http(p); CheckResult::Action }
        Primitive::Cli { .. } => { ctx.output = do_cli(p); CheckResult::Action }
        Primitive::Sql { .. } => { ctx.rows = do_sql(p); CheckResult::Action }
        Primitive::ReadFile { .. } => { ctx.content = do_read(p); CheckResult::Action }
        Primitive::Check { op, path, expected } => {
            let actual = ctx.get(path);
            check(op, actual, expected)
        }
    }).collect()
}
```

**That's the entire engine.** ~50 lines of core logic.

### 10.4 Standard Vocabulary (Pre-loaded)

The "built-in assertions" are just pre-loaded vocabulary terms:

```rust
fn standard_vocabulary() -> Vocabulary {
    vocabulary! {
        // HTTP assertions - all resolve to Check primitives
        "status: {code}"                    => Check(Equals, "response.status", {code}),
        "status is 2xx"                     => Check(InRange, "response.status", 200..299),
        "status is 4xx"                     => Check(InRange, "response.status", 400..499),
        "body contains {text}"              => Check(Contains, "response.body", {text}),
        "body not contains {text}"          => Check(NotContains, "response.body", {text}),
        "body matches {pattern}"            => Check(Matches, "response.body", {pattern}),
        "header {name} contains {value}"    => Check(Contains, "response.headers.{name}", {value}),
        "they see {text}"                   => Check(Contains, "response.body", {text}),
        "redirects to {path}"               => [Check(InRange, "response.status", 300..399),
                                                Check(Equals, "response.headers.location", {path})],

        // CLI assertions
        "exits successfully"                => Check(Equals, "cli.exit_code", 0),
        "exits with code {n}"               => Check(Equals, "cli.exit_code", {n}),
        "output contains {text}"            => Check(Contains, "cli.stdout", {text}),
        "error contains {text}"             => Check(Contains, "cli.stderr", {text}),

        // File assertions
        "file {path} exists"                => Check(Exists, "file.{path}", true),
        "file {path} contains {text}"       => Check(Contains, "file.{path}.content", {text}),

        // Database assertions
        "row exists where {condition}"      => Check(Exists, "db.query.{condition}", true),
        "row count is {n}"                  => Check(Equals, "db.query.count", {n}),

        // Timing assertions
        "responds in under {time}"          => Check(LessThan, "response.time_ms", {time}),
    }
}
```

**Adding new assertions = adding to vocabulary.** No code changes.

### 10.5 User Glossary + Components = More Vocabulary

User glossaries and components are just **additional vocabulary entries**:

```intent
## Glossary
| Term | Means |
|------|-------|
| no tasks exist | GET /api/tasks returns body contains "\"tasks\":[]" |
| an empty task list | body contains "\"tasks\":[]" |
```

Parses to:

```rust
vocab.add("no tasks exist", Terms([
    Term("GET /api/tasks"),           // resolves to Http primitive
    Term("body contains \"tasks\":[]") // resolves to Check primitive
]));
vocab.add("an empty task list", Primitive(Check(Contains, "response.body", "\"tasks\":[]")));
```

Components are the same:

```intent
Component: Success Response
  id: component.success_response
  parameters: [message]
  Inherent Behavior:
    → status is 2xx
    → body contains "ok"
    → body contains {message}
```

Parses to:

```rust
vocab.add("component.success_response(message: {message})", Terms([
    Term("status is 2xx"),
    Term("body contains 'ok'"),
    Term("body contains {message}"),
]));
```

**Components, glossary terms, and built-in assertions are all the same thing: vocabulary entries.**

### 10.6 Why This Architecture is Optimal

| Property                 | How It's Achieved                                     |
| ------------------------ | ----------------------------------------------------- |
| **Simple**               | 3 functions, 6 data types, ~100 lines of core logic   |
| **Extensible**           | Add terms to vocabulary, never modify engine          |
| **Composable**           | Terms can reference other terms infinitely            |
| **Debuggable**           | Resolution is pure - trace any term to its primitives |
| **Agent-friendly**       | Everything is explicit data, no hidden behavior       |
| **Backwards compatible** | Old syntax = old vocabulary entries that still work   |

### 10.7 Adding Future Features

Every feature from `INTENT_ASSERTION_LANGUAGE.md` is just vocabulary:

| Feature             | Implementation                                                |
| ------------------- | ------------------------------------------------------------- |
| CLI assertions      | Add standard vocabulary + `Cli` primitive already exists      |
| Database assertions | Add standard vocabulary + `Sql` primitive already exists      |
| File assertions     | Add standard vocabulary + `ReadFile` primitive already exists |
| Event assertions    | Add `EventCapture` primitive + vocabulary                     |
| Temporal extensions | Add `Sleep`, `Repeat` primitives + vocabulary                 |
| Probabilistic       | Add `InRange` checks with tolerance (already have `InRange`)  |
| State machines      | Vocabulary + `Context` state tracking                         |

**The engine never changes. Only vocabulary grows.**

### 10.8 Implementation Path

**Phase 2A: Build the Engine** (1-2 days)

```
src/intent/
  mod.rs           // Public API
  vocabulary.rs    // Pattern matching, term storage
  resolve.rs       // Recursive resolution (~30 lines)
  execute.rs       // Primitive execution (~50 lines)
  primitives.rs    // Primitive enum + CheckOp enum
  standard.rs      // Standard vocabulary definitions
```

**Phase 2B: Migrate Current Code** (1 day)

- Replace `parse_assertion()` if/else chain with vocabulary lookup
- Replace `run_assertions()` with `execute()`
- All existing tests pass unchanged

**Phase 2C: Add Glossary + Components** (2-3 days)

- Parse `## Glossary` section into vocabulary
- Parse `Component:` blocks into vocabulary
- Parse `Given`/`When`/`→` into term sequences
- Add precondition skip logic

### 10.9 Context Paths (How Checks Find Values)

The `path` in `Check(op, path, expected)` uses dot notation to navigate context:

```
response.status          → HTTP status code
response.body            → HTTP body string
response.headers.{name}  → HTTP header value
response.time_ms         → Response time in milliseconds

cli.exit_code            → Exit code
cli.stdout               → Standard output
cli.stderr               → Standard error

db.query.count           → Row count from last query
db.query.rows            → Array of rows

file.{path}.exists       → Whether file exists
file.{path}.content      → File contents
```

This is just string-based path lookup in a nested map. Simple.

### 10.10 The Beautiful Simplicity

```
┌──────────────────────────────────────────────────────────────────┐
│                         IAL ENGINE                                │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ┌─────────────┐                                                │
│   │ VOCABULARY  │  ← All knowledge lives here                    │
│   │             │    (standard + glossary + components)          │
│   │ term → def  │                                                │
│   │ term → def  │                                                │
│   │ term → def  │                                                │
│   └──────┬──────┘                                                │
│          │                                                       │
│          ▼                                                       │
│   ┌─────────────┐                                                │
│   │  RESOLVE    │  ← Pure function: term → primitives            │
│   │             │    (recursive substitution)                    │
│   └──────┬──────┘                                                │
│          │                                                       │
│          ▼                                                       │
│   ┌─────────────┐     ┌─────────────┐                           │
│   │ PRIMITIVES  │ ──▶ │  EXECUTE    │ ──▶ Results               │
│   │ (5 actions  │     │             │                           │
│   │  + checks)  │     └─────────────┘                           │
│   └─────────────┘                                                │
│                                                                  │
│   Actions: Http, Cli, Sql, ReadFile, (future: EventCapture...)  │
│   Checks:  Equals, Contains, Matches, Exists, InRange, LessThan │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

**The engine is done. Forever.**
**All future features are vocabulary.**
