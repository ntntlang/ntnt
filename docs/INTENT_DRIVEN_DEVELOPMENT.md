# Intent-Driven Development (IDD)

## Design Document

**Status:** Draft  
**Author:** Josh Cramer + GitHub Copilot  
**Created:** January 13, 2026  
**Last Updated:** January 13, 2026

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [The Problem](#the-problem)
3. [Design Goals](#design-goals)
4. [The Intent File Format](#the-intent-file-format)
5. [CLI Commands](#cli-commands)
6. [The Workflow](#the-workflow)
7. [Human Experience](#human-experience)
8. [Agent Experience](#agent-experience)
9. [Code Integration](#code-integration)
10. [Implementation Plan](#implementation-plan)
11. [Open Questions](#open-questions)
12. [Future Possibilities](#future-possibilities)

---

## Executive Summary

Intent-Driven Development (IDD) is a paradigm where **human intent becomes executable specification**. Rather than writing requirements in documents that get stale, or coding directly without clear specification, IDD creates a **single source of truth** that:

1. **Humans can read** - Natural language descriptions of what the app should do
2. **Agents can execute** - Structured assertions that verify the code matches intent
3. **Both can evolve together** - When requirements change, intent updates first, then code follows

This is the killer feature of NTNT - making it the first language where **intent is code**.

---

## The Problem

### Current State of Human-Agent Collaboration

The typical development cycle with AI looks like this:

1. Human: "Build me a snow gauge app"
2. Agent: _builds something based on assumptions_
3. Human: "No, I wanted it to show the last 30 days"
4. Agent: _rebuilds with new assumption_
5. Human: "Wait, also add site selection"
6. Agent: _patches it in, maybe breaks something_
7. ... endless back and forth ...

### The Core Problems

| Issue                     | Impact                                       |
| ------------------------- | -------------------------------------------- |
| Intent is scattered       | Chat history, code comments, human's mind    |
| No verification           | Agent cannot prove code matches intent       |
| Requirements drift        | Original intent gets lost in iterations      |
| No single source of truth | Human and agent have different mental models |
| Stale documentation       | README does not match actual behavior        |

### What We Want Instead

A feedback loop where intent drives implementation and verification proves correctness:

```
INTENT (contract) --> CODE (implementation) --> VERIFICATION (proof)
     ^                                               |
     |                                               |
     +-----------------------------------------------+
                    Feedback Loop
```

---

## Design Goals

### For Humans

| Goal               | Description                                                |
| ------------------ | ---------------------------------------------------------- |
| **Readable**       | Intent files should read like plain English requirements   |
| **Trustworthy**    | If the intent check passes, my app does what I asked       |
| **Easy to modify** | Changing requirements = editing text, not debugging code   |
| **Fun**            | Feels like having a conversation, not writing formal specs |
| **Empowering**     | Non-technical humans can meaningfully participate          |

### For Agents

| Goal            | Description                                      |
| --------------- | ------------------------------------------------ |
| **Unambiguous** | Clear success/failure criteria for every feature |
| **Parseable**   | Structured format that maps directly to tests    |
| **Complete**    | Everything needed to implement without guessing  |
| **Verifiable**  | Can prove implementation matches intent          |
| **Efficient**   | Don't waste cycles on misunderstood requirements |

### For Both

| Goal                       | Description                                   |
| -------------------------- | --------------------------------------------- |
| **Single source of truth** | One file that both human and agent reference  |
| **Living documentation**   | Intent file IS the spec, always current       |
| **Collaborative**          | Easy for human and agent to co-author         |
| **Evolvable**              | Requirements change? Update intent, re-verify |

---

## The Intent File Format

### Philosophy

The `.intent` file format must balance two tensions:

1. **Human-readable** (like Markdown) vs **Machine-parseable** (like YAML)
2. **Flexible** (natural language) vs **Precise** (testable assertions)

**Solution:** A hybrid format with clearly separated sections:

- Prose sections for human understanding
- Structured sections for machine verification

### File Structure Overview

```
myapp.intent
├── Meta (version, status, authors)
├── Purpose (free-form prose for humans)
├── Glossary (shared vocabulary)
├── Data (schemas and structures)
├── Features (testable requirements)
├── Constraints (limitations and rules)
├── UI/UX (visual requirements)
└── Non-Requirements (explicit scope boundaries)
```

### Section Details

#### Meta Section

Basic project information for tracking.

```yaml
app: snowgauge
version: 0.1.0
status: active # active | draft | deprecated
updated: 2026-01-13
```

#### Purpose Section (Human-Focused)

Free-form prose describing what the app is for. Agents read this for context but don't parse it strictly.

```
Snowgauge displays real-time snow conditions from SNOTEL weather stations
for backcountry skiers and snowboarders in Colorado. Users can quickly
check current snow depth, recent snowfall, and trends before heading out.

Target users: Backcountry enthusiasts who want quick snow data
Primary use case: Morning check before deciding where to ski
```

#### Glossary Section (Shared Understanding)

Define domain terms so human and agent share vocabulary.

```
SNOTEL: Snow Telemetry - USDA automated weather stations that measure snowpack

Snow Depth: Total height of accumulated snow on the ground, measured in inches

SWE (Snow Water Equivalent): The amount of water contained in the snowpack
if melted, in inches

New Snow: Snow accumulation in the last 24 hours, calculated as depth change
```

#### Data Section (Machine-Parseable)

Define schemas and data structures the app works with.

```
### Schema: Site
Represents a SNOTEL weather station.

| Field | Type | Description |
|-------|------|-------------|
| key | String | URL-safe identifier (e.g., "bear_lake") |
| name | String | Display name (e.g., "Bear Lake") |
| url | String | SNOTEL API endpoint URL |
| elevation | Int? | Optional elevation in feet |

### Instance: sites
- Type: Map of String to Site
- Default: bear_lake
- Required keys: bear_lake, wild_basin, copeland_lake
```

#### Features Section (Testable)

Each feature has a description AND testable assertions.

```
### Feature: Site Selection
Priority: Must Have

Users can select which SNOTEL site to view via URL query parameter.

Behavior:
- Parameter ?site=key selects the site
- Invalid keys fall back to default site (bear_lake)
- Missing parameter uses default site

Tests:
- GET /?site=bear_lake -> 200, contains "Bear Lake"
- GET /?site=wild_basin -> 200, contains "Wild Basin"
- GET /?site=invalid -> 200, contains "Bear Lake" (fallback)
- GET / -> 200, contains "Bear Lake" (default)
```

#### Constraints Section

Rules and limitations the app must follow.

```
### Constraint: No Caching
Data must always be fresh from SNOTEL API.
Rationale: Snow conditions change rapidly; stale data could be dangerous.

### Constraint: Graceful Errors
Application must not crash if SNOTEL API is unavailable.
Behavior: Show user-friendly error message.
```

#### Non-Requirements Section

Explicitly state what is OUT of scope.

```
The following are explicitly OUT OF SCOPE:

- User accounts or authentication
- Data persistence or saved preferences
- Historical year-over-year comparison
- Alerts or notifications
- Native mobile app
- Offline mode
```

---

## Complete Example: snowgauge.intent

```intent
# snowgauge.intent
# Intent Specification v1.0

## Meta

app: snowgauge
version: 0.1.0
status: active
updated: 2026-01-13

---

## Purpose

Snowgauge displays real-time snow conditions from SNOTEL weather stations
for backcountry skiers and snowboarders in Colorado. Users can quickly
check current snow depth, recent snowfall, and trends before heading out.

Target users: Backcountry enthusiasts who want quick snow data
Primary use case: Morning check before deciding where to ski

---

## Glossary

SNOTEL: Snow Telemetry - USDA automated weather stations that measure snowpack

Snow Depth: Total height of accumulated snow on the ground, measured in inches

SWE: Snow Water Equivalent - the amount of water in the snowpack if melted

New Snow: Snow accumulation in the last 24 hours, calculated as depth change

---

## Data

### Schema: Site

| Field | Type | Description |
|-------|------|-------------|
| key | String | URL-safe identifier |
| name | String | Display name |
| url | String | SNOTEL API endpoint |
| elevation | Int? | Elevation in feet |

### Instance: sites

- Type: Map of String to Site
- Default: bear_lake
- Required: bear_lake, wild_basin, copeland_lake

---

## Features

### Feature: Site Selection
Priority: Must Have

Users select which SNOTEL site to view via URL query parameter.

Behavior:
- ?site=key selects the site
- Invalid keys fall back to bear_lake
- Missing parameter uses bear_lake

Tests:
- GET /?site=bear_lake -> 200, contains "Bear Lake"
- GET /?site=wild_basin -> 200, contains "Wild Basin"
- GET /?site=invalid -> 200, contains "Bear Lake"
- GET / -> 200, contains "Bear Lake"

---

### Feature: Snow Display
Priority: Must Have

Display current snow conditions for the selected site.

Shows:
- Site name (from SNOTEL header)
- Current snow depth (inches)
- New snow (24hr change)
- Snow water equivalent

Tests:
- GET / -> contains "Snow Depth"
- GET / -> contains "New Snow"
- GET / -> contains "SWE"

---

### Feature: 30-Day Chart
Priority: Should Have

Line chart showing snow depth trend.

Specification:
- Type: Line chart
- X-axis: Date (last 30 days)
- Y-axis: Snow depth (inches)
- Library: Chart.js

Tests:
- GET / -> contains "canvas"
- GET / -> contains "Chart"

---

## Constraints

### No Caching
Always fetch fresh data from SNOTEL.
Rationale: Snow changes rapidly.

### Single File
Entire app in one .tnt file.
Rationale: Simplicity.

### Graceful Errors
Don't crash if SNOTEL is down.
Show user-friendly message.

---

## UI/UX

Layout:
- Mobile-first responsive
- Dark header with site name
- Card-based stats
- Chart below stats
- Navigation at bottom

Style:
- High contrast (outdoor visibility)
- Large numbers

---

## Non-Requirements

Out of scope:
- User accounts
- Data persistence
- Historical comparison
- Alerts/notifications
- Native mobile app
- Offline mode
```

---

## CLI Commands

### ntnt intent check

Verify implementation matches intent specification.

```
$ ntnt intent check snowgauge.tnt

Checking snowgauge.intent against snowgauge.tnt...

Features:
  [PASS] Site Selection (4/4 tests passed)
  [PASS] Snow Display (3/3 tests passed)
  [FAIL] 30-Day Chart (1/2 tests passed)
         FAIL: body contains "canvas"
               Actual: uses div element

Constraints:
  [PASS] No Caching
  [PASS] Single File
  [WARN] Graceful Errors (needs mock testing)

Data:
  [PASS] Schema: Site
  [WARN] sites missing: copeland_lake

Summary: 2/3 features passing | Coverage: 78%
```

Exit codes:

- 0 = All tests pass
- 1 = One or more tests fail
- 2 = Intent file parse error

### ntnt intent init

Generate code scaffolding from intent.

```
$ ntnt intent init snowgauge.intent

Generated: snowgauge.tnt
  - sites map with 3 entries (URLs marked TODO)
  - home_handler() stub with TODO comments
  - Feature stubs linked to intent

Next: ntnt intent check snowgauge.tnt
```

### ntnt intent watch

Continuous verification during development.

```
$ ntnt intent watch snowgauge.tnt

Watching for changes... (Ctrl+C to stop)

[12:34:56] All checks passed
[12:35:12] Feature "chart" failing
[12:35:45] All checks passed
```

### ntnt intent coverage

Show implementation coverage report.

```
$ ntnt intent coverage snowgauge.tnt

Intent Coverage Report

Feature Coverage:  100% (3/3 features)
Data Coverage:     67% (2/3 required keys)
Code Linkage:      50% (3/6 functions)

Unlinked functions:
  - extract_snotel_name()
  - parse_csv_row()
  - format_inches()
```

### ntnt intent diff

Show gaps between intent and implementation.

```
$ ntnt intent diff snowgauge.tnt

Intent vs Implementation

Data: sites
  Intent: bear_lake, wild_basin, copeland_lake
  Code:   bear_lake, wild_basin
  Missing: copeland_lake

Feature: chart
  Intent: uses canvas element
  Code:   uses div element
```

---

## The Workflow

### Phase 1: Collaborative Intent Design

Human and agent work together to create the intent file.

```
Human: "I want to build a snow conditions app"

Agent: "I'll help create an intent file. Let's start with the basics:
       - What's the main purpose?
       - Who will use it?
       - What data sources?"
```

The agent asks clarifying questions and drafts sections. The human reviews and refines. Both agree on the intent before any code is written.

### Phase 2: Code Scaffolding

```
$ ntnt intent init snowgauge.intent

Generated: snowgauge.tnt with stubs
```

Agent generates initial code structure with:

- Data structures matching schemas
- Function stubs for each feature
- TODO comments linked to intent items

### Phase 3: Implementation with Continuous Verification

```
$ ntnt intent watch snowgauge.tnt
```

Agent implements features one by one. Each save triggers verification. Agent sees immediately if implementation drifts from intent.

### Phase 4: Human Review

Human reviews by looking at:

1. The intent file (readable requirements)
2. The verification output (proof it works)

No need to read all the code - the intent check proves correctness.

### Phase 5: Requirements Change

```
Human: "Actually, I want 60 days of data, not 30"
```

Workflow:

1. Update intent file: "X-axis: Date (last 60 days)"
2. Run `ntnt intent check` - fails
3. Agent updates code
4. Check passes
5. Done

---

## Human Experience

### What Makes It Fun

1. **Conversation, not specification**: Writing intent feels like explaining to a friend
2. **Immediate feedback**: See if your app works without running it yourself
3. **Change without fear**: Update intent, agent handles the rest
4. **Understand without code**: Intent file IS the documentation

### Example Human Journey

```
Day 1: "I have an idea for an app"
       -> Conversation with agent
       -> Intent file created
       -> Basic app running

Day 2: "Can we add a feature?"
       -> Update intent file together
       -> Agent implements
       -> Verification passes

Day 3: "Actually, change that..."
       -> Edit intent file
       -> Agent adapts
       -> Still verified
```

### What Humans DON'T Have to Do

- Read code to understand what the app does
- Debug implementation details
- Write formal specifications
- Learn programming concepts

---

## Agent Experience

### What Makes It Satisfying

1. **Clear success criteria**: Know exactly when done
2. **No guessing**: Intent file tells me what to build
3. **Provable correctness**: Can demonstrate I built the right thing
4. **Efficient iteration**: Don't waste cycles on wrong approaches

### Agent Mental Model

```
1. Read intent file
2. Understand each feature's tests
3. Implement to pass tests
4. Run verification
5. If pass -> done
6. If fail -> fix and repeat
```

### What Agents Get That They Don't Have Today

| Today                    | With IDD                |
| ------------------------ | ----------------------- |
| Vague requirements       | Testable assertions     |
| Hope it's right          | Prove it's right        |
| Redo on misunderstanding | Clear spec upfront      |
| Documentation separate   | Intent IS documentation |

---

## Code Integration

### @implements Annotations

Link code to intent items for traceability.

```ntnt
// @implements: feature.site_selection
fn get_site_from_params(req) {
    let site_param = get_key(req.query_params, "site", "bear_lake")
    return get_key(sites, site_param, sites["bear_lake"])
}

// @implements: feature.snow_display
// @implements: feature.chart
fn home_handler(req) {
    // ...
}

// @implements: data.sites
let sites = map {
    "bear_lake": { "name": "Bear Lake", "url": "..." },
    "wild_basin": { "name": "Wild Basin", "url": "..." }
}
```

### Benefits of Annotations

1. **Traceability**: Know which code implements which intent
2. **Coverage**: Find unlinked code that might be dead
3. **Refactoring safety**: Know what intent an edit might affect

---

## Implementation Plan

### Phase 1: Core (2-3 weeks)

| Component           | Description             | Effort   |
| ------------------- | ----------------------- | -------- |
| Intent parser       | Parse .intent files     | 3-4 days |
| HTTP test runner    | Run GET/POST assertions | 3-4 days |
| `ntnt intent check` | Basic verification      | 2-3 days |
| Output formatting   | Clear pass/fail display | 1-2 days |

### Phase 2: Tooling (2 weeks)

| Component              | Description      | Effort   |
| ---------------------- | ---------------- | -------- |
| `ntnt intent init`     | Code scaffolding | 3-4 days |
| `ntnt intent watch`    | File watching    | 2-3 days |
| `ntnt intent coverage` | Coverage report  | 2-3 days |
| `ntnt intent diff`     | Gap analysis     | 2-3 days |

### Phase 3: Polish (1-2 weeks)

| Component          | Description            | Effort   |
| ------------------ | ---------------------- | -------- |
| @implements parser | Code annotations       | 2-3 days |
| Schema validation  | Data structure checks  | 3-4 days |
| Error messages     | Helpful failure output | 2-3 days |

**Total estimate: 5-7 weeks**

---

## Open Questions

### Format Questions

1. **Markdown-based or custom syntax?**

   - Markdown: More familiar, better tooling
   - Custom: More precise, less ambiguous

2. **How to handle complex assertions?**

   - Regex patterns?
   - JSON path queries?
   - Custom DSL?

3. **How to handle stateful tests?**
   - POST creates data, GET retrieves it
   - Setup/teardown?

### Scope Questions

4. **Should intent include implementation hints?**

   - "Use Chart.js" vs "Show a chart"
   - More prescription = less flexibility

5. **How strict should verification be?**

   - Exact string match vs contains?
   - Required fields vs nice-to-have?

6. **How to handle UI testing?**
   - Visual regression?
   - Accessibility checks?
   - Just check for elements?

### Workflow Questions

7. **Who owns the intent file?**

   - Human only?
   - Agent can suggest changes?
   - Collaborative editing?

8. **Versioning strategy?**
   - Intent file versioned with code?
   - Separate versioning?

---

## Future Possibilities

### Intent Visualization

Visual representation of intent relationships - a dashboard showing which features pass/fail.

### Intent Diffing Across Versions

Track how intent evolves over time with history commands.

### Cross-Project Intent Patterns

Reusable intent templates for common patterns (web API, CRUD app, etc.).

### AI-Assisted Intent Refinement

Agent suggests missing intents based on code analysis.

---

## Summary

Intent-Driven Development transforms the human-agent collaboration from:

**Before:** "Build me X" -> _builds something_ -> "No, I meant Y" -> _rebuilds_ -> repeat

**After:** Intent file -> verified implementation -> confident deployment

The .intent file becomes:

- **For humans**: Plain English requirements they can read and edit
- **For agents**: Testable assertions they can verify against
- **For both**: A single source of truth that evolves with the project

This is what makes NTNT truly "AI-native" - not just a language agents can write, but a development paradigm designed for human-agent collaboration.

---

_This document is a living design. Let's continue refining it together._
