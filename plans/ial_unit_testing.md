# IAL Unit Testing Extension - Design Plan

**Status:** DRAFT - Awaiting Review
**Date:** January 2026

---

## 1. Overview

Extend IAL to support unit testing of NTNT functions. The goal is a **two-tier glossary system**:

1. **System Glossary**: Universal test primitives (tiny set of "human-ish" words)
2. **Local Glossary**: Domain-specific terms that expand to bundles of system terms

This enables natural-language unit tests like:

```intent
Scenario: Slugify produces valid slugs
  When calling slugify with "Hello World"
  → result is "hello-world"
  → result is a valid URL slug
  → is deterministic
  → is idempotent
```

---

## 2. The Problem

Current IAL supports **behavioral testing** (HTTP, CLI, files) but NOT **function-level testing**:

- No way to call NTNT functions from intent files
- No property checks (deterministic, idempotent)
- No iteration constructs (`for each`, `for common cases`)
- No structured invariants (bundled assertions with semantic names)

---

## 3. Design Principles

### 3.1 Human Words → Universal Primitives

Every system term compiles to a small, fixed set of primitives. New capabilities come from **vocabulary**, not new code.

### 3.2 Composability

Complex assertions (like "valid URL slug") are built from simple terms. Invariants are just named bundles.

### 3.3 Two-Tier Glossary

| Layer | Who Defines | Example |
|-------|-------------|---------|
| **System** | IAL engine | `is`, `contains`, `is deterministic` |
| **Local** | User's .intent file | `valid URL slug`, `authenticated user` |

---

## 4. V1 System Vocabulary

### 4.1 Equality and Sameness

| Term | Compiles To |
|------|-------------|
| `is {expected}` | `Check(Equals, "eval.result", {expected})` |
| `equals {expected}` | `Check(Equals, "eval.result", {expected})` |
| `is the same as {other}` | `Check(Equals, "eval.result", {other})` |
| `differs from {other}` | `Check(NotEquals, "eval.result", {other})` |

### 4.2 Containment and Matching

| Term | Compiles To |
|------|-------------|
| `contains {text}` | `Check(Contains, "eval.result", {text})` |
| `does not contain {text}` | `Check(NotContains, "eval.result", {text})` |
| `matches {pattern}` | `Check(Matches, "eval.result", {pattern})` |
| `starts with {prefix}` | `Check(StartsWith, "eval.result", {prefix})` |
| `ends with {suffix}` | `Check(EndsWith, "eval.result", {suffix})` |

### 4.3 Type/Shape and Structure

| Term | Compiles To |
|------|-------------|
| `is a {type}` | `Check(IsType, "eval.result", {type})` |
| `has keys {keys}` | `Check(HasKeys, "eval.result", {keys})` |
| `has length {n}` | `Check(Equals, "eval.result.length", {n})` |
| `is non-empty` | `Check(NotEmpty, "eval.result", true)` |
| `is empty` | `Check(Empty, "eval.result", true)` |

### 4.4 Character/String Constraints

| Term | Compiles To |
|------|-------------|
| `uses only {charset}` | `Check(MatchesCharset, "eval.result", {charset})` |
| `is lowercase` | `Check(IsLowercase, "eval.result", true)` |
| `is uppercase` | `Check(IsUppercase, "eval.result", true)` |

### 4.5 Ordering and Bounds

| Term | Compiles To |
|------|-------------|
| `is greater than {n}` | `Check(GreaterThan, "eval.result", {n})` |
| `is less than {n}` | `Check(LessThan, "eval.result", {n})` |
| `is at least {n}` | `Check(GreaterOrEqual, "eval.result", {n})` |
| `is at most {n}` | `Check(LessOrEqual, "eval.result", {n})` |
| `is between {a} and {b}` | `Check(InRange, "eval.result", ({a}, {b}))` |

### 4.6 Errors and Failure

| Term | Compiles To |
|------|-------------|
| `fails` | `Check(Exists, "eval.error", true)` |
| `fails with {error}` | `Check(Contains, "eval.error", {error})` |
| `succeeds` | `Check(NotExists, "eval.error", true)` |

### 4.7 Stability Properties

| Term | Compiles To |
|------|-------------|
| `is deterministic` | `PropertyCheck(Deterministic, function, inputs)` |
| `is idempotent` | `PropertyCheck(Idempotent, function, inputs)` |

### 4.8 Round-trips and Inverses (V2)

| Term | Compiles To |
|------|-------------|
| `round-trips with {inverse}` | `PropertyCheck(RoundTrips, function, inverse)` |

---

## 5. New Primitives

### 5.1 Eval Primitive

```rust
/// Evaluate an NTNT function and store result in context
Eval {
    /// Function name (e.g., "slugify", "parse_json")
    function: String,
    /// Arguments to pass
    args: Vec<Value>,
}
```

