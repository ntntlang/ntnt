# Intent Assertion Language (IAL) Specification

**Version:** 1.0.0-draft  
**Status:** Draft Specification  
**Last Updated:** January 2026

## Abstract

The Intent Assertion Language (IAL) is a natural language specification format that deterministically translates human-readable behavioral descriptions into executable tests. IAL bridges the gap between project management (what should happen) and engineering (how to verify it happened), enabling non-technical stakeholders to write specifications that directly become test suites.

**Key Innovation:** IAL uses a **glossary-based approach** where domain terms are defined once with precise meaning, then used naturally throughout scenarios. This achieves:

- Natural readability (PMs understand it)
- Deterministic execution (machines run it)
- No ambiguity (every term has one meaning)

## Table of Contents

1. [Introduction](#1-introduction)
2. [Design Principles](#2-design-principles)
3. [The Glossary System](#3-the-glossary-system)
4. [File Format](#4-file-format)
5. [Standard Glossary](#5-standard-glossary)
6. [Domain-Specific Glossaries](#6-domain-specific-glossaries)
7. [Writing Scenarios](#7-writing-scenarios)
8. [Progressive Disclosure](#8-progressive-disclosure)
9. [Behavioral Properties](#9-behavioral-properties)
10. [Components (Reusable Intent Blocks)](#10-components-reusable-intent-blocks)
11. [Implementation Notes](#11-implementation-notes)
12. [Examples](#12-examples)
13. [Roadmap Coverage](#13-roadmap-coverage)
14. [**Domain Extensions**](#14-domain-extensions)
    - [14.1 Temporal (Real-time, Games, Animation)](#141-temporal-extension)
    - [14.2 State Machines (Complex Workflows)](#142-state-machine-extension)
    - [14.3 Streaming (Continuous Data)](#143-streaming-extension)
    - [14.4 Spatial (3D, Games, AR/VR)](#144-spatial-extension)
    - [14.5 Probabilistic (AI/ML)](#145-probabilistic-extension)
    - [14.6 Mobile (iOS, Android)](#146-mobile-extension)
    - [14.7 Hardware (IoT, Sensors)](#147-hardware-extension)
    - [14.8 Extension Compatibility Matrix](#148-extension-compatibility-matrix)
    - [14.9 Creating Custom Extensions](#149-creating-custom-extensions)

- [Appendix A: Grammar BNF](#appendix-a-grammar-bnf)
- [Appendix B: Standard Term Index](#appendix-b-standard-term-index)
- [Appendix C: Changelog](#appendix-c-changelog)
- [Appendix D: References](#appendix-d-references)

---

## 1. Introduction

### 1.1 The Problem

Consider this typical test specification:

```yaml
# Technical (developer-only)
test:
  - request: POST /api/login with {"email": "test@test.com", "password": "correct"}
    assert:
      - status: 200
      - json path "token" exists
      - json path "token" matches /^[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+$/
```

A PM reads this and sees noise. They can't contribute. They can't verify it matches their intent.

### 1.2 The Solution

IAL with a glossary:

```intent
## Glossary

| Term | Means |
|------|-------|
| registered user | User exists in database with verified email |
| logs in | Submits email and password to authentication |
| valid credentials | Email exists AND password matches |
| authenticated | Receives session token, can access protected resources |

---

Feature: User Login

  Scenario: Successful authentication
    When a registered user logs in with valid credentials
    → they are authenticated
```

**A PM reads this and understands exactly what happens.**

Behind the scenes, each glossary term has a precise technical binding that the test runner executes.

### 1.3 Goals

1. **Human-First Readability**: Scenarios read like plain English requirements
2. **Deterministic Translation**: Every glossary term maps to exactly one test assertion
3. **Constrained Vocabulary**: Parser only accepts defined terms—no ambiguity
4. **Progressive Disclosure**: PM sees English, developer expands to see technical details
5. **Living Documentation**: Intent files serve as both specification and verified test suite

### 1.4 Non-Goals

- IAL is not a general-purpose programming language
- IAL does not replace unit tests for implementation details
- IAL does not handle performance/load testing (use dedicated tools)

---

## 2. Design Principles

### 2.1 Principle of Constrained Vocabulary

You can **only** use terms defined in the glossary. If you write something undefined, the parser tells you:

```
Error: Unknown term "logs in successfully" on line 15
Did you mean:
  - "logs in" + "→ they are authenticated"
  - Add "logs in successfully" to your glossary?
```

This makes intent files:

- ✅ **Natural** — reads like English
- ✅ **Deterministic** — every term has one meaning
- ✅ **Discoverable** — glossary teaches the vocabulary

### 2.2 Principle of Least Surprise

IAL expressions mean exactly what they appear to mean. "they see 'Welcome'" checks for "Welcome" in the output—nothing more, nothing less.

### 2.3 Principle of Composability

Complex behaviors are expressed by composing simple terms: "a registered user logs in with valid credentials" combines three glossary terms.

### 2.4 Principle of Domain Alignment

Each application domain (HTTP, CLI, database) has vocabulary natural to that domain. You extend the standard glossary with domain-specific terms for your project.

---

## 3. The Glossary System

### 3.1 How It Works

Every intent file has two layers:

| Layer        | Who Sees It             | Purpose                     |
| ------------ | ----------------------- | --------------------------- |
| **Terms**    | Everyone                | Natural language vocabulary |
| **Bindings** | Developers (expandable) | Technical definitions       |

### 3.2 Glossary Format

```intent
## Glossary

| Term | Means |
|------|-------|
| <natural phrase> | <what it means in plain English> |

## Glossary [Technical Bindings]

<term>:
  <binding-type>: <technical definition>
```

### 3.3 Binding Types

| Type           | Purpose               | Example                           |
| -------------- | --------------------- | --------------------------------- |
| `setup`        | Prepare test state    | `INSERT INTO users ...`           |
| `action`       | What the system does  | `POST /auth/login`                |
| `precondition` | Required state before | `user.exists == true`             |
| `assert`       | What to verify        | `status 200`, `body contains "X"` |

### 3.4 Example: Complete Glossary Entry

**What PM sees:**

```
| authenticated | Receives session token, can access protected resources |
```

**What developer can expand:**

```yaml
authenticated:
  assert:
    - status 200
    - json path "token" exists
    - json path "token" matches /^[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+$/
    - header "Set-Cookie" contains "session="
```

---

## 4. File Format

### 4.1 File Extension

Intent files use the `.intent` extension and match their source files:

```
server.tnt    ↔  server.intent
crypto.tnt    ↔  crypto.intent
cli.tnt       ↔  cli.intent
```

### 4.2 Document Structure

```intent
# Project Title
# Brief description
# Run: ntnt intent check <file>.tnt

## Overview
Extended description of the project purpose.

## Glossary

| Term | Means |
|------|-------|
| term 1 | human-readable meaning |
| term 2 | human-readable meaning |

## Design
- Key design decisions

---

Feature: Feature Name
  id: feature.feature_id
  description: "Human-readable description"

  Scenario: Scenario name
    When <subject> <action> [with <context>]
    → <outcome>
    → <outcome>

---

Constraint: Constraint Name
  description: "Constraint description"
  applies_to: [feature.feature_id]
```

### 4.3 Scenario Syntax

```intent
Scenario: <name>
  When <subject> <action> [with <context>]
  → <outcome>
  → <outcome>
```

- **When**: The trigger—what happens to start the scenario
- **→**: Outcomes—what should be true after (use `->` as ASCII alternative)
- Subjects, actions, and outcomes must be glossary terms or standard terms

---

## 5. Standard Glossary

These terms are **built-in** and available without definition. They form the foundation for all intent files.

### 5.1 Universal Outcomes

| Term                    | Technical Translation                           |
| ----------------------- | ----------------------------------------------- |
| `succeeds`              | No error thrown, operation completes            |
| `fails`                 | Operation throws error or returns error status  |
| `fails with "$message"` | Error message contains `$message`               |
| `takes less than $time` | Completes within duration (e.g., `500ms`, `2s`) |
| `no error`              | Operation completed without throwing            |

### 5.2 HTTP Outcomes

| Term                               | Technical Translation                      |
| ---------------------------------- | ------------------------------------------ |
| `they see "$text"`                 | `body contains "$text"`                    |
| `they don't see "$text"`           | `body not contains "$text"`                |
| `page contains "$text"`            | `body contains "$text"`                    |
| `redirects to $path`               | `status 3xx` + `Location header == $path`  |
| `returns status $code`             | `response.status == $code`                 |
| `returns JSON`                     | `Content-Type contains "application/json"` |
| `returns HTML`                     | `Content-Type contains "text/html"`        |
| `response includes "$field"`       | JSON path `$field` exists                  |
| `"$field" is "$value"`             | JSON path `$field == $value`               |
| `header "$name" is "$value"`       | `headers[$name] == $value`                 |
| `header "$name" contains "$value"` | `headers[$name].contains($value)`          |

### 5.3 CLI Outcomes

| Term                      | Technical Translation     |
| ------------------------- | ------------------------- |
| `exits successfully`      | `exit_code == 0`          |
| `exits with error`        | `exit_code != 0`          |
| `exits with code $n`      | `exit_code == $n`         |
| `output shows "$text"`    | `stdout contains "$text"` |
| `output matches $pattern` | `stdout matches $pattern` |
| `error shows "$text"`     | `stderr contains "$text"` |
| `no error output`         | `stderr is empty`         |

### 5.4 File Outcomes

| Term                             | Technical Translation                     |
| -------------------------------- | ----------------------------------------- |
| `file "$path" exists`            | `fs.exists($path) == true`                |
| `file "$path" is created`        | `fs.exists($path) == true` (after action) |
| `file "$path" is deleted`        | `fs.exists($path) == false`               |
| `file "$path" contains "$text"`  | `fs.read($path).contains($text)`          |
| `file "$path" is empty`          | `fs.size($path) == 0`                     |
| `directory "$path" exists`       | `fs.isDirectory($path)`                   |
| `directory "$path" has $n files` | File count matches                        |

### 5.5 Database Outcomes

| Term                             | Technical Translation                        |
| -------------------------------- | -------------------------------------------- |
| `record is created`              | Row count increases by 1                     |
| `record is updated`              | Row exists with new values                   |
| `record is deleted`              | Row no longer exists                         |
| `row exists where $condition`    | `SELECT ... WHERE $condition` returns ≥1 row |
| `no row exists where $condition` | `SELECT ... WHERE $condition` returns 0 rows |
| `row count is $n`                | Result set has exactly $n rows               |
| `row count of "$table" is $n`    | Table has exactly $n rows                    |
| `row count increases by $n`      | $n more rows than before action              |
| `"$column" is "$value"`          | `result[0].$column == $value`                |

### 5.6 Event & Side Effect Outcomes

| Term                                    | Technical Translation               |
| --------------------------------------- | ----------------------------------- |
| `email is sent to "$address"`           | Email delivery triggered to address |
| `event "$name" is emitted`              | Event published to message bus      |
| `event "$name" has "$field" = "$value"` | Event payload field matches         |
| `no event "$name"`                      | Event was not published             |
| `"$message" is logged`                  | Log output contains message         |
| `no errors are logged`                  | No ERROR level log entries          |

### 5.7 Behavioral Properties

| Term                                     | Technical Translation                   |
| ---------------------------------------- | --------------------------------------- |
| `when repeated $n times, still $outcome` | Idempotency: run $n times, same result  |
| `when called simultaneously, $outcome`   | Thread safety: parallel execution works |
| `makes at most $n database queries`      | Query count ≤ $n                        |
| `makes no external calls`                | No HTTP/network requests made           |
| `data is unchanged`                      | No mutations to database/state          |

### 5.8 Timing Assertions

| Term                      | Technical Translation  |
| ------------------------- | ---------------------- |
| `responds in under $time` | Response time < $time  |
| `completes within $time`  | Total duration < $time |

---

## 6. Domain-Specific Glossaries

Each project extends the standard glossary with terms specific to its domain.

### 6.1 Authentication Domain Example

```intent
## Glossary

| Term | Means |
|------|-------|
| registered user | User exists in database with verified email |
| unregistered user | No account exists for this email |
| logs in | Submits email and password to /auth/login |
| valid credentials | Email exists AND password matches stored hash |
| invalid credentials | Email doesn't exist OR password doesn't match |
| authenticated | Receives valid session token |
| rejected | Receives error message, no session token |
| session expires | Token becomes invalid after timeout |
| locked out | Too many failed attempts, temporarily blocked |
```

**Bindings (collapsed by default):**

```yaml
## Glossary [Technical Bindings]

registered user:
  setup: |
    INSERT INTO users (email, password_hash, verified) 
    VALUES ($email, hash($password), true)

logs in:
  action: POST /auth/login
  body: { "email": "$email", "password": "$password" }

authenticated:
  assert:
    - status 200
    - json path "token" exists
    - json path "token" is valid JWT
    - header "Set-Cookie" contains "session="

rejected:
  assert:
    - status 401 or 403
    - json path "error" exists
    - json path "token" does not exist
```

### 6.2 E-Commerce Domain Example

```intent
## Glossary

| Term | Means |
|------|-------|
| customer | Authenticated user with valid session |
| guest | Unauthenticated visitor |
| cart | User's pending order items |
| empty cart | Cart with no items |
| adds to cart | Associates product with user's cart |
| removes from cart | Disassociates product from cart |
| checks out | Submits cart for payment processing |
| payment succeeds | Payment provider returns approval |
| payment fails | Payment provider returns decline |
| order is confirmed | Order record created, confirmation shown |
| order is cancelled | No order created, cart preserved |
```

### 6.3 CLI Tool Domain Example

```intent
## Glossary

| Term | Means |
|------|-------|
| user runs | Executes command from terminal |
| with flag $flag | Command includes the specified flag |
| with input file "$path" | Command receives file as input |
| in directory "$path" | Command runs in specified directory |
| help is shown | Usage information displayed |
| version is shown | Version number displayed |
| file is converted | Input file transformed to output format |
| progress is shown | Progress indicator displayed during operation |
```

---

## 7. Writing Scenarios

### 7.1 Scenario Structure

Every scenario follows a natural sentence structure:

```
When <who/what> <does something> [with/in <context>]
→ <expected outcome>
→ <expected outcome>
```

### 7.2 Examples by Domain

#### HTTP/Web Application

```intent
Scenario: Successful login
  When a registered user logs in with valid credentials
  → they are authenticated
  → they see "Welcome back"
  → redirects to /dashboard

Scenario: Failed login
  When a registered user logs in with invalid credentials
  → they are rejected with "Invalid credentials"
  → they don't see "dashboard"
```

#### CLI Application

```intent
Scenario: Convert CSV to JSON
  When user runs `convert data.csv --format json`
  → exits successfully
  → file "data.json" is created
  → file "data.json" contains "["

Scenario: Missing input file
  When user runs `convert missing.csv --format json`
  → exits with error
  → error shows "File not found"
```

#### Database Operation

```intent
Scenario: Create new user
  When inserting a new user with name "Alice"
  → record is created
  → row exists where name = "Alice"
  → no errors are logged

Scenario: Prevent duplicate email
  When inserting a user with an existing email
  → fails with "email already exists"
  → row count is unchanged
```

### 7.3 Compound Scenarios

Combine glossary terms naturally:

```intent
Scenario: Complete checkout flow
  When a customer with items in cart checks out and payment succeeds
  → order is confirmed
  → they see "Thank you for your order"
  → email is sent to customer
  → cart is empty

Scenario: Abandoned checkout
  When a customer with items in cart checks out and payment fails
  → order is cancelled
  → they see "Payment declined"
  → cart still has items
  → no email is sent
```

---

## 8. Progressive Disclosure

### 8.1 Two Views, One Truth

IAL supports progressive disclosure—PM view for stakeholders, dev view for engineers.

#### PM View (Default in Intent Studio)

```
Feature: User Authentication

✓ When a registered user logs in with valid credentials → they are authenticated
✓ When a registered user logs in with invalid credentials → they are rejected
✗ When a user is locked out → they cannot log in (FAILING)
```

#### Dev View (Expandable)

```
Feature: User Authentication

▼ When a registered user logs in with valid credentials → they are authenticated
  ├─ Setup: INSERT INTO users (email, password_hash) VALUES ('test@test.com', '$2b$...')
  ├─ Action: POST /auth/login {"email": "test@test.com", "password": "correct"}
  └─ Assert:
       ├─ status 200 ✓
       ├─ json path "token" exists ✓
       ├─ json path "token" matches /^[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+$/ ✓
       └─ header "Set-Cookie" contains "session=" ✓
```

### 8.2 Intent Studio Rendering

In Intent Studio, scenarios render as cards:

```
┌─────────────────────────────────────────────────────────────┐
│ ✓ Successful Login                                          │
│                                                             │
│   When a registered user logs in with valid credentials     │
│   → they are authenticated                                  │
│   → they see "Welcome back"                                 │
│                                                             │
│   [▶ Show Technical Details]                                │
└─────────────────────────────────────────────────────────────┘
```

Clicking "Show Technical Details" expands the bindings.

---

## 9. Behavioral Properties

### 9.1 Idempotency

Test that operations can be safely repeated:

```intent
Feature: Idempotent API

  Scenario: PUT is idempotent
    When a user updates their profile
    → when repeated 3 times, still succeeds
    → when repeated 3 times, data is unchanged
```

### 9.2 Thread Safety

Test concurrent access:

```intent
Feature: Concurrent Access

  Scenario: Simultaneous logins
    When two users log in simultaneously
    → both are authenticated
    → no errors are logged
```

### 9.3 Resource Constraints

Test efficiency bounds:

```intent
Feature: Efficient Queries

  Scenario: List users is efficient
    When requesting the user list
    → makes at most 2 database queries
    → responds in under 100ms
```

### 9.4 Side Effect Verification

Test what happens beyond the response:

```intent
Feature: User Registration

  Scenario: Welcome email sent
    When a new user registers
    → they are authenticated
    → email is sent to their address
    → event "user.registered" is emitted
    → "New user: test@test.com" is logged
```

---

## 10. Components (Reusable Intent Blocks)

Components are **reusable behavioral building blocks** that can be defined once and referenced throughout your intent files. When you verify a feature that uses a component, the component's assertions cascade—verifying both the feature and the component's fundamental behavior.

### 10.1 The Problem Components Solve

Without components, you'd repeat the same assertions everywhere:

```intent
# Without components - repetitive and error-prone
Feature: Login
  Scenario: Wrong password
    When user enters wrong password
    → error overlay is visible
    → error icon is shown
    → "Wrong password" is displayed
    → close button exists
    → clicking close dismisses overlay

Feature: Registration
  Scenario: Email taken
    When user registers with existing email
    → error overlay is visible       # Repeated!
    → error icon is shown            # Repeated!
    → "Email already exists" is displayed
    → close button exists            # Repeated!
    → clicking close dismisses overlay  # Repeated!
```

With components, you define the pattern once:

```intent
# With components - DRY and maintainable
Feature: Login
  Scenario: Wrong password
    When user enters wrong password
    → they see error popup with "Wrong password"

Feature: Registration
  Scenario: Email taken
    When user registers with existing email
    → they see error popup with "Email already exists"
```

### 10.2 Defining Components

Components are defined in a `## Components` section with their own scenarios:

```intent
## Components

Component: Error Popup
  id: component.error_popup
  parameters: [message]
  description: "Modal overlay displaying an error message with dismiss functionality"

  Inherent Behavior:
    → error overlay is visible
    → error icon is shown
    → "$message" is displayed
    → close button exists

  Scenario: Dismiss via close button
    When user clicks close button
    → overlay disappears
    → focus returns to previous element

  Scenario: Dismiss via outside click
    When user clicks outside the popup
    → overlay disappears

  Scenario: Dismiss via escape key
    When user presses Escape
    → overlay disappears

  Scenario: Accessibility
    → popup has role="alertdialog"
    → popup has aria-modal="true"
    → close button is keyboard focusable
```

### 10.3 Component Structure

| Section             | Purpose                                                        |
| ------------------- | -------------------------------------------------------------- |
| `id`                | **Unique identifier** - the deterministic link used everywhere |
| `parameters`        | Variables that customize the component (e.g., `message`)       |
| `description`       | Human-readable explanation                                     |
| `Inherent Behavior` | Assertions that ALWAYS apply when component is referenced      |
| `Scenario`          | Additional behaviors that can be individually tested           |

### 10.4 The Deterministic Link Chain

The component `id` creates an unbreakable chain connecting everything:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    COMPONENT ID: component.error_popup                       │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  1. DEFINITION (what it is)                                                 │
│     ┌─────────────────────────────────────────────────────────┐            │
│     │ Component: Error Popup                                   │            │
│     │   id: component.error_popup  ◄── THE ID                 │            │
│     │   Inherent Behavior:                                     │            │
│     │     → error overlay is visible                          │            │
│     │     → "$message" is displayed                           │            │
│     └─────────────────────────────────────────────────────────┘            │
│                              │                                              │
│                              ▼                                              │
│  2. GLOSSARY (natural language binding)                                     │
│     ┌─────────────────────────────────────────────────────────┐            │
│     │ | error popup with "$msg" | → component.error_popup(...) │◄── LINKS  │
│     └─────────────────────────────────────────────────────────┘            │
│                              │                                              │
│                              ▼                                              │
│  3. USAGE (in feature scenarios)                                           │
│     ┌─────────────────────────────────────────────────────────┐            │
│     │ Scenario: Wrong password                                 │            │
│     │   → they see error popup with "Invalid credentials"     │◄── NATURAL │
│     └─────────────────────────────────────────────────────────┘            │
│                              │                                              │
│                              ▼                                              │
│  4. IMPLEMENTATION (code annotation)                                        │
│     ┌─────────────────────────────────────────────────────────┐            │
│     │ // @implements: component.error_popup                    │◄── CODE   │
│     │ fn render_error_popup(message: String) { ... }          │            │
│     └─────────────────────────────────────────────────────────┘            │
│                              │                                              │
│                              ▼                                              │
│  5. VERIFICATION (cascading test execution)                                │
│     ┌─────────────────────────────────────────────────────────┐            │
│     │ ✓ component.error_popup verified                        │◄── PROOF  │
│     │   ├─ ✓ error overlay is visible                         │            │
│     │   └─ ✓ "Invalid credentials" is displayed               │            │
│     └─────────────────────────────────────────────────────────┘            │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Every reference uses the same ID.** This means:

- Rename `component.error_popup` → all references update or break loudly
- Change the component's assertions → all features using it re-verify
- Search for `component.error_popup` → find every usage, definition, implementation

### 10.5 Referencing Components

Reference components in your feature scenarios using the glossary term pattern:

```intent
## Glossary

| Term | Means |
|------|-------|
| error popup with "$message" | → component.error_popup(message: $message) |
| success toast with "$message" | → component.success_toast(message: $message) |
| confirmation dialog asking "$question" | → component.confirm_dialog(question: $question) |
```

Then use naturally:

```intent
Feature: User Authentication

  Scenario: Invalid credentials
    When user logs in with wrong password
    → they see error popup with "Invalid credentials"

  Scenario: Account locked
    When user fails login 5 times
    → they see error popup with "Account temporarily locked"
```

### 10.6 Cascading Verification

When the test runner encounters a component reference, it **cascades** the verification:

```
Verifying: Feature "User Authentication"
  Scenario: Invalid credentials

    ✓ When user logs in with wrong password

    → they see error popup with "Invalid credentials"
      ├─ [Component: Error Popup] Inherent Behavior
      │   ├─ ✓ error overlay is visible
      │   ├─ ✓ error icon is shown
      │   ├─ ✓ "Invalid credentials" is displayed
      │   └─ ✓ close button exists
      └─ ✓ Component verified (4/4 assertions)

    Result: PASS (5 assertions)
```

### 10.7 Component Inheritance

Components can extend other components:

```intent
Component: Critical Error Popup
  id: component.critical_error
  extends: component.error_popup
  parameters: [message]
  description: "Error popup with additional severity styling"

  Inherent Behavior:
    → all of component.error_popup
    → popup has class "critical"
    → icon is red warning triangle
    → cannot be dismissed by clicking outside
```

When you reference `critical error popup`, it verifies:

1. All inherited `error_popup` assertions
2. Plus the additional `critical_error` assertions

### 10.8 Component Libraries

For large applications, define components in separate files and import them:

```intent
# components/ui.intent

## Components

Component: Error Popup
  id: component.error_popup
  ...

Component: Success Toast
  id: component.success_toast
  ...

Component: Confirmation Dialog
  id: component.confirm_dialog
  ...
```

```intent
# auth.intent

@import "components/ui.intent"

## Glossary

| Term | Means |
|------|-------|
| error popup with "$message" | → component.error_popup(message: $message) |

---

Feature: User Login
  Scenario: Wrong password
    When user enters wrong password
    → they see error popup with "Wrong password"
```

### 10.9 Built-in Component Templates

IAL provides common component templates you can extend:

| Template          | Use Case                    |
| ----------------- | --------------------------- |
| `@template/modal` | Base modal dialog behavior  |
| `@template/toast` | Notification toast behavior |
| `@template/form`  | Form validation patterns    |
| `@template/list`  | Paginated list behavior     |
| `@template/table` | Sortable/filterable table   |

```intent
Component: User Settings Modal
  id: component.settings_modal
  extends: @template/modal

  Inherent Behavior:
    → shows user's current settings
    → has "Save" and "Cancel" buttons
```

### 10.10 Component Parameters

Parameters can have types and defaults:

```intent
Component: Pagination
  id: component.pagination
  parameters:
    - page_size: Int = 10
    - current_page: Int = 1
    - total_items: Int

  Inherent Behavior:
    → shows "$total_items" total results
    → shows items (($current_page - 1) * $page_size + 1) to ($current_page * $page_size)
    → "Previous" button is disabled when $current_page == 1
    → "Next" button is disabled when on last page
```

Usage:

```intent
→ they see pagination with total_items: 100, page_size: 25
```

### 10.11 Testing Components in Isolation

Components can be tested independently:

```bash
$ ntnt intent check components/ui.intent --component error_popup

Component: Error Popup
  ✓ Inherent Behavior (4/4 assertions)
  ✓ Scenario: Dismiss via close button
  ✓ Scenario: Dismiss via outside click
  ✓ Scenario: Dismiss via escape key
  ✓ Scenario: Accessibility

Result: PASS (12 assertions across 5 scenarios)
```

### 10.12 Component Versioning

When components change, version them to avoid breaking existing features:

```intent
Component: Error Popup
  id: component.error_popup
  version: 2.0
  deprecated_versions: [1.0]
  migration_guide: |
    v2.0 adds accessibility requirements.
    Update assertions to include ARIA attributes.
```

### 10.13 Real-World Component Example

Here's a complete example showing components in action:

```intent
# E-Commerce Application

## Components

Component: Product Card
  id: component.product_card
  parameters: [product_name, price, image_url]
  description: "Displays a product with image, name, price, and add-to-cart action"

  Inherent Behavior:
    → image from "$image_url" is displayed
    → "$product_name" is shown
    → "$price" is formatted as currency
    → "Add to Cart" button exists

  Scenario: Add to cart
    When user clicks "Add to Cart"
    → item is added to cart
    → they see success toast with "Added to cart"
    → cart count increases by 1

  Scenario: Out of stock
    Given product is out of stock
    → "Add to Cart" button is disabled
    → "Out of Stock" label is shown

Component: Success Toast
  id: component.success_toast
  parameters: [message]

  Inherent Behavior:
    → toast appears in top-right corner
    → toast has green background
    → "$message" is displayed
    → toast auto-dismisses after 3 seconds

---

## Glossary

| Term | Means |
|------|-------|
| product card for "$name" at "$price" | → component.product_card(product_name: $name, price: $price) |
| success toast with "$message" | → component.success_toast(message: $message) |

---

Feature: Product Listing
  id: feature.product_listing

  Scenario: Display products
    When user visits the product page
    → they see product card for "Wireless Headphones" at "$79.99"
    → they see product card for "USB Cable" at "$12.99"

  Scenario: Add product to cart
    When user clicks "Add to Cart" on "Wireless Headphones"
    → they see success toast with "Added to cart"
    → cart shows 1 item
```

When this runs, the test verifies both the feature AND all referenced component behaviors cascade.

---

## 11. Implementation Notes

### 10.1 Parser Requirements

The IAL parser MUST:

1. Accept only defined glossary terms and standard terms
2. Reject undefined terms with helpful suggestions
3. Expand glossary terms to technical bindings
4. Provide source mapping for error messages

### 10.2 Term Resolution Order

1. Check project glossary (defined in file)
2. Check standard glossary (built-in)
3. If not found → error with suggestions

### 10.3 Test Runner Requirements

The test runner MUST:

1. Execute setup bindings before test
2. Execute action bindings
3. Verify all assert bindings
4. Execute teardown bindings (cleanup)
5. Report pass/fail per assertion with clear messages

### 10.4 Coverage Tracking

Link scenarios to code with annotations:

```ntnt
// @implements: feature.user_login
fn login_handler(req) {
    // Implementation
}
```

Run `ntnt intent coverage` to see which features have implementations.

---

## 12. Examples

### 11.1 Complete Authentication Intent

```intent
# User Authentication
# Secure login and session management
# Run: ntnt intent check auth.tnt

## Overview
Users authenticate with email and password to receive a session token.
Sessions expire after 24 hours of inactivity.

## Glossary

| Term | Means |
|------|-------|
| registered user | User with verified account in database |
| unregistered user | Email not in our system |
| logs in | Submits credentials to authentication endpoint |
| valid credentials | Correct email and password combination |
| invalid credentials | Wrong password or unknown email |
| authenticated | Receives valid session token |
| rejected | Shown error message, no token issued |
| locked out | Temporarily blocked after 5 failed attempts |

## Design
- Passwords are hashed with bcrypt
- Tokens are JWTs with 24-hour expiry
- Failed attempts tracked per IP and email

---

Feature: User Login
  id: feature.user_login
  description: "Users can securely log in to their accounts"

  Scenario: Successful login
    When a registered user logs in with valid credentials
    → they are authenticated
    → they see "Welcome back"

  Scenario: Wrong password
    When a registered user logs in with invalid credentials
    → they are rejected with "Invalid credentials"

  Scenario: Unknown email
    When an unregistered user logs in
    → they are rejected with "Account not found"

  Scenario: Account lockout
    When a registered user fails login 5 times
    → they are locked out
    → they see "Too many attempts"

---

Feature: Session Management
  id: feature.sessions
  description: "Sessions provide secure, temporary access"

  Scenario: Token in response
    When a user is authenticated
    → response includes "token"
    → "token" is valid JWT

  Scenario: Session cookie set
    When a user is authenticated
    → header "Set-Cookie" contains "session="
    → header "Set-Cookie" contains "HttpOnly"

---

Constraint: Response Time
  description: "Authentication must be fast"
  applies_to: [feature.user_login]

  Scenario: Login is responsive
    When any user logs in
    → responds in under 500ms
```

### 11.2 CLI Tool Intent

```intent
# File Converter
# Convert between data formats
# Run: ntnt intent check converter.tnt

## Overview
A command-line tool for converting CSV, JSON, and XML files.

## Glossary

| Term | Means |
|------|-------|
| user runs | Executes the converter command |
| valid CSV file | Well-formed CSV with headers |
| invalid CSV file | Malformed or corrupted CSV |
| converted file | Output file in target format |

---

Feature: CSV to JSON
  id: feature.csv_to_json
  description: "Convert CSV files to JSON format"

  Scenario: Successful conversion
    When user runs `convert data.csv --format json`
    → exits successfully
    → file "data.json" is created
    → file "data.json" contains "["

  Scenario: File not found
    When user runs `convert missing.csv --format json`
    → exits with error
    → error shows "File not found"

  Scenario: Invalid CSV
    When user runs `convert malformed.csv --format json`
    → exits with error
    → error shows "Invalid CSV format"

---

Feature: Help and Version
  id: feature.help
  description: "Show usage information"

  Scenario: Help flag
    When user runs `convert --help`
    → exits successfully
    → output shows "Usage:"
    → output shows "--format"

  Scenario: Version flag
    When user runs `convert --version`
    → exits successfully
    → output matches /\d+\.\d+\.\d+/
```

### 11.3 Database Application Intent

```intent
# User Management
# CRUD operations for user records
# Run: ntnt intent check users.tnt

## Overview
Database-backed user management with validation.

## Glossary

| Term | Means |
|------|-------|
| creating a user | Inserting a new user record |
| valid user data | Name, email, and age all provided and valid |
| invalid email | Email doesn't match expected format |
| duplicate email | Email already exists in database |
| user record | Row in the users table |

---

Feature: Create User
  id: feature.create_user
  description: "Add new users to the system"

  Scenario: Valid user creation
    When creating a user with valid user data
    → succeeds
    → record is created
    → row exists where email = "alice@test.com"

  Scenario: Duplicate email rejected
    When creating a user with duplicate email
    → fails with "email already exists"
    → row count is unchanged

  Scenario: Invalid email rejected
    When creating a user with invalid email
    → fails with "invalid email format"
    → no record is created

---

Feature: Query Users
  id: feature.query_users
  description: "Search and filter users"

  Scenario: Filter by age
    When querying users where age > 25
    → succeeds
    → all returned rows have age > 25

  Scenario: No results
    When querying users where email = "nonexistent@test.com"
    → succeeds
    → row count is 0
```

---

## 13. Roadmap Coverage

This section maps IAL to the NTNT roadmap to ensure comprehensive coverage.

### 13.1 Currently Implemented (Phase 6)

| Roadmap Item              | IAL Coverage                  | Status |
| ------------------------- | ----------------------------- | ------ |
| `status: 200`             | `returns status 200`          | ✅     |
| `body contains "X"`       | `they see "X"`                | ✅     |
| `body not contains "X"`   | `they don't see "X"`          | ✅     |
| `body matches r"..."`     | `output matches /pattern/`    | ✅     |
| `header "X" contains "Y"` | `header "X" contains "Y"`     | ✅     |
| `@implements` annotations | Supported via `id: feature.X` | ✅     |
| Intent Studio             | PM/Dev views supported        | ✅     |

### 13.2 Planned (Phase 6 Remaining)

| Roadmap Item            | IAL Term                  | Status       |
| ----------------------- | ------------------------- | ------------ |
| `response_time < 500ms` | `responds in under 500ms` | 📋 Specified |
| `json path "X" is "Y"`  | `"X" is "Y"`              | 📋 Specified |
| CLI exit codes          | `exits with code N`       | 📋 Specified |
| Database assertions     | `row exists where...`     | 📋 Specified |

### 13.3 Future (Phase 6.9+)

| Roadmap Item        | IAL Term                        | Status       |
| ------------------- | ------------------------------- | ------------ |
| Idempotency testing | `when repeated N times...`      | 📋 Specified |
| Thread safety       | `when called simultaneously...` | 📋 Specified |
| Query count limits  | `makes at most N queries`       | 📋 Specified |
| Email verification  | `email is sent to "X"`          | 📋 Specified |
| Event verification  | `event "X" is emitted`          | 📋 Specified |
| Log verification    | `"X" is logged`                 | 📋 Specified |

### 13.4 Browser/Visual Testing (Phase 6.10)

| Roadmap Item      | IAL Term                     | Status    |
| ----------------- | ---------------------------- | --------- |
| DOM assertions    | `element "X" is visible`     | 🔮 Future |
| Visual regression | `page looks like "baseline"` | 🔮 Future |
| LLM visual verify | `page looks correct`         | 🔮 Future |
| Accessibility     | `page is accessible`         | 🔮 Future |

---

## 14. Domain Extensions

IAL Core handles request/response patterns (HTTP, CLI, database queries). For specialized domains that operate differently, IAL provides **extensions**—additional standard glossaries and constructs that plug into the core system.

### Extension Usage

```intent
@use "ial/temporal"
@use "ial/spatial"
@use "ial/streaming"

# Now temporal, spatial, and streaming terms are available
```

Extensions follow all IAL principles:

- Glossary terms with deterministic bindings
- Component compatibility
- Progressive disclosure
- ID-based linking

---

### 14.1 Temporal Extension

**Use for:** Games, animations, real-time systems, time-sensitive workflows

The temporal extension adds assertions about **when** things happen, not just **what** happens.

#### 14.1.1 Temporal Triggers

| Term                           | Means                                           |
| ------------------------------ | ----------------------------------------------- |
| `after $duration`              | Trigger fires after delay (e.g., `after 500ms`) |
| `every $interval`              | Trigger fires repeatedly                        |
| `at frame $n`                  | Trigger at specific frame (games)               |
| `when $condition becomes true` | Trigger on state change                         |

#### 14.1.2 Temporal Outcomes

| Term                                     | Technical Translation                            |
| ---------------------------------------- | ------------------------------------------------ |
| `within $time, $assertion`               | Assert becomes true before deadline              |
| `after $delay, $assertion`               | Assert checked after delay                       |
| `continuously for $duration, $assertion` | Assert holds true for entire duration            |
| `eventually $assertion`                  | Assert becomes true at some point (with timeout) |
| `never $assertion`                       | Assert never becomes true during scenario        |
| `for at least $duration, $assertion`     | Assert holds for minimum time                    |
| `exactly $n times during $duration`      | Event occurs exactly N times                     |
| `at most $n times per $duration`         | Rate limiting check                              |

#### 14.1.3 Frame-Based Assertions (Games)

| Term                           | Technical Translation            |
| ------------------------------ | -------------------------------- |
| `every frame, $assertion`      | Assert on each render frame      |
| `for $n frames, $assertion`    | Assert holds for N frames        |
| `within $n frames, $assertion` | Assert true before N frames pass |
| `frame rate stays above $fps`  | Performance assertion            |
| `frame time stays below $ms`   | Performance assertion            |

#### 14.1.4 Temporal Examples

```intent
@use "ial/temporal"

Feature: Player Jump
  id: feature.player_jump

  Scenario: Jump arc
    When player presses jump button
    → within 50ms, player.velocity.y > 0
    → continuously for 400ms, player is airborne
    → after 500ms, player is grounded
    → never player.position.y < 0

  Scenario: Double jump window
    When player is airborne
    → for 200ms after apex, double jump is available
    → after 200ms past apex, double jump is unavailable

Feature: Cooldown System
  id: feature.cooldowns

  Scenario: Ability cooldown
    When player uses fireball ability
    → immediately, fireball is on cooldown
    → for 5 seconds, fireball button is disabled
    → after 5 seconds, fireball is available
    → exactly 0 times during cooldown, fireball can be cast

Feature: Animation Timing
  id: feature.animations

  Scenario: Attack animation
    When player attacks
    → within 2 frames, attack animation starts
    → for 12 frames, attack hitbox is active
    → after 20 frames, player can move again
```

---

### 14.2 State Machine Extension

**Use for:** Complex workflows, game states, order lifecycles, UI flows

The state machine extension adds formal state transition modeling.

#### 14.2.1 State Machine Definition

```intent
@use "ial/statemachine"

StateMachine: Order Lifecycle
  id: statemachine.order
  initial: pending

  States:
    - pending: "Order created, awaiting payment"
    - confirmed: "Payment received, preparing"
    - shipped: "In transit to customer"
    - delivered: "Successfully delivered"
    - cancelled: "Order cancelled"
    - refunded: "Payment returned to customer"

  Transitions:
    - from: pending
      to: confirmed
      when: payment succeeds

    - from: pending
      to: cancelled
      when: user cancels

    - from: pending
      to: cancelled
      when: payment fails
      after: 24 hours

    - from: confirmed
      to: shipped
      when: warehouse dispatches

    - from: confirmed
      to: cancelled
      when: admin cancels

    - from: shipped
      to: delivered
      when: delivery confirmed

    - from: delivered
      to: refunded
      when: refund approved
      within: 30 days of delivery

  Terminal States: [delivered, cancelled, refunded]

  Invariants:
    - once cancelled, cannot transition to any state
    - once delivered, can only transition to refunded
    - shipped orders cannot be cancelled
```

#### 14.2.2 State Machine Assertions

| Term                                      | Technical Translation           |
| ----------------------------------------- | ------------------------------- |
| `$entity is in state "$state"`            | Current state check             |
| `$entity transitions to "$state"`         | State change occurs             |
| `$entity can transition to "$state"`      | Transition is valid             |
| `$entity cannot transition to "$state"`   | Transition is invalid           |
| `transition from "$a" to "$b" is valid`   | Validates against state machine |
| `all transitions follow statemachine.$id` | Full compliance check           |

#### 14.2.3 State Machine Examples

```intent
@use "ial/statemachine"

Feature: Order Workflow
  id: feature.order_workflow
  follows: statemachine.order

  Scenario: Happy path
    Given order is in state "pending"
    When payment succeeds
    → order transitions to "confirmed"
    → order history shows "confirmed at {timestamp}"

    When warehouse dispatches
    → order transitions to "shipped"
    → tracking number is assigned

    When delivery confirmed
    → order transitions to "delivered"
    → order is in terminal state

  Scenario: Cannot skip states
    Given order is in state "pending"
    → order cannot transition to "shipped"
    → order cannot transition to "delivered"

  Scenario: Terminal state is final
    Given order is in state "cancelled"
    → order cannot transition to "confirmed"
    → order cannot transition to "shipped"
    → order is in terminal state

Feature: Game States
  id: feature.game_states
  follows: statemachine.game

  StateMachine: Game
    id: statemachine.game
    initial: menu

    States: [menu, loading, playing, paused, game_over]

    Transitions:
      - from: menu, to: loading, when: player starts game
      - from: loading, to: playing, when: level loaded
      - from: playing, to: paused, when: player pauses
      - from: paused, to: playing, when: player resumes
      - from: playing, to: game_over, when: player dies
      - from: game_over, to: menu, when: player exits
      - from: paused, to: menu, when: player quits

  Scenario: Pause during gameplay
    Given game is in state "playing"
    When player presses escape
    → game transitions to "paused"
    → game clock is stopped
    → pause menu is visible

    When player presses resume
    → game transitions to "playing"
    → game clock resumes from paused time
```

---

### 14.3 Streaming Extension

**Use for:** Kafka, WebSocket streams, real-time feeds, event sourcing

The streaming extension handles continuous data flows rather than discrete request/response.

#### 14.3.1 Stream Triggers

| Term                        | Means                     |
| --------------------------- | ------------------------- |
| `subscribing to $stream`    | Opens stream subscription |
| `publishing to $stream`     | Sends to stream           |
| `$stream receives $message` | Message arrives on stream |

#### 14.3.2 Stream Outcomes

| Term                                  | Technical Translation            |
| ------------------------------------- | -------------------------------- |
| `stream emits $n items`               | Exactly N items received         |
| `stream emits item within $time`      | At least one item before timeout |
| `stream emits item matching $pattern` | Item matches condition           |
| `all stream items satisfy $condition` | Every item passes                |
| `no stream items satisfy $condition`  | No items pass                    |
| `stream completes`                    | Stream closes normally           |
| `stream completes within $time`       | Closes before timeout            |
| `stream errors`                       | Stream fails                     |
| `stream errors with "$message"`       | Specific error                   |
| `stream throughput >= $rate`          | Items per second                 |
| `stream backpressure stays below $n`  | Queue size limit                 |
| `stream maintains order`              | Items arrive in send order       |
| `exactly once delivery`               | No duplicates, no drops          |
| `at least once delivery`              | May duplicate, no drops          |

#### 14.3.3 Stream Examples

```intent
@use "ial/streaming"

Feature: Real-time Stock Prices
  id: feature.stock_stream

  Scenario: Subscribe to price updates
    When subscribing to "prices/AAPL" stream
    → stream emits item within 1s
    → all stream items have "symbol" = "AAPL"
    → all stream items have "price" > 0
    → stream throughput >= 1 item/second

  Scenario: Handle market close
    Given subscribed to "prices/AAPL" stream
    When market closes
    → stream completes within 5s
    → final item has "type" = "market_closed"

Feature: Chat Messages
  id: feature.chat_stream

  Scenario: Message ordering
    When user A sends "first" then "second" then "third"
    → stream maintains order
    → items arrive as ["first", "second", "third"]

  Scenario: Typing indicators
    When user starts typing
    → within 100ms, stream emits typing indicator
    → typing indicator has "user" and "timestamp"

    When user stops typing for 3s
    → stream emits typing stopped indicator

Feature: Event Sourcing
  id: feature.event_sourcing

  Scenario: Replay events
    Given event stream has 1000 historical events
    When replaying from beginning
    → stream emits 1000 items
    → all stream items have monotonic "sequence_number"
    → stream completes
    → final state matches current state

Feature: Kafka Consumer
  id: feature.kafka

  Scenario: Consumer group rebalancing
    Given 3 consumers in group "orders"
    When consumer 2 disconnects
    → within 30s, partitions are rebalanced
    → exactly once delivery is maintained
    → no messages are lost
    → consumer 1 and 3 handle increased load
```

---

### 14.4 Spatial Extension

**Use for:** Games, 3D applications, AR/VR, maps, physics simulations

The spatial extension adds assertions about position, distance, collision, and visibility.

#### 14.4.1 Position Assertions

| Term                           | Technical Translation                |
| ------------------------------ | ------------------------------------ |
| `$object is at ($x, $y)`       | 2D position check                    |
| `$object is at ($x, $y, $z)`   | 3D position check                    |
| `$object is at position $name` | Named position (e.g., "spawn point") |
| `$object.x is $value`          | Single axis check                    |
| `$object is within $bounds`    | Bounding box check                   |
| `$object is inside $region`    | Named region check                   |
| `$object is outside $region`   | Not in region                        |

#### 14.4.2 Distance & Relationship Assertions

| Term                               | Technical Translation           |
| ---------------------------------- | ------------------------------- |
| `$a is within $distance of $b`     | Distance check                  |
| `$a is at least $distance from $b` | Minimum distance                |
| `$a is exactly $distance from $b`  | Exact distance (with tolerance) |
| `$a is above $b`                   | Y position comparison           |
| `$a is below $b`                   | Y position comparison           |
| `$a is left of $b`                 | X position comparison           |
| `$a is right of $b`                | X position comparison           |
| `$a is in front of $b`             | Z or facing direction           |
| `$a is behind $b`                  | Opposite of facing              |
| `$a is facing $b`                  | Rotation/direction check        |
| `$a is facing direction $angle`    | Absolute rotation               |

#### 14.4.3 Collision & Physics Assertions

| Term                                  | Technical Translation      |
| ------------------------------------- | -------------------------- |
| `$a collides with $b`                 | Physics collision detected |
| `$a does not collide with $b`         | No collision               |
| `$a overlaps $b`                      | Bounding volumes intersect |
| `$object is grounded`                 | Touching ground/floor      |
| `$object is airborne`                 | Not grounded               |
| `$object is moving`                   | Velocity > 0               |
| `$object is stationary`               | Velocity = 0               |
| `$object velocity is ($vx, $vy, $vz)` | Exact velocity             |
| `$object speed is $value`             | Velocity magnitude         |
| `$object is falling`                  | Negative Y velocity        |
| `$object is rising`                   | Positive Y velocity        |

#### 14.4.4 Visibility & Rendering Assertions

| Term                            | Technical Translation   |
| ------------------------------- | ----------------------- |
| `$object is visible`            | Rendered and not culled |
| `$object is hidden`             | Not rendered            |
| `$object is visible to camera`  | In view frustum         |
| `$object is visible to $other`  | Line of sight check     |
| `$object is occluded by $other` | Blocked from view       |
| `$object is on screen`          | Within screen bounds    |
| `$object is off screen`         | Outside screen bounds   |
| `$object is in layer "$layer"`  | Render layer check      |

#### 14.4.5 Spatial Examples

```intent
@use "ial/spatial"
@use "ial/temporal"

Feature: Player Movement
  id: feature.player_movement

  Scenario: Walking
    Given player is at (0, 0, 0)
    When player moves forward for 1 second
    → player is at least 5 units from (0, 0, 0)
    → player is grounded
    → player is facing direction 0°

  Scenario: Jumping
    Given player is grounded
    When player jumps
    → within 100ms, player is airborne
    → player.y > 0
    → continuously for 300ms, player is rising
    → eventually player is falling
    → eventually player is grounded

Feature: Enemy AI
  id: feature.enemy_ai

  Scenario: Detection range
    Given enemy is at (10, 0, 0)
    And player is at (0, 0, 0)
    → player is within 15 units of enemy
    → enemy detects player

    When player moves to (20, 0, 0)
    → player is at least 10 units from enemy
    → enemy loses sight of player

  Scenario: Chasing
    Given enemy is chasing player
    → continuously, enemy is facing player
    → continuously, enemy is moving
    → distance between enemy and player decreases

Feature: Collision Detection
  id: feature.collision

  Scenario: Bullet hits enemy
    Given bullet is moving toward enemy
    When bullet collides with enemy
    → enemy takes damage
    → bullet is destroyed
    → impact particle is spawned at collision point

  Scenario: Wall collision
    Given player is moving toward wall
    When player collides with wall
    → player is stationary
    → player does not overlap wall
    → player is within 0.1 units of wall

Feature: Camera System
  id: feature.camera

  Scenario: Player always visible
    → continuously, player is visible to camera
    → player is on screen
    → player is not occluded by environment

  Scenario: Zoom out in combat
    Given enemy is within 20 units of player
    → camera distance increases
    → both player and enemy are on screen
```

---

### 14.5 Probabilistic Extension

**Use for:** AI/ML models, randomized systems, statistical testing, A/B tests

The probabilistic extension handles non-deterministic outcomes with statistical assertions.

#### 14.5.1 Statistical Outcomes

| Term                                                     | Technical Translation                |
| -------------------------------------------------------- | ------------------------------------ |
| `succeeds at least $percent% of the time`                | P(success) ≥ threshold (over N runs) |
| `fails at most $percent% of the time`                    | P(failure) ≤ threshold               |
| `result is "$value" approximately $percent% of the time` | Distribution check                   |
| `average $metric is within $tolerance of $expected`      | Mean check                           |
| `$metric has standard deviation < $value`                | Variance check                       |
| `distribution matches $distribution`                     | Statistical distribution test        |
| `results are uniformly distributed`                      | Uniform distribution check           |

#### 14.5.2 ML Model Assertions

| Term                                                    | Technical Translation     |
| ------------------------------------------------------- | ------------------------- |
| `accuracy is at least $percent%`                        | Model accuracy metric     |
| `precision is at least $percent%`                       | Precision metric          |
| `recall is at least $percent%`                          | Recall metric             |
| `F1 score is at least $value`                           | F1 metric                 |
| `AUC is at least $value`                                | Area under ROC curve      |
| `confidence score > $threshold`                         | Model certainty           |
| `prediction is one of $options`                         | Valid output classes      |
| `prediction matches expected for $percent% of test set` | Test set accuracy         |
| `model latency < $time`                                 | Inference speed           |
| `model size < $size`                                    | Memory/storage constraint |

#### 14.5.3 Fairness & Bias Assertions

| Term                                             | Technical Translation                   |
| ------------------------------------------------ | --------------------------------------- |
| `results are independent of "$attribute"`        | No correlation with protected attribute |
| `accuracy is similar across "$attribute" groups` | Group fairness                          |
| `false positive rate is similar across groups`   | Equalized odds                          |
| `no demographic has accuracy below $percent%`    | Minimum group performance               |

#### 14.5.4 Probabilistic Examples

```intent
@use "ial/probabilistic"

Feature: Image Classification
  id: feature.image_classification

  Scenario: Cat detection accuracy
    Given test set of 1000 cat images
    When classifying all images
    → accuracy is at least 95%
    → precision is at least 90%
    → recall is at least 92%
    → model latency < 100ms per image

  Scenario: Confidence thresholds
    When classifying a clear cat image
    → confidence score > 0.9
    → prediction is "cat"

    When classifying ambiguous image
    → confidence score < 0.7
    → prediction is one of ["cat", "dog", "unknown"]

  Scenario: Unknown handling
    When classifying random noise image
    → prediction is "unknown" or confidence score < 0.5

Feature: Random Loot System
  id: feature.loot

  Scenario: Rarity distribution
    When opening 10000 loot boxes
    → result is "common" approximately 70% of the time
    → result is "rare" approximately 25% of the time
    → result is "legendary" approximately 5% of the time
    → results are within 2% of expected distribution

  Scenario: Pity system
    When opening 100 boxes without legendary
    → next box has legendary chance > 50%
    → within 110 boxes, legendary is guaranteed

Feature: A/B Test
  id: feature.ab_test

  Scenario: Conversion rate comparison
    Given users randomly assigned to A or B
    → assignment is uniformly distributed
    → average conversion is within 0.5% between groups (baseline)

    When measuring after 10000 users
    → difference is statistically significant (p < 0.05) or not
    → confidence interval is computed

Feature: Fraud Detection
  id: feature.fraud_detection

  Scenario: Model performance
    Given test set with known fraud cases
    → accuracy is at least 99%
    → false positive rate < 1%
    → false negative rate < 0.1%
    → model latency < 50ms

  Scenario: Fairness across demographics
    → accuracy is similar across "age" groups
    → accuracy is similar across "region" groups
    → no demographic has accuracy below 97%
    → false positive rate is similar across groups

Feature: Recommendation Engine
  id: feature.recommendations

  Scenario: Relevance
    When recommending products for user
    → at least 3 of top 5 recommendations are relevant
    → succeeds at least 80% of the time across users

  Scenario: Diversity
    → recommendations include at least 3 categories
    → no single category exceeds 50% of recommendations
```

---

### 14.6 Mobile Extension

**Use for:** iOS, Android, React Native, Flutter applications

The mobile extension adds gestures, device features, and lifecycle management.

#### 14.6.1 Gesture Triggers

| Term                                 | Means                              |
| ------------------------------------ | ---------------------------------- |
| `user taps $element`                 | Single tap                         |
| `user double-taps $element`          | Double tap                         |
| `user long-presses $element`         | Long press (>500ms)                |
| `user swipes $direction`             | Swipe gesture (left/right/up/down) |
| `user swipes $direction on $element` | Swipe on specific element          |
| `user pinches $direction`            | Pinch in/out (zoom)                |
| `user rotates $direction`            | Two-finger rotation                |
| `user drags $element to $location`   | Drag and drop                      |
| `user pulls to refresh`              | Pull-to-refresh gesture            |

#### 14.6.2 Device State Triggers

| Term                             | Means                     |
| -------------------------------- | ------------------------- |
| `device rotates to $orientation` | Portrait/landscape change |
| `app goes to background`         | Home button pressed       |
| `app returns to foreground`      | App resumed               |
| `app receives push notification` | Remote notification       |
| `device loses connectivity`      | Network disconnected      |
| `device regains connectivity`    | Network reconnected       |
| `device battery is low`          | Battery < 20%             |
| `device storage is low`          | Storage warning           |

#### 14.6.3 Mobile Outcomes

| Term                                         | Technical Translation      |
| -------------------------------------------- | -------------------------- |
| `haptic feedback is triggered`               | Vibration/haptic           |
| `keyboard is shown`                          | Soft keyboard visible      |
| `keyboard is dismissed`                      | Soft keyboard hidden       |
| `screen scrolls to $element`                 | Element scrolled into view |
| `pull-to-refresh indicator is shown`         | Refresh UI visible         |
| `navigation goes to $screen`                 | Screen navigation          |
| `back navigation goes to $screen`            | Back button behavior       |
| `deep link opens $screen`                    | URL scheme handling        |
| `share sheet is shown`                       | Native share dialog        |
| `permission is requested for "$permission"`  | Permission dialog          |
| `permission "$permission" is granted/denied` | Permission result          |

#### 14.6.4 Mobile Layout Assertions

| Term                               | Technical Translation         |
| ---------------------------------- | ----------------------------- |
| `$element is tappable`             | Touch target ≥ 44pt           |
| `$element fits on screen`          | No horizontal scroll needed   |
| `content is readable`              | Font size ≥ minimum           |
| `layout adapts to $orientation`    | Responsive layout             |
| `safe areas are respected`         | Notch/home indicator handling |
| `keyboard does not cover $element` | Keyboard avoidance            |

#### 14.6.5 Mobile Examples

```intent
@use "ial/mobile"

Feature: Swipe Navigation
  id: feature.swipe_navigation

  Scenario: Swipe between tabs
    Given user is on Home tab
    When user swipes left
    → navigation goes to Search tab
    → haptic feedback is triggered

    When user swipes right
    → navigation goes to Home tab

Feature: Pull to Refresh
  id: feature.pull_to_refresh

  Scenario: Refresh content
    When user pulls to refresh
    → pull-to-refresh indicator is shown
    → data is fetched from server
    → eventually, pull-to-refresh indicator is hidden
    → new content is displayed

Feature: App Lifecycle
  id: feature.app_lifecycle

  Scenario: Background and resume
    Given user is editing a form
    When app goes to background
    → form data is saved locally

    When app returns to foreground
    → form data is restored
    → user can continue editing

  Scenario: Handle termination
    Given user has unsaved changes
    When app is terminated by system
    → changes are saved to draft

    When app is relaunched
    → draft recovery prompt is shown

Feature: Offline Mode
  id: feature.offline

  Scenario: Graceful offline handling
    When device loses connectivity
    → offline banner is shown
    → cached content is still accessible
    → write operations are queued

    When device regains connectivity
    → offline banner is hidden
    → queued operations are synced
    → they see "Synced successfully"

Feature: Device Orientation
  id: feature.orientation

  Scenario: Landscape video
    Given user is watching video
    When device rotates to landscape
    → video enters fullscreen
    → controls adapt to landscape
    → safe areas are respected

    When device rotates to portrait
    → video exits fullscreen
    → layout adapts to portrait

Feature: Permissions
  id: feature.permissions

  Scenario: Camera permission flow
    When user taps "Take Photo"
    → permission is requested for "camera"

    When permission "camera" is granted
    → camera preview is shown

    When permission "camera" is denied
    → they see error popup with "Camera access required"
    → "Open Settings" button is shown
```

---

### 14.7 Hardware Extension

**Use for:** IoT devices, sensors, embedded systems, robotics

The hardware extension adds assertions for physical devices and sensors.

#### 14.7.1 Sensor Triggers

| Term                                          | Means                 |
| --------------------------------------------- | --------------------- |
| `sensor "$name" reads $value`                 | Sensor input          |
| `sensor "$name" reads above/below $threshold` | Threshold trigger     |
| `button "$name" is pressed`                   | Physical button input |
| `button "$name" is released`                  | Button release        |
| `switch "$name" is on/off`                    | Toggle state          |

#### 14.7.2 Actuator Outcomes

| Term                              | Technical Translation |
| --------------------------------- | --------------------- |
| `LED "$name" is on/off`           | LED state             |
| `LED "$name" is color "$color"`   | RGB LED state         |
| `motor "$name" is running`        | Motor active          |
| `motor "$name" speed is $value`   | Motor speed           |
| `servo "$name" angle is $degrees` | Servo position        |
| `relay "$name" is open/closed`    | Relay state           |
| `display shows "$text"`           | LCD/display output    |
| `buzzer sounds at $frequency`     | Audio output          |

#### 14.7.3 Communication Outcomes

| Term                                | Technical Translation             |
| ----------------------------------- | --------------------------------- |
| `message is sent via $protocol`     | Protocol output (MQTT, BLE, etc.) |
| `message is received via $protocol` | Protocol input                    |
| `device is connected via $protocol` | Connection state                  |
| `device advertises via BLE`         | Bluetooth advertising             |
| `device responds to ping`           | Network reachability              |

#### 14.7.4 Hardware Examples

```intent
@use "ial/hardware"
@use "ial/temporal"

Feature: Temperature Monitor
  id: feature.temperature

  Scenario: Normal operation
    When sensor "temp" reads 72°F
    → LED "status" is color "green"
    → display shows "72°F"
    → no alert is triggered

  Scenario: Overheat warning
    When sensor "temp" reads above 90°F
    → within 100ms, LED "status" is color "red"
    → buzzer sounds at 1000Hz
    → message is sent via MQTT to "alerts/temperature"
    → display shows "WARNING: 92°F"

  Scenario: Cooling response
    Given temperature is above 90°F
    When relay "fan" is closed
    → motor "fan" is running
    → within 5 minutes, sensor "temp" reads below 85°F
    → relay "fan" is open

Feature: Smart Lock
  id: feature.smart_lock

  Scenario: Unlock via app
    When unlock command is received via BLE
    → servo "lock" angle is 90°
    → LED "status" is color "green"
    → for 3 seconds, buzzer sounds at 2000Hz
    → message is sent via MQTT to "events/unlock"

  Scenario: Auto-lock timeout
    Given lock is unlocked
    When 30 seconds pass without activity
    → servo "lock" angle is 0°
    → LED "status" is color "red"

Feature: Motion Sensor
  id: feature.motion

  Scenario: Motion detected
    When sensor "PIR" reads above 0.5
    → LED "motion" is on
    → for 30 seconds, LED "motion" stays on
    → after 30 seconds of no motion, LED "motion" is off

  Scenario: Night mode
    Given sensor "light" reads below 100 lux
    When sensor "PIR" reads above 0.5
    → relay "lights" is closed
    → for 2 minutes, lights stay on

Feature: Robotic Arm
  id: feature.robot_arm

  Scenario: Pick and place
    When command "pick from A, place at B" is received
    → servo "shoulder" moves to pickup position
    → servo "gripper" closes
    → sensor "grip" reads above 0.8
    → servo "shoulder" moves to dropoff position
    → servo "gripper" opens
    → task completion is sent via MQTT

  Scenario: Safety stop
    When sensor "force" reads above 10N
    → immediately, all motors stop
    → LED "status" is color "red"
    → alarm is triggered
    → message is sent via MQTT to "alerts/safety"
```

---

### 14.8 Extension Compatibility Matrix

| Extension         | Combines Well With         | Common Use Cases                 |
| ----------------- | -------------------------- | -------------------------------- |
| **Temporal**      | Spatial, Streaming, Mobile | Games, animations, real-time UIs |
| **State Machine** | All                        | Complex workflows in any domain  |
| **Streaming**     | Temporal, Probabilistic    | Real-time data, event sourcing   |
| **Spatial**       | Temporal                   | Games, 3D apps, AR/VR            |
| **Probabilistic** | Streaming                  | ML pipelines, A/B tests          |
| **Mobile**        | Temporal, Spatial          | Mobile games, gesture-heavy apps |
| **Hardware**      | Temporal, State Machine    | IoT, robotics, embedded          |

---

### 14.9 Creating Custom Extensions

If the built-in extensions don't cover your domain, you can create custom extensions:

```intent
# extensions/medical.intent

@extension "ial/medical"

## Extension Glossary

| Term | Means |
|------|-------|
| patient vitals are normal | Heart rate, BP, O2 within healthy ranges |
| alert is escalated to "$role" | Notification sent to medical staff |
| medication is administered | Drug delivery logged |
| dosage is within safe limits | Dose < max for patient weight |

## Extension Components

Component: Vital Signs Alert
  id: component.vital_alert
  parameters: [metric, threshold, direction]

  Inherent Behavior:
    → when $metric goes $direction $threshold
    → alert is triggered within 5 seconds
    → alert is escalated to "nurse"
    → event is logged to patient record
```

Usage:

```intent
@use "extensions/medical"

Feature: ICU Monitoring
  Scenario: Heart rate spike
    When patient heart rate exceeds 120 bpm
    → vital signs alert for heart_rate, 120, above
    → they see alert on monitoring station
```

---

## Appendix A: Grammar BNF

```bnf
<intent-file>     ::= <header> <imports>? <glossary>? <components>? <section>* <feature>+ <constraint>*

<header>          ::= "#" <text> NEWLINE+

<imports>         ::= <import>+

<import>          ::= "@import" <string> NEWLINE

<glossary>        ::= "## Glossary" NEWLINE <glossary-table> <bindings>?

<glossary-table>  ::= "|" "Term" "|" "Means" "|" NEWLINE
                      "|" "---" "|" "---" "|" NEWLINE
                      <glossary-row>+

<glossary-row>    ::= "|" <term> "|" <meaning> "|" NEWLINE

<bindings>        ::= "## Glossary [Technical Bindings]" NEWLINE <binding>+

<binding>         ::= <term> ":" NEWLINE <binding-body>

<components>      ::= "## Components" NEWLINE <component>+

<component>       ::= "Component:" <name> NEWLINE <component-body>

<component-body>  ::= <component-prop>* <inherent-behavior>? <scenario>*

<component-prop>  ::= "id:" <identifier> NEWLINE
                    | "parameters:" <param-list> NEWLINE
                    | "extends:" <component-ref> NEWLINE
                    | "description:" <string> NEWLINE

<param-list>      ::= "[" <param> ("," <param>)* "]"

<param>           ::= <identifier> (":" <type>)? ("=" <default>)?

<inherent-behavior> ::= "Inherent Behavior:" NEWLINE <outcome>+

<component-ref>   ::= "component." <identifier> "(" <arg-list>? ")"

<feature>         ::= "---" NEWLINE "Feature:" <name> NEWLINE <feature-body>

<feature-body>    ::= <property>* <scenario>+

<scenario>        ::= "Scenario:" <name> NEWLINE <when-clause>? <outcome>+

<when-clause>     ::= "When" <subject> <action> NEWLINE

<outcome>         ::= ("→" | "->") <assertion> NEWLINE

<assertion>       ::= <standard-term> | <glossary-term> | <component-ref>
```

---

## Appendix B: Standard Term Index

Quick reference for all built-in standard terms:

**Outcomes:** `succeeds`, `fails`, `fails with "$msg"`, `no error`, `takes less than $time`

**HTTP:** `they see "$text"`, `they don't see "$text"`, `redirects to $path`, `returns status $code`, `returns JSON`, `returns HTML`, `response includes "$field"`, `"$field" is "$value"`, `header "$name" is/contains "$value"`

**CLI:** `exits successfully`, `exits with error`, `exits with code $n`, `output shows "$text"`, `output matches $pattern`, `error shows "$text"`, `no error output`

**Files:** `file "$path" exists/is created/is deleted/contains "$text"/is empty`, `directory "$path" exists/has $n files`

**Database:** `record is created/updated/deleted`, `row exists where $cond`, `no row exists where $cond`, `row count is/increases by $n`, `"$column" is "$value"`

**Events:** `email is sent to "$addr"`, `event "$name" is emitted`, `no event "$name"`, `"$message" is logged`

**Behavioral:** `when repeated $n times, still $outcome`, `when called simultaneously, $outcome`, `makes at most $n database queries`, `makes no external calls`, `data is unchanged`

**Timing:** `responds in under $time`, `completes within $time`

---

## Appendix C: Changelog

### Version 1.0.0-draft (January 2026)

- Initial draft specification
- Glossary-based natural language approach
- Standard glossary with 50+ built-in terms
- Progressive disclosure (PM view / Dev view)
- Full roadmap coverage mapping
- HTTP, CLI, Database, Event, File System domains
- Behavioral properties (idempotency, thread safety, resource constraints)
- **Components (Reusable Intent Blocks)** for DRY, cascading verification
  - Component definitions with parameters and inherent behavior
  - Component inheritance and extension
  - Component libraries and imports
  - Built-in component templates
  - Cascading verification of component assertions

---

## Appendix D: References

- [Intent-Driven Development Guide](./INTENT_DRIVEN_DEVELOPMENT.md)
- [NTNT Language Specification](../LANGUAGE_SPEC.md)
- [AI Agent Guide](./AI_AGENT_GUIDE.md)
- [NTNT Roadmap](../ROADMAP.md)

---

_This specification is part of the NTNT language project. Licensed under MIT + Apache 2.0._
