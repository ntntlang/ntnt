---
name: idd
description: Intent-Driven Development for NTNT. Use when building features with .intent files.
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
  - AskUserQuestion
---

# NTNT Intent-Driven Development Skill

You are helping build an NTNT application using Intent-Driven Development (IDD).

## Arguments

`$ARGUMENTS` can be:
- `new <name>` - Create a new project with intent file
- `intent <file>` - Create/edit an intent file for a .tnt file
- `implement <file>` - Implement from an intent file
- `check <file>` - Verify implementation against intent
- (no args) - Interactive mode, ask what to do

## IDD Workflow (CRITICAL)

IDD is a **collaborative** process. Follow these steps:

### Phase 1: Draft Intent (STOP for approval)

1. Draft the `.intent` file based on requirements
2. **PRESENT to user and STOP** - do NOT implement yet
3. Ask clarifying questions
4. Wait for explicit user approval

### Phase 2: Implement (after approval)

```bash
ntnt intent init project.intent -o server.tnt
```

Add `@implements` annotations to functions:
```ntnt
// @implements: feature.user_login
fn login_handler(req) { ... }
```

### Phase 3: Verify (MANDATORY before done)

```bash
ntnt lint server.tnt
ntnt intent check server.tnt
ntnt intent coverage server.tnt
```

## Intent File Format

File must be named to match the .tnt file (e.g., `server.intent` for `server.tnt`).

```intent
# Project Name
# Description of the project
# Run: ntnt intent check server.tnt

## Overview
Brief description of what this project does.

## Glossary

| Term | Means |
|------|-------|
| success response | status 2xx |
| they see {text} | body contains {text} |
| they don't see {text} | body not contains {text} |
| returns HTML | header "Content-Type" contains "text/html" |
| page loads successfully | status 200, returns HTML |

---

Component: Reusable Pattern
  id: component.pattern_id
  description: "Reusable behavior pattern"

  Inherent Behavior:
    → expected behavior 1
    → expected behavior 2

---

Feature: Feature Name
  id: feature.feature_id
  description: "Human-readable description"

  Scenario: Happy path
    description: "What this scenario tests"
    When a user visits the homepage
    → page loads successfully
    → they see "Expected Content"

  Scenario: Error case
    description: "Handles edge cases"
    Given some precondition
    When a user performs an action
    → status 404
    → they see "not found"

---

Constraint: Constraint Name
  description: "Description of the constraint"
  applies_to: [feature.feature_id]
```

## Key Intent File Rules

- Use `Feature:` (capitalized) followed by feature name
- `id:` must be `feature.<snake_case_id>` - used for `@implements`
- Use `Scenario:` blocks with `When` and `→` (arrow) assertions
- `Given:` for preconditions, `When:` for actions
- `## Glossary` defines reusable term mappings
- `Component:` for reusable behavior patterns
- Separate sections with `---`
- Use `## Module:` headers for organizing larger apps

## NTNT Syntax Reminders

### Map Literals REQUIRE `map` keyword
```ntnt
let data = map { "key": "value" }  // CORRECT
let data = { "key": "value" }      // WRONG
```

### String Interpolation Uses `{expr}` NOT `${expr}`
```ntnt
let msg = "Hello, {name}!"  // CORRECT
```

### Route Patterns REQUIRE Raw Strings
```ntnt
get(r"/users/{id}", handler)  // CORRECT
get("/users/{id}", handler)   // WRONG
```

### HTTP Server - Global Builtins vs Imports
```ntnt
// ONLY import response builders
import { json, html } from "std/http/server"

// Routing functions are GLOBAL - no import needed
get("/", home_handler)
post(r"/api/users", create_user)
listen(8080)
```

### Named Handlers Required (No Inline Lambdas)
```ntnt
fn handler(req) { ... }
get("/path", handler)  // CORRECT

get("/path", |req| { ... })  // WRONG - parser error
```

## Standard Assertions (IAL)

| Pattern | Description |
|---------|-------------|
| `status 200` | Exact status code |
| `status 2xx` | Any success status |
| `body contains {text}` | Body includes text |
| `body not contains {text}` | Body excludes text |
| `header {name} contains {value}` | Header check |
| `redirects to {path}` | Redirect check |
| `returns JSON` | Content-Type check |

## Commands to Run

```bash
ntnt lint file.tnt           # ALWAYS run first
ntnt run file.tnt            # Run the application
ntnt intent check file.tnt   # Verify against intent
ntnt intent coverage file.tnt # Show coverage
ntnt intent studio file.intent # Visual preview (opens :3001)
```

## Execution

Based on `$ARGUMENTS`:

1. **`new <name>`**: Create `<name>.intent` and `<name>.tnt` scaffold, present intent for approval
2. **`intent <file>`**: Create/edit intent file for the given .tnt file, present for approval
3. **`implement <file>`**: Generate scaffolding from approved intent, add `@implements` annotations
4. **`check <file>`**: Run lint and intent check, report results
5. **No args**: Ask user what they want to build, then start with intent drafting

Always lint before running. Always verify intent before saying "done".

## Full Documentation

For complete NTNT syntax and stdlib reference, see [docs/AI_AGENT_GUIDE.md](../../docs/AI_AGENT_GUIDE.md).