**Context population:**
- `eval.result` - The function's return value
- `eval.error` - Error message if function failed/threw
- `eval.time_ms` - Execution time

### 5.2 PropertyCheck Primitive

```rust
/// Check a behavioral property that requires multiple evaluations
PropertyCheck {
    /// The property to verify
    property: Property,
    /// Function to test
    function: String,
    /// How to generate/specify inputs
    inputs: InputSpec,
}

enum Property {
    /// f(x) == f(x) for same input
    Deterministic,
    /// f(f(x)) == f(x)
    Idempotent,
    /// g(f(x)) == x
    RoundTrips { inverse: String },
}

enum InputSpec {
    /// Use the last evaluated input
    Current,
    /// Explicit examples
    Examples(Vec<Value>),
    /// Built-in corpus by type
    CommonCases(String),  // "strings", "numbers", "unicode", etc.
}
```

### 5.3 New CheckOps

```rust
enum CheckOp {
    // Existing
    Equals, NotEquals, Contains, NotContains, Matches,
    Exists, NotExists, LessThan, GreaterThan, InRange,

    // New for V1
    StartsWith,       // String starts with prefix
    EndsWith,         // String ends with suffix
    IsType,           // Value is of type
    HasKeys,          // Map/struct has specific keys
    NotEmpty,         // String/array/map is not empty
    Empty,            // String/array/map is empty
    MatchesCharset,   // String only contains chars from set
    IsLowercase,      // String is all lowercase
    IsUppercase,      // String is all uppercase
    GreaterOrEqual,   // Numeric >=
    LessOrEqual,      // Numeric <=
}
```

---

## 6. Invariants (Bundled Assertions)

An **Invariant** is a named bundle of assertions. It's syntactic sugar for a glossary term that expands to multiple checks.

### 6.1 Definition Syntax

```intent
Invariant: Valid URL Slug
  id: invariant.url_slug
  description: "A URL-safe slug string"

  Assertions:
    → uses only [a-z0-9-]
    → is lowercase
    → does not start with "-"
    → does not end with "-"
    → does not contain "--"
    → is non-empty
```

### 6.2 Implementation

Parses to vocabulary entries:

```rust
// Glossary entry
vocab.add_terms("is a valid URL slug", vec![
    Term::new("uses only \"[a-z0-9-]\""),
    Term::new("is lowercase"),
    Term::new("does not start with \"-\""),
    Term::new("does not end with \"-\""),
    Term::new("does not contain \"--\""),
    Term::new("is non-empty"),
]);

// Also register as component for tracking
components.insert("invariant.url_slug", InvariantDef { ... });
```

### 6.3 Usage

```intent
Scenario: Slugify produces valid slugs
  When calling slugify with "Hello World"
  → result is a valid URL slug
```

---

## 7. Iteration: `for each` and `for common cases`

### 7.1 `for each` (Table-Driven Tests)

```intent
Scenario: Edge cases
  for each:
    | input | expected |
    | "" | "post" |
    | "   " | "post" |
    | "---" | "post" |
    | "Hello---World" | "hello-world" |
  When calling slugify with {input}
  → result is {expected}
```

**Execution**: Run the scenario once per row, substituting variables.

### 7.2 `for common cases` (Built-in Corpus)

```intent
Scenario: Handles common strings
  for common cases: strings
  When calling slugify with {input}
  → result is a valid URL slug
  → is deterministic
```

**Built-in corpora** (V1):
- `strings`: empty, whitespace, unicode, long, special chars
- `numbers`: 0, -1, MAX_INT, MIN_INT, floats, NaN
- `edge`: null, empty array, empty map

---

## 8. Proof Point: The Slugify Invariant

### 8.1 Complete Example

```intent
# Slugify Function Tests
# Run: ntnt intent check slugify.tnt

## Invariants

Invariant: Valid URL Slug
  id: invariant.url_slug
  description: "A URL-safe slug string"

  Assertions:
    → uses only [a-z0-9-]
    → is lowercase
    → does not start with "-"
    → does not end with "-"
    → does not contain "--"
    → is non-empty

---

## Glossary

| Term | Means |
|------|-------|
| valid URL slug | → invariant.url_slug |

---

Feature: URL Slugification
  id: feature.slugify

  Scenario: Basic slugification
    When calling slugify with "Hello World"
    → result is "hello-world"
    → result is a valid URL slug

  Scenario: Preserves numbers
    When calling slugify with "Article 123"
    → result is "article-123"
    → result is a valid URL slug

  Scenario: Handles special characters
    When calling slugify with "What's Up? #Trending!"
    → result is "whats-up-trending"
    → result is a valid URL slug

  Scenario: Stability properties
    When calling slugify
    → is deterministic
    → is idempotent

  Scenario: Edge cases
    for each:
      | input | expected |
      | "" | "post" |
      | "   " | "post" |
      | "---" | "post" |
      | "Hello---World" | "hello-world" |
      | "UPPERCASE" | "uppercase" |
      | "  Trim Me  " | "trim-me" |
    When calling slugify with {input}
    → result is {expected}
    → result is a valid URL slug

  Scenario: Common string inputs
    for common cases: strings
    When calling slugify with {input}
    → result is a valid URL slug
    → is deterministic
```

