# IAL Extension Plan

**Status:** COMPLETE (All Phases Done)
**Date:** January 2026
**Approach:** Iterate on working system, don't rewrite

---

## Executive Summary

### Phases 0-5 Implementation (January 2026)

All phases of unit testing are complete:

**Phase 0:** Core infrastructure
- ✅ `FunctionCall`, `PropertyCheck`, `InvariantCheck` primitives
- ✅ Keyword syntax parsing (`call:`, `input:`, `check:`, `property:`)
- ✅ Cycle detection in glossary term resolution
- ✅ Function call execution via interpreter
- ✅ Unit test vocabulary (result assertions, property checks)

**Phase 1:** Invariant resolution
- ✅ Parse `Invariant:` blocks with bundled assertions
- ✅ `check: invariant.{id}` resolves to assertion bundle
- ✅ Invariants added to IAL vocabulary for term resolution
- ✅ Parameterized invariants with test data substitution

**Phase 2:** Test data binding
- ✅ Parse `Test Cases:` blocks with tabular data
- ✅ `input: test_data.{id}` references test data sections
- ✅ Scenarios expand to multiple test cases (one per row)
- ✅ Row values substituted into test case parameters

**Phase 3:** Corpus testing
- ✅ Built-in corpora defined (`corpus.strings`, `corpus.numbers`, `corpus.edge`)
- ✅ `input: corpus.strings` generates ~30 edge case test inputs
- ✅ Corpus values substituted as `{input}` variable

**Phase 4:** Property checks
- ✅ `property: deterministic` - calls function twice, verifies same result
- ✅ `property: idempotent` - calls f(f(x)), verifies equals f(x)

**Phase 5:** Verbosity levels
- ✅ Default: Summary output (feature names + pass/fail counts)
- ✅ `-v`: Scenario output (each scenario listed, failures shown with details)
- ✅ `-vv`: Assertion output (all assertions, When/Then clauses, term resolution)

**V1 COMPLETE** - All planned IAL unit testing features implemented.

---

The current IAL system works:
- **Glossary** - Term rewriting ✓
- **Components** - Reusable templates ✓
- **Feature → Scenarios** - Core structure ✓
- **Constraints** - Cross-cutting concerns ✓

This plan adds **incremental capabilities** without restructuring:

1. **Invariants** - Bundled assertions with IDs
2. **Keyword glossary syntax** - `call:`, `input:`, `check:`, `property:` for unit testing
3. **Comprehensive system vocabulary** - Equality, bounds, stability, errors, round-trips, conservation
4. **Corpus testing** - Built-in test sets (strings, numbers, edge cases)
5. **Property checks** - Deterministic, idempotent, round-trip verification
6. **Verbosity levels** - `-v`, `-vv` output modes

**Key principle:** Human-facing scenarios stay completely clean. All technical syntax lives in the glossary (system-managed section).

---

## Part I: Philosophy (Unchanged)

### The Core Insight

> **Humans think in bundles of meaning. Computers verify atomic facts.**

IAL lets humans see bundles while the system executes atoms.

### What Intent Is

Intent is NOT a test framework. Intent IS:
- A claim about behavior
- A commitment that must be proven
- A semantic anchor against drift

### Why This Matters

Better models make interpretation more dangerous because they're more convincing when wrong. Models cannot know which interpretations are allowed to change. **Intent encodes commitment.**

---

## Part II: What We Have (Reference)

### Current server.intent Structure

```intent
# Project Name

## Glossary
| Term | Means |
|------|-------|
| a visitor goes to the {path} | GET {path} |
| home page | / |
| page loads successfully | status 200, returns HTML |
| they see {text} | body contains {text} |

## Components
Component: Branded Page
  id: component.branded_page
  inherent_behavior:
    - Site logo visible
    - Navigation present

---

Feature: Homepage
  id: feature.homepage
  description: "Landing page with hero section"

  Scenario: Visitor discovers site
    When a visitor goes to the home page
    → page loads successfully
    → they see "Welcome"

---

Constraint: Responsive Design
  applies_to: [feature.homepage]
```

**This works. We're adding to it, not replacing it.**

---

## Part III: Addition 1 - Invariants

### The Problem

Without invariants, you'd need to repeat technical assertions everywhere:

```intent
# BAD: Technical jargon repeated in every scenario
Scenario: Simple title works
  When generating a URL from "Hello World"
  → result uses only [a-z0-9-]
  → result is lowercase
  → result does not start with "-"
  → result does not end with "-"
  → result is non-empty

Scenario: Complex title works
  When generating a URL from "What's Up?"
  → result uses only [a-z0-9-]    # repeated
  → result is lowercase            # repeated
  ...
```

### The Solution

**Step 1:** Define the invariant once (system-level, humans don't read this):

```intent
Invariant: URL Slug
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

**Step 2:** Map to human language in glossary:

```intent
## Glossary
| Term | Means |
|------|-------|
| URL is valid | satisfies invariant.url_slug |
```

**Step 3:** Use human language in scenarios:

```intent
# GOOD: Human-readable
Scenario: Blog Posts Have Valid URLs
  When generating a URL from "Hello World"
  → result is "hello-world"
  → URL is valid
```

The human reads "URL is valid". The system expands it to 6 technical checks.

### Invariant Syntax

```intent
Invariant: {Name}
  id: invariant.{snake_case_id}
  description: "{what this invariant means}"

  Assertions:
    → {assertion 1}
    → {assertion 2}
    ...
```

### Parameterized Invariants (V1.1)

```intent
Invariant: Bounded Integer
  id: invariant.bounded_int
  parameters:
    - min: Int
    - max: Int

  Assertions:
    → is at least {min}
    → is at most {max}
```

Glossary: `| score is valid | satisfies invariant.bounded_int(0, 100) |`
Usage: `→ score is valid`

---

## Part IV: Addition 2 - Test Data (System-Managed)

### The Problem

The human shouldn't have to read through tables of test data. They want to confirm:
- "Blog posts have valid URLs" ✓

Not read through 10 rows of example inputs.

### The Solution

**Human-facing scenarios stay clean:**

```intent
Feature: Blog Posts
  id: feature.blog_posts
  description: "Authors write blog posts with titles and clean URLs"

  Scenario: Blog Posts Have Valid URLs
    When generating a URL from a title
    → URL is valid
    → URL matches expected format

  Scenario: Blog Post URLs Are Predictable
    When generating a URL from a title
    → URL is predictable
    → URL is stable
```

**Test data lives in system-managed section:**

```intent
---

## Test Data (System-Managed)

# Agent generates/maintains this based on scenarios

Test Cases: URL Examples
  for: feature.blog_posts
  scenario: Blog Posts Have Valid URLs

  | title | expected |
  | "Hello World" | "hello-world" |
  | "My First Post!" | "my-first-post" |
  | "What's Up?" | "whats-up" |
  | "UPPERCASE TITLE" | "uppercase-title" |
  | "  Extra   Spaces  " | "extra-spaces" |
  | "Numbers 123 Here" | "numbers-123-here" |
  | "Special @#$% Chars" | "special-chars" |
  | "already-valid-slug" | "already-valid-slug" |
  | "Trailing Punctuation..." | "trailing-punctuation" |
  | "---Leading Dashes" | "leading-dashes" |
```

### How It Works

1. Human writes clean scenario: "Blog Posts Have Valid URLs"
2. Glossary maps "generating a URL from a title" → function call with `{title}` variable
3. System finds test data tagged with `scenario: Blog Posts Have Valid URLs`
4. System runs the scenario for each row, binding `{title}` and `{expected}`
5. Human sees: "Blog Posts Have Valid URLs ✓ (10 examples)"

### The Separation

| Section | Who Writes | Who Reads |
|---------|------------|-----------|
| Features & Scenarios | Human confirms | Human |
| Glossary | Agent drafts, human confirms | System |
| Invariants | Agent drafts | System |
| Test Data | Agent generates | System |

The human only reads the top section. Everything below `---` is system-managed.

---

## Part V: Addition 3 - Corpus Testing

### The Problem

Edge cases are easy to miss. You might test "Hello World" but forget:
- Empty string `""`
- Whitespace only `"   "`
- Very long strings (1000+ chars)
- Unicode `"Héllo Wörld 🎉"`
- Only special characters `"@#$%^&*"`
- SQL injection attempts `"'; DROP TABLE posts;--"`

### The Solution

**Human writes clean scenario:**

```intent
Scenario: Blog Posts Handle Any Title
  description: "Edge cases and weird inputs don't break URL generation"
  When generating a URL from user input
  → URL is valid
```

**Glossary maps "user input" to corpus testing:**

```intent
## Glossary
| Term | Means |
|------|-------|
| generating a URL from user input | call: slugify({input}), input: corpus.strings |
```

The human writes "user input". The system runs ~20 edge case tests.

### Available Corpora (System-Level)

| Corpus | Contains |
|--------|----------|
| `strings` | empty, whitespace, unicode, long, special chars, quotes, injection attempts |
| `numbers` | 0, -1, 1, MAX_INT, MIN_INT, floats, NaN, Infinity |
| `edge` | null, undefined, empty array, empty map |

### How It Works

```
Human writes:      "When generating a URL from user input"
                           ↓
Glossary resolves: "calling slugify with {input}, for common cases: strings"
                           ↓
System expands:    Run slugify() with ~20 corpus values
                           ↓
Human sees:        "Blog Posts Handle Any Title ✓ (20 examples)"
```

No `{input}` variables or `for common cases:` syntax visible to humans.

---

## Part VI: Addition 4 - Property Checks

### The Problem

Some properties can't be tested with a single assertion:
- **Deterministic**: Does the same input always produce the same output?
- **Idempotent**: If I run it twice, do I get the same result?

These require calling the function multiple times and comparing.

### The Solution

Map property checks to human language in the glossary:

```intent
## Glossary
| Term | Means |
|------|-------|
| URL is predictable | is deterministic |
| URL is stable | is idempotent |
```

Then use human language:

```intent
Scenario: Blog Post URLs Don't Change
  When generating a URL from "Hello World"
  → URL is predictable
  → URL is stable
```

The human reads "URLs are predictable and stable". The system:
1. Calls the function twice with same input, compares results
2. Calls the function on its own output, checks it doesn't change

### Available Properties

| Property | What It Checks | Human-Friendly Term |
|----------|----------------|---------------------|
| `is deterministic` | f(x) == f(x) for same input | "is predictable", "is consistent" |
| `is idempotent` | f(f(x)) == f(x) | "is stable", "doesn't change on reprocess" |
| `round-trips with {inverse}` | g(f(x)) == x | "can be reversed" |

### How It Works

When the system sees `is deterministic`:
1. Run the function with current input → result1
2. Run the function again with same input → result2
3. Pass if result1 == result2

For `is idempotent`:
1. Run f(x) → result1
2. Run f(result1) → result2
3. Pass if result1 == result2

---

## Part VII: Addition 5 - Verbosity Levels

### The Problem

Default output shows everything or nothing. Humans need progressive disclosure.

### The Solution

Three verbosity levels:

**Default: Summary**
```
$ ntnt intent check server.tnt

✔️ Homepage          3 scenarios passed
✔️ Learn Page        5 scenarios passed
✔️ Blog              4 scenarios passed

All features satisfied. 12 scenarios · 47 checks
```

**Verbose (-v): Scenarios**
```
$ ntnt intent check server.tnt -v

Feature: Homepage
  ✔️ Visitor discovers site
  ✔️ Visitor sees call-to-action
  ✔️ Visitor sees feature highlights

Feature: Learn Page
  ✔️ Developer reads quick start
  ✔️ Developer sees install scripts
  ...
```

**Very Verbose (-vv): Assertions**
```
$ ntnt intent check server.tnt -vv

Feature: Homepage
  Scenario: Visitor discovers site
    When a visitor goes to the home page
      → resolved: GET /
    ✔️ page loads successfully
      → resolved: status 200, returns HTML
        ✔️ status 200
        ✔️ header "Content-Type" contains "text/html"
    ✔️ has proper layout
      → resolved: has site branding, has navigation, body contains "footer"
        ✔️ body contains "ntnt-logo"
        ✔️ body contains "NTNT"
        ✔️ body contains "Learn"
        ✔️ body contains "Blog"
        ✔️ body contains "footer"
```

This shows how glossary terms decompose recursively.

### Failure Output

On failure, automatically show relevant detail:

```
$ ntnt intent check server.tnt

✔️ Homepage
✔️ Learn Page
❌ Blog

  Scenario: Blog shows posts
    When a visitor goes to the blog
      → resolved: GET /blog
    ✔️ status 200
    ❌ body contains "Latest Posts"
       Expected: body to contain "Latest Posts"
       Actual: body contains "Blog" but not "Latest Posts"
```

---

## Part VIII: Implementation Plan

### Phase 0: Function Calls via Glossary ✓ COMPLETE

This is the foundation that enables unit testing.

- [x] Parse `call:` keyword syntax in glossary definitions
- [x] Resolve glossary term → function call → execute → store result
- [x] Support `result` as the captured return value
- [x] Find functions in linked .tnt file
- [x] **Cycle detection:** Build dependency graph, detect cycles via topological sort
- [x] **Depth limit:** Fail after 50 expansion levels as runtime safety net

**Implemented in:**
- `ial/primitives.rs`: Added `FunctionCall`, `PropertyCheck`, `InvariantCheck` primitives
- `ial/execute.rs`: Added execution functions for new primitives
- `ial/resolve.rs`: Added cycle detection with path tracking and clear error messages
- `ial/standard.rs`: Added unit test vocabulary (result assertions, property checks, string assertions)
- `intent.rs`: Added `KeywordSyntax` parsing, `WhenAction::FunctionCall`, `run_function_call_test()`

**Test:** `| generating a URL from {title} | call: slugify({title}) |` → executes slugify()
**Test:** Circular glossary reference → clear error with cycle path

### Phase 1: Invariants (1-2 weeks) - ✅ COMPLETE

- [x] Parse `Invariant:` blocks in intent files
- [x] Store invariants with IDs (`Invariant` struct in `intent.rs`)
- [x] Resolve `check: invariant.{id}` to assertion bundle
- [x] Allow glossary terms to reference invariants
- [x] Parameterized invariants with `{param}` substitution from test data

**Test:** `| URL is valid | check: invariant.url_slug |` → expands to 6 checks ✓

### Phase 2: Test Data Linking (1 week) - ✅ COMPLETE

- [x] Parse `Test Cases:` blocks in system-managed section
- [x] Link test data to features via `for: feature.id`
- [x] When scenario runs, find linked test data
- [x] Run scenario once per row, substituting variables

**Test:** Scenario for `feature.text_utilities` finds and runs 12 test cases ✓

### Phase 3: Corpus Testing (1 week) - ✅ COMPLETE

- [x] Define built-in corpora (strings, numbers, edge)
- [x] Parse `input: corpus.strings` keyword syntax
- [x] Expand to scenario per corpus value
- [x] Bind `{input}` variable

**Test:** `input: corpus.strings` runs ~30 edge case examples ✓

### Phase 4: Property Checks (1-2 weeks) - ✅ COMPLETE

- [x] Parse `property: deterministic` assertion
- [x] Parse `property: idempotent` assertion
- [x] Implement multi-evaluation execution
- [x] Report property failures with counterexamples

**Test:** Non-deterministic function fails with "different results" message ✓

### Phase 5: Verbosity Levels (1 week) - ✅ COMPLETE

- [x] Add `-v` and `-vv` flags to `ntnt intent check`
- [x] Implement three output formatters (summary, scenarios, assertions)
- [x] Show automatic detail on failure (at all verbosity levels)
- [ ] Update Intent Studio with expandable views (future enhancement)

**Test:** Same intent file, three different output levels ✓

---

## Part IX: Glossary Syntax Reference

The glossary supports **two syntax styles** for the "means" column:

### Style 1: Term Rewriting (Original, Backwards Compatible)

Simple comma-separated terms that expand recursively:

```intent
## Glossary

| Term | Means |
|------|-------|
| a visitor goes to the {path} | GET {path} |
| home page | / |
| page loads successfully | status 200, returns HTML |
| returns HTML | header "Content-Type" contains "text/html" |
| has proper layout | has site branding, has navigation, body contains "footer" |
| they see {text} | body contains {text} |
```

This is the existing working style from `server.intent`. **It continues to work unchanged.**

### Style 2: Keyword Syntax (New, for Unit Testing)

Deterministic keywords for function calls and property tests:

| Keyword | Purpose | Example |
|---------|---------|---------|
| `call:` | Invoke a function | `call: slugify({title})` |
| `input:` | Specify input source | `input: corpus.strings` |
| `check:` | Assert against invariant | `check: invariant.url_slug` |
| `property:` | Property-based check | `property: deterministic` |

```intent
## Glossary

| Term | Means |
|------|-------|
| generating a URL from a title | call: slugify({title}), input: test_data |
| generating a URL from user input | call: slugify({input}), input: corpus.strings |
| URL is valid | check: invariant.url_slug |
| URL is predictable | property: deterministic |
```

### How the Parser Chooses

**Detection rule:** If the "means" column contains `call:`, `input:`, `check:`, or `property:`, parse as keyword syntax. Otherwise, parse as term rewriting.

This allows both styles to coexist in the same file:

```intent
## Glossary

| Term | Means |
|------|-------|
| page loads successfully | status 200, returns HTML |
| they see {text} | body contains {text} |
| generating a URL from a title | call: slugify({title}), input: test_data |
| URL is valid | check: invariant.url_slug |
```

### Combining Keywords

Multiple keywords are comma-separated:

```
| generating a URL from user input | call: slugify({input}), input: corpus.strings |
```

### Input Sources (Keyword Syntax)

| Source | Meaning |
|--------|---------|
| `input: test_data` | Use Test Data section for this scenario |
| `input: corpus.strings` | Use built-in strings corpus |
| `input: corpus.numbers` | Use built-in numbers corpus |
| `input: corpus.edge` | Use built-in edge cases corpus |

### Resolution Priority

**For term rewriting:**
1. Match term pattern in glossary
2. Expand to comma-separated terms
3. Recursively expand each term until reaching system vocabulary

**For keyword syntax:**
1. Match term pattern in glossary
2. Extract keyword:value pairs
3. Execute `call:` with function
4. Bind variables from `input:` source
5. Run `check:` and `property:` assertions

### Cycle Detection (Preventing Infinite Loops)

Recursive term rewriting can create infinite loops:

```intent
## Glossary (BAD - creates cycle)
| Term | Means |
|------|-------|
| A | B |
| B | C, A |
```

Expansion: A → B → C, A → C, B → C, C, A → ... (infinite)

**Solution: Static cycle detection at parse time**

When loading a glossary, the system:

1. **Builds a dependency graph** - For each term, track which other glossary terms it references
2. **Detects cycles** - Use topological sort; if sorting fails, a cycle exists
3. **Reports clear error** - Show the cycle path to the user

```
Error: Circular reference in glossary
  A → B → A

Fix: Break the cycle by defining one term using only system vocabulary.
```

**Implementation:**

```
fn check_glossary_cycles(glossary):
    graph = build_dependency_graph(glossary)

    for term in glossary:
        visited = {}
        if has_cycle(term, graph, visited, path=[]):
            error("Circular reference: {path}")

    return ok
```

**Runtime safety net:**

Even with static analysis, add a depth limit (e.g., 50 levels) as a failsafe:

```
Error: Maximum expansion depth (50) exceeded for term "A"
  This usually indicates a circular reference.
```

**Valid recursive patterns:**

Not all chains are bad. This is fine:

```intent
| has proper layout | has site branding, has navigation |
| has site branding | body contains "logo", body contains "NTNT" |
| has navigation | body contains "Learn", body contains "Blog" |
```

This terminates because each term eventually reaches system vocabulary (`body contains`).

---

## Part X: System Vocabulary (Built-in Terms)

These terms are built into IAL and don't need glossary definitions. Map them to human-friendly language in the glossary for scenarios.

### Equality and Sameness
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `is {expected}` | Check(Equals, result, {expected}) | "result is correct" |
| `equals {expected}` | Check(Equals, result, {expected}) | "matches expected value" |
| `differs from {value}` | Check(NotEquals, result, {value}) | "is different from original" |
| `is the same as {ref}` | Check(Equals, a, b) | "stays the same" |

### Containment and Matching
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `contains {text}` | Check(Contains, target, {text}) | "includes the keyword" |
| `does not contain {text}` | Check(NotContains, target, {text}) | "excludes bad words" |
| `matches {pattern}` | Check(Matches, target, pattern) | "looks like an email" |
| `starts with {prefix}` | Check(StartsWith, target, prefix) | "begins correctly" |
| `ends with {suffix}` | Check(EndsWith, target, suffix) | "has proper ending" |
| `body contains {text}` | Check(Contains, body, {text}) | "page shows message" |

### Type and Shape
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `is a {type}` | Check(IsType, result, type) | "is a valid response" |
| `has keys {list}` | Check(HasKeys, result, list) | "has all required fields" |
| `has length {n}` | Check(Equals, len(result), n) | "has correct size" |
| `is non-empty` | Check(NotEmpty, result) | "is not blank" |

### Bounds and Ordering
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `is at least {min}` | Check(GTE, result, min) | "meets minimum" |
| `is at most {max}` | Check(LTE, result, max) | "stays within limit" |
| `is between {min} and {max}` | Check(InRange, result, min, max) | "is in valid range" |
| `is greater than {value}` | Check(GT, result, value) | "exceeds threshold" |
| `is less than {value}` | Check(LT, result, value) | "is below maximum" |
| `never exceeds {cap}` | Invariant(LTE, result, cap) | "respects the cap" |

### Stability Properties
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `is deterministic` | PropertyCheck(f(x) == f(x)) | "is predictable" |
| `is idempotent` | PropertyCheck(f(f(x)) == f(x)) | "is stable" |
| `is pure` | PropertyCheck(NoSideEffects) | "has no side effects" |

### Errors and Rejection
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `fails with {error}` | Check(Raises, fn, error) | "rejects bad input" |
| `fails with message containing {text}` | Check(RaisesContaining, fn, text) | "explains what's wrong" |
| `rejects {input}` | Check(Rejects, fn, input) | "refuses invalid data" |
| `never crashes` | Invariant(NoPanic) | "handles errors gracefully" |
| `never corrupts {resource}` | Invariant(Preserves, resource) | "keeps data safe" |

### Round-trips and Inverses
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `round-trips` | Check(f(g(x)) == x) | "can be reversed" |
| `round-trips with {inverse}` | Check(inverse(f(x)) == x) | "parse and format are consistent" |
| `is reversible with {g}` | Check(g(f(x)) == x) | "can be undone" |

### Conservation
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `does not change {field}` | Check(Equals, before.field, after.field) | "preserves the original" |
| `preserves {property}` | Check(Equals, prop(before), prop(after)) | "keeps data intact" |

### Authorization
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `requires {permission}` | Check(HasPermission, ctx, permission) | "needs login" |
| `cannot {action} without {condition}` | Check(Forbidden, action, condition) | "blocked without auth" |

### Algebraic Laws (Advanced)
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `is commutative` | Check(op(a,b) == op(b,a)) | "order doesn't matter" |
| `is associative` | Check(op(op(a,b),c) == op(a,op(b,c))) | "grouping doesn't matter" |
| `has identity {e}` | Check(op(x,e) == x) | "zero element exists" |

### Quantification (Test Sets)
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `for each {set}` | ForEach(set, assertion) | "works for all cases" |
| `for common cases` | ForEach(corpus, assertion) | "handles edge cases" |
| `for all inputs of kind {K}` | PropertyBased(K, assertion) | "works for any input" |

### HTTP
| System Term | Compiles To | Human-Friendly Example |
|-------------|-------------|------------------------|
| `status {code}` | Check(Equals, response.status, code) | "succeeds" |
| `status 2xx` | Check(InRange, response.status, 200-299) | "returns success" |
| `redirects to {path}` | Check(Redirect, response, path) | "sends to new page" |
| `returns JSON` | Check(ContentType, response, "application/json") | "gives JSON response" |

---

### Mapping System Terms to Human Language

The key insight: **system terms stay in the glossary, human language stays in scenarios**.

```intent
## Glossary

| Term | Means |
|------|-------|
| URL is well-formed | matches [a-z0-9-]+, is non-empty |
| result is predictable | is deterministic |
| handles bad input gracefully | never crashes, rejects {invalid} |
| data stays safe | does not change {original}, preserves {integrity} |
| operation can be undone | round-trips with {inverse} |
```

Then scenarios read naturally:

```intent
Scenario: URLs are reliable
  When generating a URL from a title
  → URL is well-formed
  → result is predictable

Scenario: System is robust
  When processing user input
  → handles bad input gracefully
  → data stays safe
```

**The human never sees the system terms directly.**

---

## Part XI: What We're NOT Doing

To keep this iterative and simple, we're explicitly **not** doing:

1. **Obligations section** - System infers these, humans don't write them
2. **Implementation Binding section** - `@implements` in code is enough
3. **Four-layer architecture rewrite** - Conceptually useful, not a code change
4. **Agent protocol** - Future work
5. **Restructuring Feature format** - Current format works

---

## Part XII: How Unit Testing Works

### The Key Insight

The **Glossary** bridges human language to function calls. Test data is system-managed, not human-facing.

### Complete Example: Testing Blog Post URL Generation

**What the human reads and confirms:**

```intent
# Blog Application

Feature: Blog Posts
  id: feature.blog_posts
  description: "Authors write blog posts with titles and clean URLs"

  Scenario: Blog Posts Have Valid URLs
    description: "URLs are generated correctly from titles"
    When generating a URL from a title
    → URL is valid
    → URL matches expected format

  Scenario: Blog Post URLs Are Predictable
    description: "Same title always produces same URL"
    When generating a URL from a title
    → URL is predictable
    → URL is stable

  Scenario: Blog Posts Handle Any Title
    description: "Edge cases and weird inputs don't break URL generation"
    When generating a URL from user input
    → URL is valid
```

**That's it.** Three clean scenarios with descriptions. Human confirms "yes, that's what I want."

---

**What the system manages (human doesn't need to read):**

```intent
---

## Glossary (System-Managed)

| Term | Means |
|------|-------|
| generating a URL from a title | call: slugify({title}), input: test_data |
| generating a URL from user input | call: slugify({input}), input: corpus.strings |
| URL is valid | check: invariant.url_slug |
| URL matches expected format | result is {expected} |
| URL is predictable | property: deterministic |
| URL is stable | property: idempotent |

---

## Invariants (System-Managed)

Invariant: URL Slug
  id: invariant.url_slug

  Assertions:
    → uses only [a-z0-9-]
    → is lowercase
    → does not start with "-"
    → does not end with "-"
    → does not contain "--"
    → is non-empty

---

## Test Data (System-Managed)

Test Cases: URL Examples
  for: feature.blog_posts
  scenario: Blog Posts Have Valid URLs

  | title | expected |
  | "Hello World" | "hello-world" |
  | "My First Post!" | "my-first-post" |
  | "What's the Deal?" | "whats-the-deal" |
  | "UPPERCASE TITLE" | "uppercase-title" |
  | "  Extra   Spaces  " | "extra-spaces" |
  | "Numbers 123 Here" | "numbers-123-here" |
  | "Special @#$% Chars" | "special-chars" |
  | "already-valid-slug" | "already-valid-slug" |
  | "Trailing Punctuation..." | "trailing-punctuation" |
  | "---Leading Dashes" | "leading-dashes" |
```

### The Resolution Flow

**For "generating a URL from a title":**
```
Human writes:      "When generating a URL from a title"
                           ↓
Glossary match:    "generating a URL from a title"
                           ↓
Glossary means:    call: slugify({title}), input: test_data
                           ↓
System parses:     call: slugify({title})  → function to invoke
                   input: test_data        → where to get inputs
                           ↓
System finds:      Test Cases for this scenario (10 rows)
                           ↓
For each row:      {title}="Hello World", {expected}="hello-world"
                           ↓
System executes:   slugify("Hello World") → "hello-world"
                           ↓
Human wrote:       "→ URL is valid"
                           ↓
Glossary means:    check: invariant.url_slug
                           ↓
System runs:       6 invariant assertions ✓
```

**For "generating a URL from user input":**
```
Human writes:      "When generating a URL from user input"
                           ↓
Glossary match:    "generating a URL from user input"
                           ↓
Glossary means:    call: slugify({input}), input: corpus.strings
                           ↓
System parses:     call: slugify({input})  → function to invoke
                   input: corpus.strings   → use strings corpus
                           ↓
System expands:    ~20 corpus values (empty, unicode, long, special chars...)
                           ↓
For each value:    {input}="" → slugify("") → "post"
                   {input}="🎉" → slugify("🎉") → "post"
                   ...
                           ↓
Human wrote:       "→ URL is valid"
                           ↓
System runs:       6 assertions × 20 inputs = 120 checks ✓
```

### What This Runs

| Scenario | Test Cases | Checks Per Case | Total |
|----------|------------|-----------------|-------|
| Have Valid URLs | 10 examples | 1 match + 6 invariant | 70 |
| Are Predictable | 10 examples | 2 property | 20 |
| Handle Any Title | ~20 corpus | 6 invariant | 120 |
| **Total** | | | **~210 checks** |

### What The Human Sees

**Default output:**
```
✔️ Blog Posts    3 scenarios · 210 checks
```

**With -v:**
```
Feature: Blog Posts
  ✔️ Blog Posts Have Valid URLs (10 examples)
  ✔️ Blog Post URLs Are Predictable (10 examples)
  ✔️ Blog Posts Handle Any Title (20 corpus examples)
```

**The human never sees:**
- Function name `slugify`
- Test data tables
- Invariant definitions
- Individual assertions

### How Function Binding Works

The glossary term `calling slugify with {title}` tells the system to:

1. Find the function `slugify` in the linked .tnt file
2. Call it with the provided argument
3. Store the return value as `result`

```ntnt
// In blog.tnt

// @implements: feature.blog_posts
fn slugify(title) {
    // ...
}
```

---

## Part XIII: Success Criteria

### V1 Complete When:

- [x] Glossary can map terms to function calls (`call: {fn}({args})`)
- [x] Keyword syntax parsing (`call:`, `input:`, `check:`, `property:`)
- [x] Cycle detection in glossary term resolution
- [x] Invariants can be defined and referenced via glossary terms
- [x] Test Data section links to features via `for: feature.id`
- [x] Scenarios auto-run against linked test data
- [x] `input: corpus.strings` runs ~30 edge cases
- [x] `property: deterministic` calls function twice, compares
- [x] `property: idempotent` calls f(f(x)), compares to f(x)
- [x] `-v` and `-vv` output modes work
- [ ] Intent Studio shows expandable results (future enhancement)

### What A Real Unit Test Looks Like

**Human writes:**
```
Scenario: Blog Posts Have Valid URLs
  When generating a URL from a title
  → URL is valid
```

**System runs:**
- 10 explicit examples (from Test Data)
- ~20 corpus examples (from Corpus)
- 6 invariant checks per example
- = ~210 total checks

**Human sees:**
```
✔️ Blog Posts Have Valid URLs (30 examples)
```

### The Clean Separation

| Section | Contains | Who Manages |
|---------|----------|-------------|
| Features & Scenarios | Human-readable intent | Human confirms |
| `---` | | |
| Glossary | Term → technical mappings | Agent drafts |
| Invariants | Bundled assertions | Agent drafts |
| Test Data | Example inputs/outputs | Agent generates |

Everything above `---` is human-facing. Everything below is system-managed.

---

## Summary

**Before:** Feature → Scenarios (works for HTTP tests, not unit tests)

**After:** Feature → Scenarios (clean human language) + System-Managed sections

**Key additions:**
1. **Backwards-compatible glossary** - Term rewriting still works, keyword syntax adds unit testing
2. **Comprehensive test vocabulary** - Equality, bounds, stability, errors, round-trips, conservation, authorization
3. **Clean separation** - Human scenarios stay natural, technical syntax hidden in glossary

**The key insight:**

```
┌─────────────────────────────────────────────────────────────┐
│  HUMAN-FACING (above ---)                                   │
│                                                             │
│  Feature: Blog Posts                                        │
│    Scenario: Blog Posts Have Valid URLs                     │
│      description: "URLs are generated correctly from titles"│
│      When generating a URL from a title                     │
│      → URL is valid                                         │
│                                                             │
│    Scenario: Blog Posts Handle Any Title                    │
│      description: "Edge cases don't break URL generation"   │
│      When generating a URL from user input                  │
│      → URL is valid                                         │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│  SYSTEM-MANAGED (below ---)                                 │
│                                                             │
│  Glossary (both styles work):                               │
│    Term rewriting:  "page loads" → "status 200, returns HTML" │
│    Keyword syntax:  "from a title" → call: slugify({title})   │
│    "URL is valid"   → check: invariant.url_slug               │
│                                                             │
│  Invariants: bundled assertion checks                       │
│  Test Data: example inputs with expected outputs            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Two glossary styles:**
- **Term rewriting** (existing): `page loads successfully | status 200, returns HTML`
- **Keyword syntax** (new): `generating a URL | call: slugify({title}), input: test_data`

**Human writes natural language. Glossary resolves to system vocabulary. System executes.**

No `{variables}`, no `for each:`, no `for common cases:` in human-facing layer.

---

*"Clean scenarios above the line. Deterministic resolution below."*
