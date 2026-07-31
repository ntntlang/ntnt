# Intent Assertion Language (IAL) Reference

> **Auto-generated from [ial.toml](ial.toml)** - Do not edit directly.
>
> Last updated: v0.5.3

IAL is a term rewriting engine that translates natural language assertions into executable tests

## Table of Contents

- [Primitives](#primitives)
- [Check Operations](#check-operations)
- [Standard Terms](#standard-terms)
- [Context Paths](#context-paths)
- [Glossary System](#glossary-system)
- [Intent File Format](#intent-file-format)
- [Verification Reports](#verification-reports)
- [Commands](#commands)

---

## Primitives

IAL primitives are the leaf nodes of term resolution - they execute directly

| Primitive | Description | Context Sets | Status |
|-----------|-------------|---------------|--------|
| **Http** | Execute an HTTP request and capture response in context | `response.status`, `response.body`, `response.headers.*`, `response.time_ms` | Implemented |
| **Cli** | Execute a CLI command and capture output | `cli.exit_code`, `cli.stdout`, `cli.stderr` | Implemented |
| **CodeQuality** | Run lint/validation checks on source files | `code.quality.passed`, `code.quality.error_count`, `code.quality.warning_count`, `code.quality.errors` | Implemented |
| **ReadFile** | Read file contents into context | `file.content`, `file.exists` | Implemented |
| **FunctionCall** | Call an NTNT function for unit testing | `result` | Implemented |
| **PropertyCheck** | Verify a function property (deterministic, idempotent, round-trips) | `property.passed`, `property.failures` | Implemented |
| **Check** | Universal assertion - compare context value against expected |  | Implemented |

---

## Check Operations

Operations available for the Check primitive

### Equality

| Operation | Description |
|-----------|-------------|
| `Equals` | Exact equality |
| `NotEquals` | Not equal |

### Containment

| Operation | Description |
|-----------|-------------|
| `Contains` | String contains substring, or array contains element |
| `NotContains` | Does not contain |

### Pattern Matching

| Operation | Description |
|-----------|-------------|
| `EndsWith` | String ends with suffix |
| `Matches` | Regex pattern match |
| `StartsWith` | String starts with prefix |

### Existence

| Operation | Description |
|-----------|-------------|
| `Exists` | Value exists (not null/None) |
| `NotExists` | Value does not exist |

### Comparison

| Operation | Description |
|-----------|-------------|
| `GreaterThan` | Numeric greater than |
| `InRange` | Value within numeric range (e.g., 200-299) |
| `LessThan` | Numeric less than |

### Type Checks

| Operation | Description |
|-----------|-------------|
| `HasLength` | Check length equals value |
| `IsType` | Check value type |

---

## Standard Terms

Built-in terms that resolve to primitives. Use these in intent file assertions.

### HTTP Status

_HTTP response status assertions_

| Term | Resolves To |
|------|-------------|
| `status 2xx` | `Check(InRange, response.status, 200-299)` |
| `status 4xx` | `Check(InRange, response.status, 400-499)` |
| `status 5xx` | `Check(InRange, response.status, 500-599)` |
| `status: {code}` | `Check(Equals, response.status, {code})` |

### HTTP Body

_HTTP response body assertions_

| Term | Resolves To |
|------|-------------|
| `body contains {text}` | `Check(Contains, response.body, {text})` |
| `body has field {field}` | `Check(Contains, response.body, {field})` |
| `body is empty` | `Check(Equals, response.body, "")` |
| `body is not empty` | `Check(NotEquals, response.body, "")` |
| `body matches {pattern}` | `Check(Matches, response.body, {pattern})` |
| `body not contains {text}` | `Check(NotContains, response.body, {text})` |
| `response is valid JSON` | `Check(IsType, response.body, JSON)` |

### HTTP Headers

_HTTP header assertions_

| Term | Resolves To |
|------|-------------|
| `header {name} contains {value}` | `Check(Contains, response.headers.{name}, {value})` |
| `header {name} equals {value}` | `Check(Equals, response.headers.{name}, {value})` |
| `header {name} exists` | `Check(Exists, response.headers.{name})` |

### Content-Type Shortcuts

_Content-Type shorthand assertions_

| Term | Resolves To |
|------|-------------|
| `content-type is html` | `Check(Contains, response.headers.content-type, text/html)` |
| `content-type is json` | `Check(Contains, response.headers.content-type, application/json)` |
| `content-type is text` | `Check(Contains, response.headers.content-type, text/plain)` |

### Response Time

_Response time assertions_

| Term | Resolves To |
|------|-------------|
| `response time < {ms}ms` | `Check(LessThan, response.time_ms, {ms})` |
| `response time < {seconds}s` | `Check(LessThan, response.time_ms, {seconds}*1000)` |

### CLI

_CLI command assertions_

| Term | Resolves To |
|------|-------------|
| `exit code is {code}` | `Check(Equals, cli.exit_code, {code})` |
| `exits successfully` | `Check(Equals, cli.exit_code, 0)` |
| `stderr contains {text}` | `Check(Contains, cli.stderr, {text})` |
| `stderr is empty` | `Check(Equals, cli.stderr, "")` |
| `stdout contains {text}` | `Check(Contains, cli.stdout, {text})` |

### Code Quality

_Code quality/lint assertions_

| Term | Resolves To |
|------|-------------|
| `code is valid` | `Check(Equals, code.quality.passed, true)` |
| `code passes lint` | `Check(Equals, code.quality.passed, true)` |
| `no lint warnings` | `Check(Equals, code.quality.warning_count, 0)` |
| `no syntax errors` | `Check(Equals, code.quality.error_count, 0)` |

### Unit Test Results

_Function result assertions_

| Term | Resolves To |
|------|-------------|
| `result equals {expected}` | `Check(Equals, result, {expected})` |
| `result is {expected}` | `Check(Equals, result, {expected})` |

### Function Properties

_Function property assertions_

| Term | Resolves To |
|------|-------------|
| `is deterministic` | `PropertyCheck(_, Deterministic)` |
| `is idempotent` | `PropertyCheck(_, Idempotent)` |
| `is predictable` | `PropertyCheck(_, Deterministic)` |
| `is stable` | `PropertyCheck(_, Idempotent)` |

### String/Value Checks

_General string and value assertions_

| Term | Resolves To |
|------|-------------|
| `does not contain {text}` | `Check(NotContains, result, {text})` |
| `does not end with {suffix}` | `Check(Not(EndsWith), result, {suffix})` |
| `does not start with {prefix}` | `Check(Not(StartsWith), result, {prefix})` |
| `ends with {suffix}` | `Check(EndsWith, result, {suffix})` |
| `is lowercase` | `Check(Matches, result, ^[^A-Z]*$)` |
| `is non-empty` | `Check(NotEquals, result, "")` |
| `starts with {prefix}` | `Check(StartsWith, result, {prefix})` |
| `uses only {pattern}` | `Check(Matches, result, ^{pattern}+$)` |

### Bounds

_Numeric and length bound assertions_

| Term | Resolves To |
|------|-------------|
| `is at least {min}` | `Check(GreaterThan, result, {min}-1)` |
| `is at most {max}` | `Check(LessThan, result, {max}+1)` |
| `length is at most {max}` | `Check(LessThan, result.length, {max}+1)` |

---

## Context Paths

Context stores values during test execution, accessed via dot-notation paths

### response

| Path | Description |
|------|-------------|
| `response.body` | HTTP response body (string) |
| `response.headers.*` | HTTP headers (map, lowercase keys) |
| `response.status` | HTTP status code (number) |
| `response.time_ms` | Response time in milliseconds |

### cli

| Path | Description |
|------|-------------|
| `cli.exit_code` | Process exit code (number) |
| `cli.stderr` | Standard error (string) |
| `cli.stdout` | Standard output (string) |

### code_quality

| Path | Description |
|------|-------------|
| `code.quality.error_count` | Number of errors found |
| `code.quality.errors` | Array of error messages |
| `code.quality.passed` | Whether lint/validation passed (bool) |
| `code.quality.warning_count` | Number of warnings found |

### result

| Path | Description |
|------|-------------|
| `result` | Return value from function calls (any type) |

### file

| Path | Description |
|------|-------------|
| `file.content` | File contents (string) |
| `file.exists` | Whether file exists (bool) |

---

## Glossary System

Define domain-specific terms in your .intent file

### Format

```intent
## Glossary

| Term | Means |
|------|-------|
| term name | human-readable description |
| term with {param} | description using {param} |

```

### Parameters

Use {param} syntax for dynamic values in terms

Example: `| they see {text} | body contains {text} |`

### Keyword Syntax for Unit Tests

Glossary terms can use special keywords to invoke function calls for unit testing

```intent
## Glossary

| Term | Means |
|------|-------|
| rounding {value} | call: round_1dp({value}), source: utils.tnt |
| validating {email} | call: is_valid_email({email}), source: validators.tnt |
```

**Keywords:**

| Keyword | Description | Example |
|---------|-------------|---------|
| `call:` | Function to invoke | `call: my_function({param})` |
| `input:` | Test data reference for expansion | `input: test_data.examples` |
| `property:` | Property to verify | `property: deterministic` |
| `source:` | Source file containing the function (required) | `source: myfile.tnt` |

**Usage in Scenarios:**

```intent
Feature: Email Validation
  id: feature.unit_email

  Scenario: Valid email accepted
    When validating "user@example.com"
    → result is true

  Scenario: Invalid email rejected
    When validating "not-an-email"
    → result is false
```

The `source:` keyword is **required** - it tells IAL which .tnt file contains the function to call.

### Resolution Order

1. Check project glossary (defined in .intent file)
2. Check standard terms (built-in)
3. If not found, error with suggestions

---

## Intent File Format

Structure of .intent files

### Structure

```intent
# Project Name
# Run: ntnt intent check server.tnt

## Title
My Application

## Overview
Brief description of what this project does.

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the about page | /about |
| the page loads | status 200 |
| they see {text} | body contains {text} |
| they don't see {text} | body not contains {text} |

---

Feature: Home Page
  id: feature.home_page
  description: "Display welcome message on home page"

  Scenario: Visitor sees welcome
    When a user visits the home page
    → the page loads
    → they see "Welcome"

---

Feature: About Page
  id: feature.about_page
  description: "About page with company info"

  Scenario: Visitor sees about info
    When a user visits the about page
    → the page loads
    → they see "About Us"

---

Constraint: No Errors
  description: "Pages should not show error messages"
  applies_to: [feature.home_page, feature.about_page]

```

### File Linking

Intent files link to source files by filename

- `server.tnt ↔ server.intent`
- `crypto.tnt ↔ crypto.intent`

### Code Annotations

Link code to intent features with annotations

| Annotation | Purpose |
|------------|----------|
| `@implements: feature.X` | Links function to a feature |
| `@infrastructure` | Config/setup code |
| `@internal` | Internal implementation |
| `@supports: constraint.X` | Links function to a constraint |
| `@utility` | Marks helper functions |

---

## Output Symbols

Status icons in `ntnt intent check` output

| Symbol | Meaning |
|--------|--------|
| ✓ | Passed — scenario or assertion succeeded |
| ✗ | Failed — scenario or assertion failed |
| ⏭️ | Skipped — preconditions not met (Given clause failed) |
| ⧗ | Warning/Pending — scenario has unresolved outcomes (terms that couldn't be mapped to executable checks) or could not be resolved at all. Counts as failed in the summary. |

> **Tip:** If you see ⧗, check that all terms in your → outcome lines are defined in your Glossary or match a built-in assertion term. Unresolved outcomes often mean a term wasn't recognized — try rephrasing to use a built-in term like `body contains`, `body has field`, or `status 200`.

## POST Requests

Sending POST (and PUT/PATCH/DELETE) requests with bodies in intent scenarios

> Use the `body` keyword in glossary Means to separate the path from the JSON payload.

```yaml
## Glossary

| Term | Means |
|------|-------|
| a user creates a message with {body} | POST /messages body {body} |
| a user updates message {id} with {body} | PUT /messages/{id} body {body} |
| a user visits {path} | GET {path} |

---

Feature: Messages API
  id: feature.messages

  Scenario: Create a message
    When a user creates a message with {"text": "hello"}
    → status: 201
    → body has field "id"

  Scenario: List messages
    When a user visits /messages
    → status: 200
    → response is valid JSON
```

## Known Limitations

Known limitations and workarounds for assertion terms

| Assertion Term | Notes |
|----------------|-------|
| `response is valid JSON` | Uses regex matching on the response body. Works for single-line and multiline JSON. For more reliable JSON validation, prefer `content-type is json` (checks the Content-Type header) or `body has field "key"` (checks for a specific field). |

## Verification Reports

`ntnt intent check` and `ntnt intent coverage` render one versioned `RunReport` evidence ledger. Human totals, JSON totals, coverage, thresholds, and process exit status are projections of that same ledger.

- `ntnt intent check <file> --json` writes banner-free schema-v1 JSON to stdout. Compatibility live checks are profile-qualified as `legacy-live` and cannot be promoted to a global or `full` claim.
- `ntnt intent coverage <file> --json` reports implementation, executable, and current verified obligation coverage separately. Use `--min-implementation PERCENT`, `--min-executable PERCENT`, and `--min-verified PERCENT` to set explicit thresholds.
- A selected required binding satisfies an obligation only when it is bound, executable, current, assertion-resolved, passed, and contains at least one concrete evidence atom. Every selected required binding must satisfy this rule.
- Unbound, unsupported, blocked, skipped, stale, failed, flaky, cancelled, and zero-assertion (`no-result`) evidence fails strict verification. A fail-then-pass remains flaky with both attempts retained.
- Advisory and profile-excluded bindings remain visible but never satisfy required obligations. `@implements` remains linkage metadata, not execution evidence.

The committed JSON Schema is `tests/fixtures/verification/reports/schema-v1.json`.

JUnit output is intentionally deferred; future renderers must consume the same ledger.

## Commands

CLI commands for IAL

| Command | Description |
|---------|-------------|
| `ntnt intent check <file>` | Run tests and enforce every required binding through the shared evidence ledger; `--json` emits only schema-v1 JSON |
| `ntnt intent lint <file>` | Statically validate glossary and scenarios without running tests: unresolved terms and when-clauses (with did-you-mean suggestions), definition cycles, and orphan glossary entries. Exit 1 on unresolved terms or cycles; orphans are warnings only. --json for CI |
| `ntnt intent coverage <file>` | Render implementation, executable, and verified obligation coverage; supports `--json` and minimum coverage threshold flags |
| `ntnt intent init <file.intent> -o <output.tnt>` | Generate code scaffolding from intent file |
| `ntnt intent studio <file.intent>` | Visual preview with live test execution |