### 8.2 What This Tests

| Assertion | What It Verifies |
|-----------|------------------|
| `result is "hello-world"` | Exact output for known input |
| `result is a valid URL slug` | Invariant bundle (6 checks) |
| `is deterministic` | Same input → same output |
| `is idempotent` | f(f(x)) == f(x) |
| `for each` | Table-driven edge cases |
| `for common cases` | Fuzz with built-in corpus |

---

## 9. Architecture Changes

### 9.1 File Changes

| File | Changes |
|------|---------|
| `src/ial/primitives.rs` | Add `Eval`, `PropertyCheck`, new `CheckOp` variants |
| `src/ial/execute.rs` | Implement `execute_eval()`, `execute_property_check()`, new check ops |
| `src/ial/standard.rs` | Add system vocabulary for unit testing |
| `src/ial/corpus.rs` | **NEW**: Built-in test corpora |
| `src/intent.rs` | Parse Invariant blocks, `for each` tables |
| `src/intent.rs` | Connect Eval to interpreter |

### 9.2 Interpreter Integration

The `Eval` primitive needs access to the NTNT interpreter. Options:

**Option A: Pass interpreter to execute()**
```rust
pub fn execute(
    primitive: &Primitive,
    ctx: &mut Context,
    port: u16,
    interpreter: Option<&mut Interpreter>,  // NEW
) -> ExecuteResult
```

**Option B: Interpreter in Context**
```rust
pub struct Context {
    values: HashMap<String, Value>,
    interpreter: Option<Rc<RefCell<Interpreter>>>,  // NEW
}
```

**Recommendation**: Option A is cleaner - pass interpreter only when needed for unit tests.

### 9.3 Context Paths for Eval

```
eval.result           → Function return value
eval.result.length    → Length if result is string/array
eval.result.keys      → Keys if result is map
eval.error            → Error message if failed
eval.time_ms          → Execution time
```

---

## 10. Implementation Phases

### Phase 1: Core Primitives (Foundation)
- [ ] Add `Eval` primitive to primitives.rs
- [ ] Add new `CheckOp` variants (StartsWith, EndsWith, etc.)
- [ ] Implement `execute_eval()` with interpreter integration
- [ ] Implement new check operations in execute.rs

### Phase 2: System Vocabulary
- [ ] Add system terms to standard.rs (is, contains, starts with, etc.)
- [ ] Test basic function calls: `When calling foo with "bar" → result is "baz"`

### Phase 3: Invariants
- [ ] Parse `Invariant:` blocks in intent.rs
- [ ] Auto-generate glossary entry from invariant
- [ ] Test the slugify invariant

### Phase 4: Property Checks
- [ ] Add `PropertyCheck` primitive
- [ ] Implement `is deterministic` and `is idempotent`
- [ ] Implement `InputSpec::Current`

### Phase 5: Iteration
- [ ] Parse `for each:` tables
- [ ] Execute scenario per row
- [ ] Add `for common cases` with built-in corpus

### Phase 6: Polish
- [ ] Intent Studio display for unit test results
- [ ] Error messages with helpful suggestions
- [ ] Documentation and examples

---

## 11. Open Questions

1. **How do we specify which function to test?**
   - `When calling {function} with {args}` syntax?
   - Implicit from `@implements` annotation?

2. **How do we handle async functions?**
   - Block until complete?
   - Timeout?

3. **Should invariants support parameters?**
   - `Invariant: Bounded Number(min, max)` with `→ is at least {min}`?

4. **How do we handle side effects?**
   - Pure function assumption for property checks?
   - `is pure` as an assertion?

---

## 12. Success Criteria

- [ ] Can write `When calling slugify with "Hello" → result is "hello"`
- [ ] Can define invariants that bundle multiple assertions
- [ ] Can verify `is deterministic` and `is idempotent`
- [ ] Can use `for each` for table-driven tests
- [ ] Can use `for common cases` for fuzz testing
- [ ] All checks visible in Intent Studio

---

## 13. References

- Current IAL architecture: `src/ial/`
- IAL Features V1 plan: `plans/ial_features_v1.md`
- IAL Specification: `docs/INTENT_ASSERTION_LANGUAGE.md`
- ROADMAP Phase 7: Testing Framework
