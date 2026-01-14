# Intent-Driven Development (IDD)

## Design Document

**Status:** Draft  
**Author:** Josh Cramer + GitHub Copilot  
**Created:** January 13, 2026  
**Last Updated:** January 13, 2026

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [How IDD Differs from TDD](#how-idd-differs-from-test-driven-development-tdd)
3. [The Problem](#the-problem)
4. [Design Goals](#design-goals)
5. [The Intent File Format](#the-intent-file-format)
6. [CLI Commands](#cli-commands)
7. [The Workflow](#the-workflow)
8. [Human Experience](#human-experience)
9. [Agent Experience](#agent-experience)
10. [Team Collaboration](#team-collaboration)
11. [Code Integration](#code-integration)
12. [Implementation Plan](#implementation-plan)
13. [Open Questions](#open-questions)
14. [Future Possibilities](#future-possibilities)

---

## Executive Summary

Intent-Driven Development (IDD) is a paradigm where **human intent becomes executable specification**. Rather than writing requirements in documents that get stale, or coding directly without clear specification, IDD creates a **single source of truth** that:

1. **Humans can read** - Natural language descriptions of what the app should do
2. **Agents can execute** - Structured assertions that verify the code matches intent
3. **Both can evolve together** - When requirements change, intent updates first, then code follows

NTNT attempts to be the first language where **intent is code**.

---

## The Problem

### Current State of Human-Agent Collaboration

The typical development cycle with AI looks like this:

1. Human: "Build me a an app that shows precipitation over the last 24 hours"
2. Agent: _builds something based on assumptions_
3. Human: "No, I wanted it to show the last 30 days"
4. Agent: _rebuilds with new assumption_
5. Human: "Wait, also add location selection"
6. Agent: _patches it in, maybe breaks something_
7. ... endless back and forth ...

### The Core Problems

| Issue                     | Impact                                       |
| ------------------------- | -------------------------------------------- |
| Intent emerges over time  | Humans evolve their thinking and desires     |
| Intent mixed with extras  | Resulting code may contain extraneous bits   |
| Intent is scattered       | Chat history, code comments, human's mind    |
| No verification           | Agent cannot prove code matches intent       |
| Requirements drift        | Original intent gets lost in iterations      |
| No single source of truth | Human and agent have different mental models |
| Stale documentation       | README does not match actual behavior        |

### A Proposed Solution

A feedback loop where intent drives implementation and verification proves correctness:

```
INTENT (contract) --> CODE (implementation) --> VERIFICATION (proof)
     ^                                               |
     |                                               |
     +-----------------------------------------------+
                    Feedback Loop
```

---

## How IDD Differs from Test-Driven Development (TDD)

Test-Driven Development already exists and is widely practiced. Why do we need something new?

### TDD: What It Is

In TDD, developers:

1. Write a failing test first
2. Write code to make the test pass
3. Refactor while keeping tests green

TDD is excellent for **code quality** and **developer confidence**.

### The Gap TDD Leaves

| Aspect               | TDD                                | IDD                                      |
| -------------------- | ---------------------------------- | ---------------------------------------- |
| **Written by**       | Developers                         | Humans + Agents collaboratively          |
| **Written in**       | Code (Python, JavaScript, etc.)    | Natural language + structured assertions |
| **Readable by**      | Developers only                    | Anyone                                   |
| **Answers**          | "Does the code work?"              | "Does the code do what I wanted?"        |
| **Abstraction**      | Implementation details             | Business intent                          |
| **Owns the spec**    | Tests ARE the spec (but in code)   | Intent IS the spec (in English)          |
| **Documentation**    | Tests != docs (separate artifacts) | Intent = living documentation            |
| **AI collaboration** | Not designed for agents            | Explicitly designed for human-agent work |

### A Concrete Example

**TDD Test (developer writes this):**

```python
def test_location_selection():
    response = client.get("/?location=denver")
    assert response.status_code == 200
    assert "Denver" in response.text

def test_invalid_location_falls_back():
    response = client.get("/?location=invalid")
    assert response.status_code == 200
    assert "Denver" in response.text  # Falls back to default
```

**IDD Intent (human writes this):**

```
### Feature: Location Selection

Users can select which location to view via URL parameter.

Behavior:
- ?location=<key> selects the location
- Invalid keys fall back to default (Denver)

Tests:
- GET /?location=denver -> 200, contains "denver"
- GET /?location=invalid -> 200, contains "denver"
```

### What IDD Solves That TDD Doesn't

1. **The "Why" Problem**

   - TDD tests say _what_ should happen
   - IDD intent explains _why_ it should happen
   - A test named `test_location_selection` doesn't explain the feature; intent does

2. **The Audience Problem**

   - TDD requires code literacy to understand the spec
   - IDD lets non-technical stakeholders read and edit requirements
   - Product managers can review intent files; they can't review pytest files

3. **The AI Collaboration Problem**

   - TDD assumes a human developer writes both tests and code
   - IDD explicitly separates "what I want" (human) from "how to build it" (agent)
   - Agents can verify their work against intent without human re-review

4. **The Documentation Problem**

   - TDD tests become stale documentation (or aren't documentation at all)
   - IDD intent IS the documentation, always verified against code
   - No separate README that drifts from reality

5. **The Evolution Problem**
   - In TDD, changing requirements means rewriting tests (developer work)
   - In IDD, changing requirements means editing plain text (human work)
   - The agent handles translating new intent into new code

### IDD and TDD Can Coexist

IDD doesn't replace TDD - it operates at a different level:

```
┌─────────────────────────────────────────────────────────┐
│  INTENT (human-readable, business requirements)         │  ← IDD
├─────────────────────────────────────────────────────────┤
│  INTEGRATION TESTS (API contracts, end-to-end)          │  ← IDD generates these
├─────────────────────────────────────────────────────────┤
│  UNIT TESTS (implementation details, edge cases)        │  ← TDD lives here
├─────────────────────────────────────────────────────────┤
│  CODE (the actual implementation)                       │
└─────────────────────────────────────────────────────────┘
```

- **IDD** handles the top layer: "Does this app do what the human wanted?"
- **TDD** handles the bottom layer: "Does this function handle edge cases correctly?"

An agent might still use TDD practices when implementing complex functions. IDD just ensures the overall app matches human intent.

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

## Test Execution Mechanics

How does `ntnt intent check` actually determine pass or fail? The verification engine uses different strategies based on program type and what's being tested.

### Program Type Detection

The engine auto-detects the program type from the intent file or code:

```yaml
# Intent file can declare type explicitly
Meta:
  type: http-server # or: cli, library, script, worker, daemon
```

If not declared, the engine infers type:

- Imports `std/http_server` → HTTP server
- Has `fn main()` with args → CLI tool
- Exports public functions only → Library
- Has `fn main()` without args → Script
- Imports `std/concurrent` with workers → Background worker

### Function/Unit Tests (Libraries, Utilities)

For libraries and pure functions, the engine calls functions directly:

```yaml
Feature: CSV Parsing
  description: Parse CSV data into structured records
  test:
    - call: parse_csv("name,age\nAlice,30")
      assert:
        - result[0]["name"] == "Alice"
        - result[0]["age"] == "30"
        - len(result) == 1

    - call: parse_csv("")
      assert:
        - result == []

    - call: parse_csv("invalid\ndata,missing")
      assert:
        - throws: ParseError
```

Engine executes:

```
1. Import module
2. Call parse_csv("name,age\nAlice,30")
3. Check result[0]["name"] == "Alice" ✓
4. Check result[0]["age"] == "30" ✓
5. Check len(result) == 1 ✓
6. Report: PASS (3/3 assertions)
```

### CLI Tests (Command-Line Tools)

For CLI applications, the engine runs the program with arguments and checks output:

```yaml
Feature: File Search
  description: Find files matching a pattern
  test:
    - run: search "*.txt" ./testdir
      assert:
        - exit_code: 0
        - stdout contains "found 3 files"
        - stdout contains "notes.txt"

    - run: search "*.xyz" ./testdir
      assert:
        - exit_code: 0
        - stdout contains "found 0 files"

    - run: search "*.txt" ./nonexistent
      assert:
        - exit_code: 1
        - stderr contains "directory not found"
```

Engine executes:

```
1. Create temp test directory with fixtures
2. Run: ntnt run program.tnt search "*.txt" ./testdir
3. Capture stdout, stderr, exit code
4. Check exit_code == 0 ✓
5. Check "found 3 files" in stdout ✓
6. Report: PASS
```

### Script Tests (Data Processing, Automation)

For scripts that process data or perform tasks:

```yaml
Feature: Data Migration
  description: Convert old format to new format
  test:
    - run_with_input:
        stdin: '{"legacy": true, "value": 42}'
      assert:
        - stdout is_json
        - stdout.json.migrated == true
        - stdout.json.data.value == 42

    - run_with_files:
        input: "testdata/old_format.json"
        output: "testdata/expected_new.json"
      assert:
        - output_matches_expected
```

### HTTP Server Tests

For web applications, the engine starts the server and makes requests:

```yaml
Feature: Site Selection
  description: User can select a monitoring site
  test:
    - request: GET /
      assert:
        - status: 200
        - body contains "Bear Lake"
        - body contains "Wild Basin"

    - request: POST /select
      body: "site=bear_lake"
      assert:
        - status: 302
        - header "Location" == "/dashboard"
```

Engine executes:

```
1. Start server on random port
2. GET http://localhost:54321/
3. Check response.status == 200 ✓
4. Check "Bear Lake" in body ✓
5. Shutdown server
6. Report: PASS
```

### Background Worker Tests

For workers, daemons, and long-running processes:

```yaml
Feature: Queue Processor
  description: Process jobs from a queue
  test:
    - start_worker
      with_queue: [job1, job2, job3]
      wait_until: queue_empty
      timeout: 5s
      assert:
        - processed_count == 3
        - error_count == 0

    - start_worker
      with_queue: [valid_job, invalid_job, valid_job]
      wait_until: queue_empty
      assert:
        - processed_count == 2
        - error_count == 1
        - error_log contains "invalid_job"
```

### Database Operation Tests

For code that interacts with databases:

```yaml
Feature: User Registration
  description: Create new user accounts
  test:
    - call: register_user("alice@test.com", "Alice")
      with_db: test_database
      assert:
        - result.id > 0
        - query("SELECT * FROM users WHERE email = 'alice@test.com'").count == 1

    - call: register_user("alice@test.com", "Duplicate")
      with_db: test_database
      assert:
        - throws: DuplicateEmailError
```

Database tests automatically:

- Use a test database or transactions
- Roll back changes after each test
- Provide query assertions

### File System Tests

For code that reads/writes files:

```yaml
Feature: Log Rotation
  description: Rotate logs when they exceed size limit
  test:
    - setup_files:
        "app.log": "x" * 1000000  # 1MB file
      call: rotate_logs("app.log", max_size: 500000)
      assert:
        - file_exists("app.log.1")
        - file_size("app.log") < 500000

    - setup_files:
        "app.log": "small content"
      call: rotate_logs("app.log", max_size: 500000)
      assert:
        - not file_exists("app.log.1")  # No rotation needed
```

File tests automatically:

- Create temp directories
- Set up fixture files
- Clean up after tests

### Common Assertion Types

These assertions work across all test types:

**Value assertions**

```yaml
assert:
  - result == expected # Exact equality
  - result != bad_value # Inequality
  - result > 0 # Numeric comparison
  - result contains "substring" # String/list contains
  - result matches r"\d+" # Regex match
  - result is_empty # Empty check
  - len(result) == 5 # Length check
```

**Type assertions**

```yaml
assert:
  - result is_string
  - result is_int
  - result is_list
  - result is_map
  - result is_json # Valid JSON string
  - result is_none # None/null value
```

**Error assertions**

```yaml
assert:
  - throws: ErrorType # Specific error
  - throws # Any error
  - not throws # No error (success)
  - error_message contains "invalid"
```

**Timing assertions**

```yaml
assert:
  - duration < 100ms # Performance check
  - duration > 0ms # Actually ran
```

### Data Schema Validation

For data structures defined in the intent:

```yaml
Data: sites
  type: map
  required_keys: [bear_lake, wild_basin, copeland_lake]
  value_type: string (URL format)
```

Engine executes:

```
1. Find "sites" definition in code
2. Extract keys: ["bear_lake", "wild_basin"]
3. Compare: missing "copeland_lake"
4. Report: WARN - missing required key
```

### Constraint Tests

For non-functional requirements:

**Static analysis** (no runtime needed):

```yaml
Constraint: Single File
  verify: all code in one .tnt file

# Engine: Count .tnt files, check == 1
```

**Runtime analysis**:

```yaml
Constraint: Fast Startup
  verify: program starts in under 100ms

# Engine: Time startup, check < 100ms
```

**Resource analysis**:

```yaml
Constraint: Memory Efficient
  verify: peak memory under 50MB

# Engine: Monitor memory during test run
```

### Mock and Simulation

For testing error handling and edge cases:

```bash
$ ntnt intent check myapp.tnt --mock
```

Engine can simulate:

- Network failures (timeouts, connection refused)
- External API errors (500s, rate limits)
- File system issues (permission denied, disk full)
- Database failures (connection lost, constraint violations)

```yaml
Feature: Graceful Degradation
  test:
    - call: fetch_weather()
      mock:
        http("api.weather.com"): timeout
      assert:
        - result == default_weather
        - log contains "API timeout, using cached data"
```

### Test Isolation

Each test runs in isolation:

- Fresh program instance per test
- No shared state between tests
- Database tests use transactions (rolled back)
- File tests use temp directories (cleaned up)
- Environment variables reset between tests

### Parallel Execution

For speed, independent tests run in parallel:

```bash
$ ntnt intent check app.tnt --parallel

Running 12 tests across 4 workers...
[====================================] 100%

Results: 12/12 passed (2.3s)
```

### Failure Output

When tests fail, the engine provides actionable output:

```
[FAIL] Feature: CSV Parsing (2/3 tests passed)

  ✓ parse_csv with valid data returns records
  ✓ parse_csv with empty string returns []
  ✗ parse_csv with invalid data throws ParseError

    Expected: throws ParseError
    Actual:   returned [{"data": "missing"}] (no error)

    Hint: Intent says "invalid CSV should throw ParseError"
          but implementation silently handles malformed data.

    Intent location: csvlib.intent:23 (CSV Parsing)
    Code location:   csvlib.tnt:45 (parse_csv)
```

---

## Visual and Interactive Testing

Many NTNT apps serve web frontends that include third-party technologies: HTML/CSS, JavaScript frameworks (React, Vue, Alpine.js), data visualization (Chart.js, D3.js, Three.js), video, WebGL, and more. These can't be tested with simple string assertions like `body contains "canvas"`. You need to verify:

- "Does the chart actually render correctly?"
- "Is the layout responsive on mobile?"
- "Does the animation feel smooth?"
- "Is the color scheme accessible?"

### Multi-Layer Testing Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    INTENT TESTING LAYERS                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Layer 4: LLM Visual Verification (AI interprets screenshots)   │
│     "Does this look like a snow depth chart?"                   │
│     "Is the error message user-friendly?"                       │
│                                                                  │
│  Layer 3: Browser Automation (Puppeteer/Playwright)             │
│     Click buttons, fill forms, measure performance              │
│     Screenshot comparison, accessibility audits                  │
│                                                                  │
│  Layer 2: DOM Assertions (Structure verification)               │
│     Element exists, has attributes, computed styles             │
│                                                                  │
│  Layer 1: HTTP Response (Current implementation)                │
│     Status codes, headers, body content                         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Layer 2: DOM Assertions

Test the structure and properties of rendered HTML:

```yaml
Feature: Snow Chart
  id: snow_chart

  test "chart container exists":
    request: GET /
    browser:
      - element "#snow-chart" exists
      - element "#snow-chart" has_tag "canvas"
      - element "#snow-chart" has_attribute "width"

  test "chart has data":
    request: GET /
    browser:
      - element ".chart-legend" exists
      - elements ".data-point" count >= 30

  test "responsive layout":
    request: GET /
    browser:
      - viewport 375x667  # iPhone SE
      - element "#snow-chart" width <= 375
      - element ".nav-menu" is_visible false  # Collapsed on mobile
    browser:
      - viewport 1920x1080  # Desktop
      - element ".nav-menu" is_visible true
```

**Implementation:** Uses headless browser (Puppeteer/Playwright) to render page and query DOM.

### Layer 3: Browser Automation

Test interactive behavior:

```yaml
Feature: Site Selection
  id: site_selection

  test "dropdown interaction":
    request: GET /
    browser:
      - click "#site-selector"
      - wait_for "#dropdown-menu" is_visible
      - click "option[value='wild_basin']"
      - wait_for_navigation
      - url contains "?site=wild_basin"
      - element "#site-name" text_content == "Wild Basin"

  test "form submission":
    request: GET /contact
    browser:
      - fill "#name" with "Alice"
      - fill "#email" with "alice@example.com"
      - fill "#message" with "Hello!"
      - click "#submit-btn"
      - wait_for ".success-message" is_visible
      - element ".success-message" text_content contains "Thank you"

  test "keyboard navigation":
    request: GET /
    browser:
      - press "Tab" 3 times
      - focused_element has_id "site-selector"
      - press "Enter"
      - element "#dropdown-menu" is_visible

  test "animation performance":
    request: GET /
    browser:
      - start_tracing
      - click "#animate-btn"
      - wait 2 seconds
      - stop_tracing
      - frame_rate >= 30  # Smooth animation
      - largest_contentful_paint < 2500ms
```

### Layer 3.5: Accessibility Testing

Built-in accessibility verification:

```yaml
Constraint: Accessibility
  id: accessibility

  test "WCAG compliance":
    request: GET /
    browser:
      accessibility_audit:
        - color_contrast >= 4.5  # WCAG AA
        - all_images_have_alt true
        - all_inputs_have_labels true
        - heading_hierarchy valid
        - no_empty_links true

  test "screen reader friendly":
    request: GET /
    browser:
      - element "#main-content" has_attribute "role" == "main"
      - element "#nav" has_attribute "aria-label"
      - elements "button" all_have_attribute "aria-label" or text_content

  test "keyboard accessible":
    request: GET /
    browser:
      - tab_order_matches ["#skip-link", "#nav", "#site-selector", "#main-content"]
      - all_interactive_elements focusable
      - no_keyboard_traps
```

### Layer 4: LLM Visual Verification

Use AI to interpret visual output and verify subjective qualities:

```yaml
Feature: Snow Chart
  id: snow_chart

  test "chart renders correctly":
    request: GET /
    browser:
      - screenshot "#snow-chart" as "chart.png"
    llm_verify "chart.png":
      prompt: |
        This should be a line chart showing snow depth over time.
        Verify:
        - There is a line chart visible
        - X-axis shows dates
        - Y-axis shows depth in inches
        - The line has data points (not empty)
        - Colors are visible and distinguishable
      confidence: 0.85  # Require 85% confidence

  test "error state looks correct":
    mock: SNOTEL returns 500
    request: GET /
    browser:
      - screenshot "body" as "error.png"
    llm_verify "error.png":
      prompt: |
        This should show a user-friendly error state.
        Verify:
        - There is an error message visible
        - The message is polite/helpful (not technical jargon)
        - The page layout is intact (not broken)
        - There's a way to retry or go back
      confidence: 0.80

  test "mobile layout is usable":
    request: GET /
    browser:
      - viewport 375x667
      - screenshot "body" as "mobile.png"
    llm_verify "mobile.png":
      prompt: |
        This should be a mobile-friendly layout.
        Verify:
        - Content is not cut off or overflowing
        - Text is readable (not too small)
        - Buttons/links are tap-friendly (not tiny)
        - No horizontal scrolling required
        - Navigation is accessible (hamburger menu or similar)
      confidence: 0.80
```

### How LLM Verification Works

```
┌─────────────────────────────────────────────────────────────────┐
│                   LLM VISUAL VERIFICATION                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. CAPTURE                                                      │
│     Browser renders page → Screenshot taken → PNG/JPEG           │
│                                                                  │
│  2. PREPARE                                                      │
│     Intent prompt + Screenshot → Multimodal LLM request          │
│                                                                  │
│  3. ANALYZE                                                      │
│     LLM (GPT-4V, Claude Vision, Gemini) inspects image          │
│     Returns: { "pass": true/false, "confidence": 0.92,          │
│                "observations": [...], "issues": [...] }          │
│                                                                  │
│  4. DECIDE                                                       │
│     if confidence >= threshold AND pass == true:                 │
│         TEST PASSES                                              │
│     else:                                                        │
│         TEST FAILS with observations                             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Example LLM interaction:**

```json
// Request to LLM
{
  "model": "gpt-4-vision-preview",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "image_url",
          "image_url": { "url": "data:image/png;base64,..." }
        },
        {
          "type": "text",
          "text": "This should be a line chart showing snow depth over time. Verify: [criteria from intent]. Respond with JSON: {pass: bool, confidence: float, observations: string[], issues: string[]}"
        }
      ]
    }
  ]
}

// Response from LLM
{
  "pass": true,
  "confidence": 0.94,
  "observations": [
    "Line chart is visible with blue line",
    "X-axis shows dates from Dec 15 to Jan 14",
    "Y-axis shows 'Snow Depth (inches)' with scale 0-60",
    "Approximately 30 data points visible",
    "Legend shows 'Bear Lake' label"
  ],
  "issues": []
}
```

### Visual Regression Testing

Compare screenshots over time to catch unintended changes:

```yaml
Constraint: Visual Consistency
  id: visual_consistency

  test "homepage hasn't changed unexpectedly":
    request: GET /
    browser:
      - screenshot "body" as "current.png"
    visual_diff "current.png" against "baseline/homepage.png":
      threshold: 0.05  # Allow 5% pixel difference
      ignore_regions: [".timestamp", ".dynamic-ad"]

  test "chart style is consistent":
    request: GET /
    browser:
      - screenshot "#snow-chart" as "chart-current.png"
    visual_diff "chart-current.png" against "baseline/chart.png":
      threshold: 0.10  # Charts may vary slightly with data

# Update baseline when intentional changes are made:
# ntnt intent update-baseline snow_chart
```

### Testing Third-Party Frameworks

Specific patterns for common frameworks:

```yaml
# Chart.js
Feature: Interactive Chart
  id: interactive_chart

  test "Chart.js initializes":
    request: GET /
    browser:
      - wait_for_js "window.snowChart !== undefined"
      - js_eval "window.snowChart.data.datasets.length" == 1
      - js_eval "window.snowChart.data.labels.length" >= 30

  test "chart tooltip works":
    request: GET /
    browser:
      - hover "#snow-chart" at 50% 50%
      - wait_for ".chartjs-tooltip" is_visible
      - element ".chartjs-tooltip" text_content matches r"\d+ inches"
```

```yaml
# Three.js / WebGL
Feature: 3D Visualization
  id: 3d_visualization

  test "WebGL renders":
    request: GET /3d-view
    browser:
      - wait_for_js "window.scene !== undefined"
      - js_eval "window.renderer.info.render.triangles" > 0
      - screenshot "#canvas-3d" as "3d-render.png"
    llm_verify "3d-render.png":
      prompt: |
        This should show a 3D terrain visualization.
        Verify:
        - 3D perspective is visible (not flat)
        - Terrain/mountain shapes are rendered
        - Colors indicate elevation (snow on peaks)
      confidence: 0.75  # 3D is harder to verify

  test "3D interaction":
    request: GET /3d-view
    browser:
      - drag "#canvas-3d" from 50%,50% to 70%,30%
      - wait 500ms
      - screenshot "#canvas-3d" as "rotated.png"
    visual_diff "rotated.png" against "3d-render.png":
      should_differ: true  # Rotation should change the view
      min_difference: 0.20
```

```yaml
# Video Player
Feature: Video Player
  id: video_player

  test "video loads and plays":
    request: GET /tutorial
    browser:
      - element "video" exists
      - element "video" has_attribute "src"
      - js_eval "document.querySelector('video').readyState" >= 2
      - click "video"  # Play
      - wait 2 seconds
      - js_eval "document.querySelector('video').currentTime" > 0
      - js_eval "document.querySelector('video').paused" == false
```

### Frontend Performance Testing

Measure Core Web Vitals and performance metrics:

```yaml
Constraint: Performance
  id: frontend_performance

  test "page loads quickly":
    request: GET /
    browser:
      performance:
        - first_contentful_paint < 1500ms
        - largest_contentful_paint < 2500ms
        - time_to_interactive < 3000ms
        - cumulative_layout_shift < 0.1

  test "chart renders efficiently":
    request: GET /
    browser:
      - start_profiling
      - wait_for "#snow-chart canvas"
      - stop_profiling
      profile:
        - js_heap_size < 50MB
        - dom_nodes < 1000
        - js_execution_time < 500ms

  test "no memory leaks":
    request: GET /
    browser:
      - js_eval "performance.memory.usedJSHeapSize" as initial_memory
      - repeat 5:
          - navigate "/about"
          - navigate "/"
      - js_eval "performance.memory.usedJSHeapSize" as final_memory
      - assert final_memory < initial_memory * 1.5  # Max 50% growth
```

### Configuration for Visual Testing

```yaml
# In myapp.intent

Test Configuration:
  visual_testing:
    browser: chromium # or firefox, webkit
    headless: true
    viewport: 1280x720 # Default viewport

    # LLM provider for visual verification
    llm_provider: openai
    llm_model: gpt-4-vision-preview
    llm_confidence_threshold: 0.80

    # Baseline management
    baseline_dir: "./test/baselines"
    auto_update_baselines: false

    # Screenshot settings
    screenshot_format: png
    screenshot_full_page: false

    # Performance budgets
    performance:
      max_fcp: 1500ms
      max_lcp: 2500ms
      max_tti: 3000ms
      max_cls: 0.1
```

### CLI Commands for Visual Testing

```bash
# Run all tests including visual
$ ntnt intent check myapp.tnt --visual

# Run only visual tests
$ ntnt intent check myapp.tnt --only visual

# Update visual baselines after intentional changes
$ ntnt intent update-baseline homepage
$ ntnt intent update-baseline --all

# Run with visible browser (debugging)
$ ntnt intent check myapp.tnt --visual --headed

# Generate visual test report with screenshots
$ ntnt intent check myapp.tnt --visual --report ./test-report/

# Skip LLM verification (faster, less thorough)
$ ntnt intent check myapp.tnt --visual --no-llm
```

### Cost and Performance Considerations

LLM visual verification has tradeoffs:

| Aspect          | Consideration                                                                             |
| --------------- | ----------------------------------------------------------------------------------------- |
| **Cost**        | Each LLM vision call costs ~$0.01-0.03. Budget-conscious projects can limit LLM tests.    |
| **Speed**       | LLM calls add 2-5 seconds per test. Run with `--no-llm` for fast feedback.                |
| **Reliability** | LLM responses can vary. Use confidence thresholds and multiple checks for critical tests. |
| **Privacy**     | Screenshots sent to LLM provider. Use `llm_provider: local` for sensitive UIs.            |

**Recommended strategy:**

- Use DOM assertions (Layer 2) for most tests - fast, deterministic
- Use LLM verification (Layer 4) for subjective/complex visuals - error states, overall UX
- Run full visual tests in CI, quick tests locally
- Cache LLM responses for unchanged screenshots

### What This Enables

With visual and interactive testing, intent files can express subjective UX qualities:

```yaml
Feature: User Dashboard
  id: user_dashboard
  description: "A clean, modern dashboard for viewing personal stats"

  # Traditional assertions
  test "data loads":
    request: GET /dashboard
    assert:
      - status: 200
      - body contains_json_path "$.stats.visits"

  # DOM structure
  test "layout is correct":
    browser:
      - element ".dashboard-grid" exists
      - elements ".stat-card" count == 4
      - element ".chart-container" has_child "canvas"

  # Interactive behavior
  test "date picker works":
    browser:
      - click "#date-range"
      - click ".preset-last-30-days"
      - wait_for ".chart-container" content_changes

  # Subjective UX quality (LLM)
  test "dashboard looks professional":
    browser:
      - screenshot "body" as "dashboard.png"
    llm_verify "dashboard.png":
      prompt: |
        This should be a professional-looking analytics dashboard.
        Verify:
        - Clean, modern design (not cluttered)
        - Data visualizations are clear and readable
        - Consistent color scheme
        - Proper spacing and alignment
        - No obvious UI bugs (overlapping elements, cut-off text)
      confidence: 0.80
```

**The human can now say "I want a professional-looking dashboard" and the system can actually verify that intent** - something impossible with traditional testing.

---

## Beyond "Clean and Modern": Concrete UI/UX Intent

### The Problem with Aesthetic Adjectives

Vague terms like "clean," "modern," "professional," and "user-friendly" are **exactly the kind of intent that leads to bad AI-generated UIs**. The agent thinks it's doing great because:

- It added rounded corners and drop shadows → "modern" ✓
- It used a blue color scheme → "professional" ✓
- It centered everything → "clean" ✓

But what you actually got was:

- 40% of the screen consumed by UI chrome and decorations
- Important data buried below the fold
- Exceptions indistinguishable from normal values
- Equal visual weight given to everything
- No clear information hierarchy

**The LLM verifier has the same problem** - it will happily confirm that the dashboard "looks modern" because it matches the LLM's generic mental model of "modern."

### The Solution: Specific, Testable UI Intent

Instead of aesthetic adjectives, express **concrete requirements** about:

1. **Data hierarchy** - What's most important?
2. **Spatial priority** - Where should attention go first?
3. **Exception handling** - How are anomalies surfaced?
4. **Content-to-chrome ratio** - How much space for actual data?
5. **Information density** - How much data per viewport?

### Intent File: UI/UX Section

Add a dedicated UI/UX section to your intent file:

```yaml
## UI/UX

### Information Hierarchy
# What data matters most? List in priority order.

Primary (must be visible without scrolling):
  - current_snow_depth: "The single most important number - largest text on page"
  - trend_direction: "Is it increasing or decreasing? Show arrow or indicator"
  - last_updated: "When was this data captured?"

Secondary (visible but not emphasized):
  - 7_day_chart: "Sparkline or small chart showing recent trend"
  - site_name: "Which site is selected"

Tertiary (available on demand):
  - 30_day_history: "Full chart, can scroll to see"
  - raw_data_table: "Detailed numbers, expandable"
  - data_source_info: "Where does this come from?"

### Exception Highlighting
# How should anomalies be surfaced?

Exceptions:
  - value_above_threshold:
      condition: "snow_depth > 48 inches"
      treatment: "Yellow background, show warning icon"
  - value_extreme:
      condition: "snow_depth > 72 inches"
      treatment: "Red background, prominent alert"
  - data_stale:
      condition: "last_updated > 6 hours ago"
      treatment: "Gray out data, show 'Data may be outdated' warning"
  - no_data:
      condition: "API returned no data"
      treatment: "Clear message, not an error page - show last known value"

### Layout Principles
# How should space be used?

Spatial Rules:
  - above_fold: "Primary data visible without scrolling on 768px viewport"
  - content_ratio: "At least 70% of viewport is actual data, not UI chrome"
  - whitespace: "Adequate breathing room, but not excessive padding"
  - navigation: "Minimal - site selector only, no elaborate nav bars"

### Anti-Patterns
# What should be explicitly avoided?

Avoid:
  - hero_sections: "No large header images or welcome banners"
  - decorative_elements: "No icons/graphics that don't convey information"
  - equal_emphasis: "Not everything should look the same"
  - hidden_data: "Don't put essential data in tooltips or modals"
  - style_over_substance: "No drop shadows, gradients, or effects that don't serve function"
```

### Tests That Verify Concrete Intent

Now the LLM verifier has specific criteria instead of vibes:

```yaml
Feature: Dashboard Layout
  id: dashboard_layout

  test "primary data is prominent":
    request: GET /dashboard
    browser:
      - screenshot "body" as "dashboard.png"
    llm_verify "dashboard.png":
      prompt: |
        Verify information hierarchy:
        1. Is there a large, prominent number showing current snow depth?
           It should be the largest text element on the page.
        2. Is there a clear trend indicator (arrow, +/-, or similar)?
        3. Is the "last updated" timestamp visible without scrolling?

        FAIL if: Snow depth is same size as other text, or buried in a table,
                 or requires scrolling to see.
      confidence: 0.90

  test "content-to-chrome ratio":
    request: GET /dashboard
    browser:
      - viewport 1280x720
      - screenshot "body" as "full.png"
    llm_verify "full.png":
      prompt: |
        Analyze space usage:
        1. What percentage of the visible area is actual DATA vs UI elements?
           (headers, nav bars, decorative elements, excessive padding)
        2. Is there a large hero section or banner taking up space?
        3. Are there decorative graphics that don't convey information?

        PASS if: At least 70% of viewport is data/content
        FAIL if: UI chrome, padding, or decorations dominate the view
      confidence: 0.85

  test "exceptions are highlighted":
    # Simulate high snow condition
    mock: snow_depth = 60
    request: GET /dashboard
    browser:
      - screenshot "body" as "exception.png"
    llm_verify "exception.png":
      prompt: |
        The snow depth value (60 inches) exceeds the warning threshold (48 inches).
        Verify:
        1. Is the snow depth number visually distinct? (different color, background, icon)
        2. Is there a warning indicator visible?
        3. Does the exception stand out from normal values?

        FAIL if: The high value looks identical to normal values
      confidence: 0.85

  test "no decorative bloat":
    request: GET /dashboard
    browser:
      - screenshot "body" as "dashboard.png"
    llm_verify "dashboard.png":
      prompt: |
        Check for unnecessary visual elements:
        1. Are there decorative icons that don't convey data?
        2. Is there a large hero image or welcome banner?
        3. Are there drop shadows, gradients, or visual effects?
        4. Is there excessive rounded corners or card styling?

        PASS if: Design is minimal and functional
        FAIL if: Decorative elements consume significant space or attention
      confidence: 0.80
```

### DOM-Based Layout Verification

Combine LLM checks with deterministic DOM assertions:

```yaml
test "data appears above fold":
  request: GET /dashboard
  browser:
    - viewport 1280x768
    - element "#snow-depth-value" is_visible
    - element "#snow-depth-value" bounding_box: top < 400 # In upper half of viewport
    - element "#snow-depth-value" font_size >= 32px
    - element "#trend-indicator" is_visible
    - element "#last-updated" is_visible

test "minimal navigation":
  request: GET /dashboard
  browser:
    - element "nav" height < 80px # Not a giant nav bar
    - elements "nav a" count <= 5 # Not too many nav items
    - not element ".hero" # No hero sections
    - not element ".banner" # No banners

test "content ratio":
  request: GET /dashboard
  browser:
    - viewport 1280x720
    # Main content area should be most of the viewport
    - element "main" bounding_box: width >= 900
        height >= 500
    # Header should be minimal
    - element "header" height < 100
```

### Style Reference: Showing What You Mean

For truly specific intent, include reference examples:

```yaml
## UI/UX

### Style Reference

Good Examples (emulate these):
  - url: "https://example.com/minimal-dashboard.png"
    what_i_like: |
      - Single prominent metric at top
      - Sparkline charts, not full charts
      - No sidebar, just top nav
      - Data table is compact, not styled

  - url: "https://example.com/data-dense-ui.png"
    what_i_like: |
      - High information density
      - Small fonts for secondary data
      - Color only for exceptions
      - No decorative elements

Bad Examples (avoid these):
  - url: "https://example.com/over-designed-dashboard.png"
    what_to_avoid: |
      - Giant cards with drop shadows
      - Icons next to every label
      - Gradient backgrounds
      - Equal sizing for all metrics

  - url: "https://example.com/low-density-ui.png"
    what_to_avoid: |
      - Huge padding everywhere
      - One metric per card
      - Scroll required for basic info
      - Style over substance
```

The LLM verifier can then compare against these references:

```yaml
test "matches style intent":
  request: GET /dashboard
  browser:
    - screenshot "body" as "dashboard.png"
  llm_verify "dashboard.png":
    reference_good: ["minimal-dashboard.png", "data-dense-ui.png"]
    reference_bad: ["over-designed-dashboard.png", "low-density-ui.png"]
    prompt: |
      Compare this dashboard to the reference images.
      It should be MORE like the good examples and LESS like the bad examples.

      Key criteria from the intent:
      - High information density
      - Minimal decorative styling
      - Clear data hierarchy (one metric prominent)
      - Exceptions visually distinct

      Score similarity to good examples (0-100): ?
      Score similarity to bad examples (0-100): ?

      PASS if: good_score > 70 AND bad_score < 30
    confidence: 0.85
```

### The Taste Problem

Even with specific intent, "taste" is hard to define. Some strategies:

**1. Iterative Refinement**

```yaml
## UI/UX

### Learned Preferences
# Updated as human provides feedback

Previous Feedback:
  - iteration_1: "Too much padding, cards too large"
  - iteration_2: "Better, but the chart dominates - make it smaller"
  - iteration_3: "Good density, but exceptions not visible enough"

Current Rules (derived from feedback):
  - padding: "8px max between elements"
  - cards: "No cards - use subtle borders or none"
  - charts: "Sparkline size (100px height max) unless expanded"
  - exceptions: "Red text + icon, not just color"
```

**2. Component-Level Approval**

Instead of approving/rejecting whole pages, approve components:

```yaml
## UI/UX

### Approved Components
# Human-approved patterns to reuse

MetricDisplay:
  approved: true
  screenshot: "components/metric-display-approved.png"
  rules: |
    - Large number (32px+)
    - Unit below in smaller text
    - Trend arrow to right
    - No background/card

DataTable:
  approved: true
  screenshot: "components/data-table-approved.png"
  rules: |
    - Compact rows (32px height)
    - Alternating backgrounds (subtle)
    - No borders between cells
    - Header row is bold, same size

SparklineChart:
  approved: true
  screenshot: "components/sparkline-approved.png"
  rules: |
    - 100px wide, 40px tall
    - Single color line
    - No axes, no labels
    - Hover shows value
```

**3. Constraint-Based Design**

Express intent as constraints rather than aesthetics:

```yaml
## UI/UX

### Hard Constraints
# These are measurable and must pass

Layout:
  - viewport_768_shows: ["snow_depth", "trend", "last_updated"]
  - max_scroll_to_essential: 0px # No scrolling for essential data
  - header_height: "<= 60px"
  - nav_items: "<= 3"
  - sidebar: "none"

Typography:
  - primary_metric_size: ">= 48px"
  - secondary_text_size: "14-16px"
  - font_weights_used: "<= 2" # Regular and bold only
  - font_families_used: "1" # Single font family

Spacing:
  - max_padding: "16px"
  - max_margin: "24px"
  - max_gap: "16px"

Color:
  - colors_used: "<= 5" # Not a rainbow
  - decorative_colors: "0" # Color only for meaning
  - exception_color: "distinct from normal"

Elements:
  - drop_shadows: "0"
  - gradients: "0"
  - border_radius: "<= 4px"
  - decorative_icons: "0"
```

These constraints can be verified deterministically:

```yaml
test "typography constraints":
  request: GET /dashboard
  browser:
    - element "#snow-depth-value" font_size >= 48px
    - elements "body *" max_font_size <= 64px # Not absurdly large
    - elements "body *" distinct_font_families <= 1
    - elements "body *" distinct_font_weights <= 2

test "spacing constraints":
  request: GET /dashboard
  browser:
    - elements "*" max_padding <= 16px
    - elements "*" max_margin <= 24px

test "no decorative styling":
  request: GET /dashboard
  browser:
    - elements "*" box_shadow == "none"
    - elements "*" background_image == "none"
    - elements "*" border_radius <= 4px
```

### Summary: From Vibes to Verification

| Bad Intent (Vibes)         | Good Intent (Verifiable)                                       |
| -------------------------- | -------------------------------------------------------------- |
| "Clean design"             | "Content-to-chrome ratio >= 70%"                               |
| "Modern look"              | "No drop shadows, gradients, or decorative elements"           |
| "User-friendly"            | "Primary data visible without scrolling on 768px viewport"     |
| "Professional"             | "Consistent font family, <= 2 font weights, <= 5 colors"       |
| "Highlight important data" | "Snow depth >= 48px font size, exceptions have red background" |
| "Good spacing"             | "Max padding 16px, max margin 24px"                            |

The key insight: **Every aesthetic preference can be decomposed into concrete, testable constraints.** The intent file is where you do that decomposition, not where you use subjective adjectives.

---

## Handling Undocumented Code

Not every line of code maps to a feature. The system handles this gracefully.

### Default Behavior: Warn, Don't Fail

Code without `@implements` or `@supports` annotations is **allowed but flagged**:

```
$ ntnt intent coverage snowgauge.tnt

Code Linkage Report

Linked (documented purpose):
  ✓ home_handler          @implements: feature.site_selection
  ✓ fetch_snow_data       @implements: feature.snow_display
  ✓ render_chart          @implements: feature.chart

Unlinked (review recommended):
  ? extract_snotel_name   No annotation - consider @utility or @supports
  ? parse_csv_row         No annotation - consider @utility or @supports
  ? format_inches         No annotation - consider @utility or @supports

Coverage: 50% (3/6 functions)
Status: PASS with warnings
```

Key points:

- **Doesn't fail the build** - unlinked code is allowed
- **Generates warnings** - so it's visible for review
- **Suggests actions** - add annotation or review if needed

### Suppressing Warnings

For intentionally undocumented code, use markers:

```ntnt
// @utility - helper function, no specific feature
fn format_inches(value: Float) -> String {
    return str(round(value, 1)) + "\""
}

// @internal - implementation detail, not user-facing
fn parse_csv_row(line: String) -> List[String] {
    return split(line, ",")
}

// @deprecated - legacy code, will be removed
fn old_snow_parser(data: String) -> Map {
    // ...
}
```

These markers tell the coverage tool "I know this isn't linked, and that's intentional."

### Coverage Report Categories

The full coverage report shows:

```
$ ntnt intent coverage snowgauge.tnt --detailed

═══════════════════════════════════════════════════
  Intent Coverage Report: snowgauge.tnt
═══════════════════════════════════════════════════

FEATURE IMPLEMENTATIONS (@implements: feature.*)
──────────────────────────────────────────────────
  ✓ home_handler        → feature.site_selection
  ✓ fetch_snow_data     → feature.snow_display
  ✓ render_chart        → feature.chart

SUPPORTING CODE (@supports: *)
──────────────────────────────────────────────────
  ✓ log_request         → feature.site_selection, feature.snow_display
  ✓ db_pool             → infra.database

UTILITY CODE (@utility)
──────────────────────────────────────────────────
  ○ format_inches       (documented as utility)
  ○ parse_csv_row       (documented as utility)

UNLINKED CODE (needs review)
──────────────────────────────────────────────────
  ? extract_snotel_name   Line 45 - no annotation
  ? validate_input        Line 89 - no annotation

──────────────────────────────────────────────────
Summary:
  Feature coverage:  100% (3/3)
  Code linkage:      75% (6/8 functions documented)
  Unlinked:          2 functions need review

Status: PASS (warnings present)
```

### Configuration Options

Control strictness via config:

```yaml
# .ntnt/config.yaml

intent:
  # How to handle unlinked code
  unlinked_code: warn # warn (default), allow, deny

  # Minimum coverage threshold
  min_coverage: 80% # Fail if below this

  # Require all public functions to be linked
  require_public_linked: true

  # Ignore patterns (never warn about these)
  ignore_patterns:
    - "*_test" # Test helpers
    - "debug_*" # Debug functions
```

### Strictness Levels

Different projects need different strictness:

**Relaxed** (prototyping, learning):

```yaml
intent:
  unlinked_code: allow
  min_coverage: 0%
```

**Standard** (most projects):

```yaml
intent:
  unlinked_code: warn
  min_coverage: 50%
```

**Strict** (production, compliance):

```yaml
intent:
  unlinked_code: deny
  min_coverage: 90%
  require_public_linked: true
```

### Why Not Require Everything?

Some code legitimately has no feature mapping:

- Generic utilities (string formatting, math helpers)
- Framework boilerplate (setup, teardown)
- Generated code (scaffolding)
- Debug/development helpers

Requiring annotations for everything would:

- Create friction for simple changes
- Generate meaningless annotations ("this formats a string")
- Discourage refactoring into small functions

The **warn by default** approach balances:

- Visibility (you know what's not linked)
- Pragmatism (doesn't block legitimate code)
- Intentionality (you can document if needed)

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

## Team Collaboration

IDD isn't just about human-agent collaboration—it fundamentally changes how **humans collaborate with each other** on software teams.

### The Clarity Problem

Most software projects suffer from fuzzy requirements:

- **Product managers** write user stories that engineers interpret differently
- **Stakeholders** have expectations that never get documented
- **Engineers** make assumptions that don't match business needs
- **QA** tests against their understanding, which may differ from everyone else's

The result: endless meetings, rework, and "that's not what I meant."

### How IDD Forces Clarity

The intent file requires you to answer hard questions **upfront**:

| Vague Requirement          | Intent Forces You to Specify                                   |
| -------------------------- | -------------------------------------------------------------- |
| "Users can filter results" | Filter by what? Which fields? Multiple filters? Default state? |
| "Fast performance"         | What's fast? Under what load? Measured how?                    |
| "Mobile-friendly"          | Responsive? Native? What breakpoints? Touch targets?           |
| "Handle errors gracefully" | Which errors? What message? Retry? Fallback?                   |

You can't write a testable assertion for "fast performance." You CAN write one for "GET /api/users returns within 200ms for 1000 concurrent users."

### Role-by-Role Impact

#### Product Managers

**Before IDD:**

```
User Story: As a user, I want to select my location so I can see local data.

Acceptance Criteria:
- User can select location
- Shows local data
```

**With IDD:**

```intent
### Feature: Location Selection
id: location_selection
Priority: Must Have

Users select their location to see locally-relevant weather data.

Behavior:
- Dropdown shows all available locations (sorted alphabetically)
- Selection persists in URL as ?location=<key>
- Invalid/missing location falls back to "denver" (not error page)
- Current location highlighted in dropdown

Tests:
- GET /?location=boulder -> 200, contains "Boulder"
- GET /?location=invalid -> 200, contains "Denver"
- GET / -> 200, contains "Denver"
```

**The difference:** PM must think through edge cases (invalid location), defaults, persistence, and UI behavior. No ambiguity for engineers to interpret.

**Benefits for PMs:**

- Forces complete thinking before development starts
- Creates a reviewable artifact stakeholders can approve
- Reduces "that's not what I meant" moments
- Provides clear acceptance criteria that aren't subjective

#### Stakeholders (Executives, Clients, Business Owners)

**Before IDD:**

- Review PRDs full of jargon
- See demos and say "that's not right"
- Can't verify the software meets requirements without using it
- Trust that engineers interpreted requirements correctly

**With IDD:**

```
$ ntnt intent check app.tnt

Features:
  ✅ Location Selection (4/4 tests)
  ✅ Weather Display (3/3 tests)
  ✅ 7-Day Forecast (2/2 tests)

All requirements verified.
```

**Benefits for stakeholders:**

- Readable intent file serves as living documentation
- Verification report proves requirements are met
- Don't need to understand code to trust the software
- Can review and edit requirements directly (it's just text)

#### Software Engineers

**Before IDD:**

- Interpret vague requirements
- Make assumptions, hope they're right
- Get requirements changed mid-sprint
- Argue about what "done" means

**With IDD:**

- Clear, testable requirements before coding starts
- Know exactly when a feature is complete (tests pass)
- Push back on vague requirements: "What should the test be?"
- Automated verification = confidence to refactor

**Benefits for engineers:**

- Less time in meetings clarifying requirements
- Unambiguous definition of "done"
- Refactor fearlessly—intent tests catch regressions
- Focus on implementation, not interpretation

#### QA / Testers

**Before IDD:**

- Write test cases from their interpretation of requirements
- Discover edge cases during testing that were never specified
- "Is this a bug or a feature?" debates

**With IDD:**

- Intent file IS the test specification
- Edge cases must be specified upfront
- No ambiguity about expected behavior
- Can focus on exploratory testing beyond the intent

### Team Dynamics Transformation

#### Fewer "Lost in Translation" Moments

Traditional flow:

```
Stakeholder → PM → Jira Ticket → Engineer → Code → QA → Bug?
     ↓          ↓         ↓           ↓
  (interpretation at each step, meaning drifts)
```

IDD flow:

```
Stakeholder + PM + Engineer → Intent File → Code → Verification
                                  ↓
                         (single source of truth)
```

#### Conversations Happen Earlier

Without IDD, hard conversations happen during code review or QA:

- "That's not what I meant"
- "We didn't think about that case"
- "This requirement is impossible"

With IDD, these conversations happen during intent review:

- "Can we actually test 'fast performance'? Let's define it."
- "What happens if the API is down? We need to specify."
- "This feature is too complex—can we split it?"

#### The Intent Review Meeting

New team ritual: **Intent Review** (replaces requirements review)

1. PM drafts intent file
2. Team reviews together (PM, engineer, QA, stakeholder)
3. Discussion focuses on testable assertions:
   - "Can we verify this?"
   - "What are the edge cases?"
   - "Is this priority right?"
4. Everyone agrees before coding starts
5. Intent file is committed—it's now the contract

**Result:** Alignment happens before work begins, not during firefighting.

### The Glossary as Shared Language

The Glossary section of the intent file creates **shared vocabulary**:

```intent
## Glossary

Location: A geographic area with its own weather data (e.g., "Boulder", "Denver")

Active Location: The currently selected location, shown in the header

Default Location: "Denver" - used when no location specified or invalid location given

Weather Data: Temperature, conditions, humidity, wind for a location
```

**Why this matters:**

- No more "when you say X, do you mean Y?"
- New team members learn domain language quickly
- Agents understand the business domain
- Reduces miscommunication across roles

### Accountability and Traceability

Intent file + git = clear accountability:

```bash
$ git log --oneline intent/app.intent

a1b2c3d PM: Add "offline mode" feature (stakeholder request)
d4e5f6g Eng: Clarify cache invalidation behavior
g7h8i9j PM: Remove "dark mode" (descoped for v1)
j0k1l2m Team: Initial intent for weather app
```

- Every requirement change is tracked
- Know who added/changed what requirement
- No "who decided this?" mysteries
- Audit trail for compliance if needed

### Impact Summary

| Before IDD                                          | With IDD                                     |
| --------------------------------------------------- | -------------------------------------------- |
| Requirements interpreted differently by each person | Single source of truth everyone references   |
| Edge cases discovered during testing                | Edge cases specified upfront                 |
| "Done" is subjective                                | "Done" = all intent tests pass               |
| Documentation is separate artifact that gets stale  | Intent file IS documentation, always current |
| Stakeholders trust but can't verify                 | Stakeholders can run verification themselves |
| Hard conversations happen late (expensive)          | Hard conversations happen early (cheap)      |
| Engineers guess at PM's intent                      | Engineers implement explicit specifications  |
| QA writes tests from their interpretation           | Intent file IS the test specification        |

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

### Intent Identifiers

Each intent item has a **stable identifier** that links to code annotations.

#### Identifier Format

```
<type>.<id>

Types:
  feature.   - Features from ## Features section
  data.      - Schemas/instances from ## Data section
  constraint. - Constraints from ## Constraints section
```

#### How IDs Are Assigned

**Option A: Explicit IDs (Recommended)**

Intent file explicitly declares the ID:

```intent
### Feature: Site Selection
id: site_selection
Priority: Must Have

Users can select which site to view...
```

The `id:` field is the stable identifier. The human-readable name can change freely.

**Option B: Derived IDs (Fallback)**

If no explicit `id:` is provided, derive from the name:

1. Take the feature name after "Feature: " (e.g., "Site Selection")
2. Lowercase it
3. Replace spaces with underscores
4. Remove special characters

"Site Selection" → `site_selection`
"30-Day Chart" → `30_day_chart`

#### What Happens When Names Change?

| Scenario                                     | With Explicit ID                                   | With Derived ID                                                   |
| -------------------------------------------- | -------------------------------------------------- | ----------------------------------------------------------------- |
| Rename "Site Selection" to "Location Picker" | No code changes needed (id stays `site_selection`) | Must update all `@implements: feature.site_selection` annotations |
| Add new feature                              | Add new `id:`                                      | Works automatically                                               |
| Typo fix in name                             | No impact                                          | Breaks all links                                                  |

**Recommendation:** Always use explicit IDs for stability. The human-readable name is for humans; the ID is for machines.

#### Validation on Name Change

When `ntnt intent check` runs and detects a derived ID changed:

```
$ ntnt intent check app.tnt

WARNING: Intent ID may have changed
  "Site Selection" → "Location Picker"
  Old ID: feature.site_selection
  New ID: feature.location_picker

  3 annotations reference old ID:
    - app.tnt:15  @implements: feature.site_selection
    - app.tnt:42  @implements: feature.site_selection
    - app.tnt:78  @implements: feature.site_selection

  Options:
    1. Add explicit id: site_selection to preserve links
    2. Update annotations to feature.location_picker
    3. Run: ntnt intent rename site_selection location_picker
```

#### The Rename Command

```bash
$ ntnt intent rename feature.site_selection feature.location_picker

Updated 3 annotations in app.tnt
Updated intent file with explicit id: location_picker
```

### Benefits of Annotations

1. **Traceability**: Know which code implements which intent
2. **Coverage**: Find unlinked code that might be dead
3. **Refactoring safety**: Know what intent an edit might affect

### Utility Code and Shared Infrastructure

Not all code maps directly to a single feature. IDD accommodates several patterns:

#### Pattern 1: Utility Functions (No Annotation Needed)

General-purpose helpers don't need `@implements` annotations:

```ntnt
// No annotation - this is a pure utility function
fn format_date(timestamp) {
    return format(timestamp, "YYYY-MM-DD")
}

// No annotation - generic CSV parsing
fn parse_csv_row(line) {
    return split(line, ",")
}

// No annotation - string helper
fn slugify(text) {
    return replace(to_lower(text), " ", "_")
}
```

**Rule of thumb:** If a function could be copy-pasted into a completely different app and still make sense, it's a utility and doesn't need an annotation.

#### Pattern 2: Multiple Features (Multiple Annotations)

When code implements multiple features, list them all:

```ntnt
// @implements: feature.snow_display
// @implements: feature.chart
// @implements: feature.data_export
fn home_handler(req) {
    let data = fetch_snow_data(req)

    // This one handler serves all three features
    return html(render_page(data))
}
```

#### Pattern 3: Shared Infrastructure (Use `infra.` Type)

For code that supports multiple features but isn't a utility, use the `infra.` type:

```ntnt
// @implements: infra.data_fetching
fn fetch_snow_data(site_key) {
    let url = sites[site_key].url
    return fetch(url).body
}

// @implements: infra.error_handling
fn handle_api_error(error) {
    log_error(error)
    return html(error_page())
}

// @implements: infra.caching
let cache = map {}
fn get_cached(key, fetch_fn) {
    if !has_key(cache, key) {
        cache[key] = fetch_fn()
    }
    return cache[key]
}
```

Define infrastructure in the intent file:

```intent
## Infrastructure

### Infra: Data Fetching
id: data_fetching

Centralized data fetching from SNOTEL API.

Used by: feature.snow_display, feature.chart, feature.comparison

---

### Infra: Error Handling
id: error_handling

Graceful handling of API failures and invalid input.

Used by: All features
```

#### Pattern 4: Indirect Support (Use `supports.` Annotation)

When code doesn't directly implement a feature but is required by it:

```ntnt
// @supports: feature.chart
fn calculate_trend(data_points) {
    // This doesn't render the chart, but the chart needs it
    let sum = 0
    for point in data_points {
        sum = sum + point.value
    }
    return sum / len(data_points)
}

// @supports: feature.snow_display
// @supports: feature.comparison
fn extract_site_name(csv_header) {
    // Parsing helper used by display features
    return trim(split(csv_header, "#")[1])
}
```

The difference:

- `@implements` = "This code IS the feature"
- `@supports` = "This code is used BY the feature"

#### Coverage Report with Code Categories

The coverage report distinguishes between code types:

```
$ ntnt intent coverage app.tnt

Intent Coverage Report
═══════════════════════════════════════════════════════

Features:
  site_selection     ✅ Implemented    get_site_from_params()
  snow_display       ✅ Implemented    home_handler()
  chart              ✅ Implemented    home_handler(), render_chart()

Infrastructure:
  data_fetching      ✅ Implemented    fetch_snow_data()
  error_handling     ✅ Implemented    handle_api_error()

Supporting Code (has @supports):
  calculate_trend()        → feature.chart
  extract_site_name()      → feature.snow_display, feature.comparison

Utility Code (no annotation, expected):
  format_date()
  parse_csv_row()
  slugify()

Unlinked Code (review these):
  mysterious_function()    ← Consider adding annotation or removing
  old_handler()            ← Possibly dead code?

═══════════════════════════════════════════════════════
Feature Coverage:   100% (3/3)
Infra Coverage:     100% (2/2)
Code Linkage:       85% (17/20 functions linked or utility)
```

#### When to Use What

| Code Type                     | Annotation               | Example                         |
| ----------------------------- | ------------------------ | ------------------------------- |
| Directly implements a feature | `@implements: feature.X` | Route handlers, main logic      |
| Implements multiple features  | Multiple `@implements`   | Shared handlers                 |
| Shared infrastructure         | `@implements: infra.X`   | Caching, logging, auth          |
| Helper used by features       | `@supports: feature.X`   | Calculations, parsing           |
| Generic utility               | None                     | Date formatting, string helpers |
| Dead code                     | None (flagged in report) | Old functions to remove         |

#### The "Unlinked Code" Question

When coverage reports show unlinked code, ask:

1. **Is it a utility?** → Leave it unannotated, it's fine
2. **Does it support a feature?** → Add `@supports`
3. **Is it infrastructure?** → Add `@implements: infra.X`
4. **Is it dead code?** → Delete it
5. **Is it a missing feature?** → Add feature to intent file

The goal isn't 100% annotation coverage—it's that every piece of code has a clear reason to exist.

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
