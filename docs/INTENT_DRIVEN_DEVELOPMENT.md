# Intent-Driven Development (IDD)

> **The missing contract layer between human requirements and AI-generated code—where plain English becomes executable tests.**

## Design Document

**Status:** Draft  
**Author:** Josh Cramer + Claude Opus 4.5  
**Created:** January 13, 2026  
**Last Updated:** January 14, 2026

---

## Executive Summary

Intent-Driven Development (IDD) is a paradigm where **human intent becomes executable specification**. Rather than writing requirements in documents that get stale, or coding directly without clear specification, IDD creates a **single source of truth** that:

1. **Humans can read** - Natural language descriptions of what the app should do
2. **Agents can execute** - Structured assertions that verify the code matches intent
3. **Both can evolve together** - When requirements change, intent updates first, then code follows

NTNT aims to be the first language where **intent is code**.

---

## The Problem

### Current State of Human-Agent Collaboration

The typical development cycle with AI looks like this:

1. Human: "Build me an app that shows snow depth over the last 24 hours"
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

### The Proposed Solution

A feedback loop where intent drives implementation and verification proves correctness:

```
INTENT (contract) --> CODE (implementation) --> VERIFICATION (proof)
     ^                                               |
     |                                               |
     +-----------------------------------------------+
                    Feedback Loop
```

---

## How IDD Differs from TDD

Test-Driven Development already exists and is widely practiced. Why do we need something new?

### TDD: What It Is

In TDD, developers:

1. Write a failing test first
2. Write code to make the test pass
3. Refactor while keeping tests green

TDD is excellent for **code quality** and **developer confidence**.

### The Gap TDD Leaves

| Aspect               | TDD                               | IDD                                      |
| -------------------- | --------------------------------- | ---------------------------------------- |
| **Written in**       | Code (Python, JavaScript, etc.)   | Natural language + structured assertions |
| **Readable by**      | Developers & agents only          | Anyone                                   |
| **Answers**          | "Does the code work?"             | "Does the code do what I wanted?"        |
| **Abstraction**      | Implementation details            | Business intent                          |
| **Owns the spec**    | Tests ARE the spec (but in code)  | Intent IS the spec (in English)          |
| **Documentation**    | Tests ≠ docs (separate artifacts) | Intent = living documentation            |
| **AI collaboration** | Not designed for agents           | Explicitly designed for human-agent work |

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

**IDD Intent (human and agent write together):**

```yaml
Feature: Location Selection
  description: "Users can select which location to view via URL parameter"

  behavior:
    - "?location=<key> selects the location"
    - "Invalid keys fall back to default (Denver)"

  test:
    - request: GET /?location=denver
      assert: [status 200, body contains "Denver"]
    - request: GET /?location=invalid
      assert: [status 200, body contains "Denver"]
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

   - TDD assumes a human writes both tests and code
   - IDD explicitly separates "what I want" (human) from "how to build it" (agent)
   - Agents can verify their work against intent without human re-review

4. **The Documentation Problem**

   - TDD tests become stale documentation (or aren't documentation at all)
   - IDD intent IS the documentation, always verified against code
   - No separate README that drifts from reality

5. **The Evolution Problem**
   - In TDD, changing requirements means rewriting tests (developer work)
   - In IDD, changing requirements means editing plain text (anyone can do it)
   - The agent handles translating new intent into code

### IDD and TDD Coexist

IDD doesn't replace TDD—it operates at a different level:

```
┌─────────────────────────────────────────────────────────┐
│  INTENT (human-readable, business requirements)         │  ← IDD
├─────────────────────────────────────────────────────────┤
│  INTEGRATION TESTS (API contracts, end-to-end)          │  ← IDD generates
├─────────────────────────────────────────────────────────┤
│  UNIT TESTS (implementation details, edge cases)        │  ← TDD lives here
├─────────────────────────────────────────────────────────┤
│  CODE (the actual implementation)                       │
└─────────────────────────────────────────────────────────┘
```

**IDD** handles the top layer: "Does this app do what the human wanted?"
**TDD** handles the bottom layer: "Does this function handle edge cases correctly?"

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

Intent files use a hybrid format balancing human readability with machine parseability:

```yaml
# snowgauge.intent
# Intent specification for SnowGauge - Rocky Mountain snow monitoring

Meta:
  name: SnowGauge
  version: 1.0.0
  description: "Real-time snow depth monitoring for Rocky Mountain sites"

## Purpose

This application helps backcountry skiers and snowboarders check current
snow conditions at key Rocky Mountain locations. Users select a site and
see current snow depth with historical trends.

## Glossary

- site: A physical location with a snow monitoring station
- snow depth: Measured depth of snow in inches at a site
- chart: Visual representation of snow depth over time

## Data

### Sites
id: data.sites
type: map
description: "Monitoring locations with their data feed URLs"
schema:
  key: string (site_id)
  value:
    name: string
    url: string (SNOTEL data feed URL)
required_entries:
  - bear_lake
  - wild_basin
  - copeland_lake

## Features

### Site Selection
id: feature.site_selection
description: "Users can select from available monitoring sites"
priority: must-have
test:
  - request: GET /
    assert:
      - status: 200
      - body contains "Bear Lake"
      - body contains "Wild Basin"
      - body contains "Copeland Lake"

### Snow Display
id: feature.snow_display
description: "Users see current snow depth for selected site"
priority: must-have
test:
  - request: GET /?site=bear_lake
    assert:
      - status: 200
      - body contains "Snow Depth"
      - body matches r"\d+(\.\d+)?\s*inches"

### Snow Chart
id: feature.snow_chart
description: "30-day historical snow depth chart"
priority: should-have
test:
  - request: GET /?site=bear_lake
    assert:
      - body contains "<canvas"
      - body matches r"chart.*30.*day|30.*day.*chart"
  - browser:
      - navigate: /?site=bear_lake
      - assert: element "canvas" is visible
      - assert: element ".chart-legend" contains "30 days"

## Constraints

### Response Time
id: constraint.response_time
description: "Pages load within 2 seconds"
test:
  - request: GET /
    assert:
      - response_time < 2000ms

### Data Freshness
id: constraint.data_freshness
description: "Snow data is no more than 1 hour old"
test:
  - request: GET /?site=bear_lake
    assert:
      - body matches r"Updated:.*ago"
      - body not matches r"Updated: [2-9]\d* hours ago"

### Mobile Responsive
id: constraint.mobile_responsive
description: "Usable on mobile devices"
test:
  - browser:
      - viewport: 375x667
      - navigate: /
      - assert: no horizontal scroll
      - assert: element ".site-selector" is visible
      - assert: tap target ".site-button" >= 44px

## UI/UX Constraints

### Visual Hierarchy
id: ux.visual_hierarchy
description: "Content is clearly organized with obvious primary action"
test:
  - browser:
      - navigate: /
      - dom:
          - content_area_ratio: ">70%"
          - largest_element_is: ".snow-depth-display" or "canvas"
          - above_fold_contains: ["site selector", "current depth"]

### Accessibility
id: ux.accessibility
description: "WCAG 2.1 AA compliant"
test:
  - browser:
      - navigate: /
      - accessibility:
          - color_contrast: ">=4.5:1"
          - all images have alt text
          - all form inputs have labels
          - keyboard navigable

## Security

### Input Validation
id: security.input_validation
description: "User input is sanitized"
test:
  - request: GET /?site=<script>alert('xss')</script>
    assert:
      - body not contains "<script>alert"
      - status: 400 or body contains "Invalid site"

### No Sensitive Data Exposure
id: security.no_exposure
description: "API keys and credentials never appear in responses"
test:
  - request: GET /
    assert:
      - body not matches r"(api[_-]?key|password|secret|token)\s*[:=]\s*['\"][^'\"]+['\"]"

## Non-Requirements

Things explicitly out of scope for v1.0:

- User accounts or authentication
- Historical data beyond 30 days
- Weather forecasts
- Mobile native app
```

---

## CLI Commands

### `ntnt intent check`

Verify implementation matches intent:

```bash
$ ntnt intent check snowgauge.tnt

SnowGauge Intent Verification
=============================

Feature: Site Selection
  ✓ GET / returns status 200
  ✓ body contains "Bear Lake"
  ✓ body contains "Wild Basin"
  ✓ body contains "Copeland Lake"

Feature: Snow Display
  ✓ GET /?site=bear_lake returns status 200
  ✓ body contains "Snow Depth"
  ✓ body matches snow depth pattern

Feature: Snow Chart
  ✓ body contains canvas element
  ✗ chart shows 30 days (found: 7 days)

Constraint: Response Time
  ✓ GET / completed in 342ms (< 2000ms)

Security: Input Validation
  ✓ XSS attempt properly rejected

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Result: 11/12 assertions passed (91%)
        1 feature partially failing
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### `ntnt intent init`

Generate code scaffolding from intent:

```bash
$ ntnt intent init snowgauge.intent

Generated: snowgauge.tnt

  Created:
  - sites data map (3 entries)
  - home_handler() stub      @implements: feature.site_selection
  - site_handler() stub      @implements: feature.snow_display
  - render_chart() stub      @implements: feature.snow_chart

  Next steps:
  1. Implement the TODO sections
  2. Run `ntnt intent check snowgauge.tnt`
```

### `ntnt intent watch`

Continuous verification during development:

```bash
$ ntnt intent watch snowgauge.tnt

Watching for changes... (Ctrl+C to stop)

[14:23:01] ✓ All 12 assertions passing
[14:23:45] ✗ feature.snow_chart failing (canvas not found)
[14:24:12] ✓ All 12 assertions passing
```

### `ntnt intent coverage`

Show which features have implementations:

```bash
$ ntnt intent coverage snowgauge.tnt

Intent Coverage Report
======================

Features:
  ✓ site_selection    → home_handler() [lines 45-67]
  ✓ snow_display      → site_handler() [lines 69-98]
  ✗ snow_chart        → (no @implements found)

Constraints:
  ✓ response_time     → (tested via HTTP)
  ✓ data_freshness    → fetch_data() [lines 23-41]

Data:
  ✓ sites             → let sites = map {...} [line 12]

Coverage: 5/6 items (83%)
```

### `ntnt intent diff`

Show gaps between intent and implementation:

```bash
$ ntnt intent diff snowgauge.tnt

Intent vs Implementation Gap Analysis
=====================================

Missing in Code:
  - feature.snow_chart (no @implements annotation found)

Extra in Code (undocumented):
  - fn debug_handler() at line 102
  - fn legacy_text_view() at line 145

Data Mismatches:
  - data.sites missing: copeland_lake
  - data.sites extra: (none)

Suggestions:
  1. Add @implements: feature.snow_chart to render_chart()
  2. Add copeland_lake to sites map
  3. Either add debug_handler to intent or mark @internal
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

### Engine Execution Flow

**For HTTP Servers:**

```
1. Start server on random port
2. GET http://localhost:54321/
3. Check response.status == 200 ✓
4. Check "Bear Lake" in body ✓
5. Shutdown server
6. Report: PASS
```

**For Libraries/Functions:**

```
1. Import module
2. Call parse_csv("name,age\nAlice,30")
3. Check result[0]["name"] == "Alice" ✓
4. Check result[0]["age"] == "30" ✓
5. Check len(result) == 1 ✓
6. Report: PASS (3/3 assertions)
```

**For CLI Tools:**

```
1. Create temp test directory with fixtures
2. Run: ntnt run program.tnt search "*.txt" ./testdir
3. Capture stdout, stderr, exit code
4. Check exit_code == 0 ✓
5. Check "found 3 files" in stdout ✓
6. Report: PASS
```

**For Database Operations:**

```
1. Begin transaction (or use test database)
2. Call register_user("alice@test.com", "Alice")
3. Check result.id > 0 ✓
4. Query: SELECT * FROM users WHERE email = '...'
5. Check row count == 1 ✓
6. Rollback transaction
7. Report: PASS
```

### Common Assertion Types

These assertions work across all test types:

**Value assertions:**

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

**Type assertions:**

```yaml
assert:
  - result is_string
  - result is_int
  - result is_list
  - result is_map
  - result is_json # Valid JSON string
  - result is_none # None/null value
```

**Error assertions:**

```yaml
assert:
  - throws: ErrorType # Specific error
  - throws # Any error
  - not throws # No error (success)
  - error_message contains "invalid"
```

**Timing assertions:**

```yaml
assert:
  - duration < 100ms # Performance check
  - response_time < 2000ms # HTTP response time
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

### Test Isolation

Each test runs in isolation:

- Fresh program instance per test
- No shared state between tests
- Database tests use transactions (rolled back)
- File tests use temp directories (cleaned up)
- Environment variables reset between tests

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

## Test Execution by Program Type

IDD supports verification for all program types, not just HTTP servers:

### HTTP Servers

```yaml
test:
  - request: GET /api/users
    headers:
      Authorization: "Bearer {test_token}"
    assert:
      - status: 200
      - body.json.users is array
      - body.json.users[0].name exists
```

### CLI Applications

```yaml
test "successful conversion":
  run: convert input.csv --format json
  assert:
    - exit_code: 0
    - stdout is valid json
    - stdout.rows is array

test "missing file error":
  run: convert nonexistent.csv
  assert:
    - exit_code: 1
    - stderr contains "File not found"
```

### Library Modules

```yaml
test "string splitting":
  eval: |
    import { split } from "std/string"
    split("a,b,c", ",")
  assert:
    - result == ["a", "b", "c"]

test "handles empty string":
  eval: split("", ",")
  assert:
    - result == [""]
```

### Database Operations

```yaml
setup:
  - execute: "DELETE FROM users WHERE email LIKE '%@test.example%'"

test "user creation":
  eval: |
    let user = create_user("test@test.example", "Test User")
    user
  assert:
    - result.id > 0
    - result.email == "test@test.example"

  verify_db:
    - query: "SELECT * FROM users WHERE email = 'test@test.example'"
    - assert: row_count == 1

teardown:
  - execute: "DELETE FROM users WHERE email LIKE '%@test.example%'"
```

### Background Workers

```yaml
test "job processing":
  setup:
    - enqueue: { type: "email", to: "test@example.com" }

  run: worker.tnt --once
  timeout: 10s

  assert:
    - exit_code: 0
    - stdout contains "Processed 1 job"

  verify:
    - queue "email" is empty
```

---

## Multi-Layer Testing Architecture

IDD uses progressive layers, each catching what the previous cannot:

```
┌─────────────────────────────────────────────────────────────────┐
│  LAYER 4: LLM Visual Verification                               │
│  "Does this look professional?" → Subjective quality            │
├─────────────────────────────────────────────────────────────────┤
│  LAYER 3: Browser Automation                                    │
│  Click, scroll, interact → Behavioral correctness               │
├─────────────────────────────────────────────────────────────────┤
│  LAYER 2: DOM Assertions                                        │
│  Elements exist, visible, positioned → Structural correctness   │
├─────────────────────────────────────────────────────────────────┤
│  LAYER 1: HTTP Response                                         │
│  Status codes, body content → Functional correctness            │
└─────────────────────────────────────────────────────────────────┘
```

### Layer 1: HTTP Response

Fast, reliable, catches functional bugs:

```yaml
test:
  - request: GET /
    assert:
      - status: 200
      - body contains "Welcome"
      - header "Content-Type" contains "text/html"
```

### Layer 2: DOM Assertions

Verify page structure without full browser:

```yaml
test:
  - request: GET /
    dom:
      - element "nav" exists
      - element "h1" text == "SnowGauge"
      - element ".site-list" has >= 3 children
      - element "#chart" has attribute "data-days"
```

### Layer 3: Browser Automation

Test real user interactions:

```yaml
test "site selection flow":
  browser:
    - navigate: /
    - click: ".site-button[data-site='bear_lake']"
    - wait_for: ".loading" to disappear
    - assert: url contains "site=bear_lake"
    - assert: element ".snow-depth" is visible
    - screenshot: "site_selected.png"
```

### Layer 4: LLM Visual Verification

For subjective qualities that resist mechanical testing:

```yaml
test "professional appearance":
  browser:
    - navigate: /
    - screenshot: full_page
    - llm:
        prompt: |
          Evaluate this dashboard screenshot:
          1. Is the visual hierarchy clear? (primary data prominent)
          2. Is the color scheme consistent and accessible?
          3. Does it look like a professional data visualization tool?
          4. Are there any obvious visual bugs or misalignments?
        expect: all yes
        model: gpt-4-vision
        confidence: 0.8
```

#### LLM Verification Reliability

LLM verification is powerful but requires careful handling:

**Determinism Strategy:**

```yaml
llm:
  prompt: "Is the navigation menu visible and properly aligned?"
  expect: "yes"
  model: gpt-4-vision
  temperature: 0 # Minimize randomness
  retries: 3 # Retry on ambiguous results
  consensus: 2/3 # 2 of 3 must agree
```

**Fallback Behavior:**

```yaml
llm:
  prompt: "Does this look professional?"
  expect: "yes"
  fallback:
    on_timeout: skip # Don't fail build if LLM unavailable
    on_ambiguous: warn # Flag for human review
    on_disagree: manual # Require human decision
```

**Cost Management:**

```yaml
# Only run expensive LLM checks on release branches
llm:
  enabled: ${CI_BRANCH == "main" || CI_BRANCH == "release/*"}
  budget: 100 calls/day
  cache: 24h # Cache results for identical screenshots
```

---

## Concrete UI/UX Constraints

Vague aesthetic requirements ("clean", "modern") are untestable. IDD requires concrete constraints:

### ❌ Bad: Aesthetic Adjectives

```yaml
# UNTESTABLE - what does "clean" mean?
description: "Clean, modern design with good UX"
```

### ✓ Good: Measurable Constraints

```yaml
Visual Constraints:
  layout:
    - content_area_ratio: ">70%" # Content vs chrome
    - max_nesting_depth: 3 # Visual hierarchy
    - whitespace_ratio: "20-40%" # Breathing room

  typography:
    - body_font_size: ">=16px" # Readable
    - line_height: "1.4-1.6" # Comfortable reading
    - max_line_length: "45-75ch" # Optimal reading
    - heading_scale: "consistent" # h1 > h2 > h3

  color:
    - contrast_ratio: ">=4.5:1" # WCAG AA
    - color_palette_size: "<=5" # Cohesive
    - primary_action_distinct: true # CTA stands out

  interaction:
    - tap_target_size: ">=44px" # Mobile friendly
    - hover_states: "all interactive" # Clear affordances
    - focus_visible: true # Keyboard users
    - loading_indicator: "for >200ms ops" # Feedback
```

### Information Hierarchy Tests

```yaml
test "clear visual hierarchy":
  browser:
    - navigate: /
    - assert:
        # Most important element is most prominent
        - largest_text_element is "h1" or ".primary-data"
        # Primary action is visually distinct
        - element ".cta-button" has highest contrast in viewport
        # Related items are grouped
        - elements ".site-option" are visually grouped
```

### Progressive Disclosure

```yaml
test "appropriate information density":
  browser:
    - navigate: /
    - above_fold:
        - contains: primary_data, primary_action
        - not_contains: settings, advanced_options
    - click: "Show Advanced"
    - assert: element ".advanced-panel" is visible
```

### The Taste Problem

Even with specific intent, "taste" is hard to define. This is perhaps the deepest challenge in human-AI collaboration: how do you communicate aesthetic judgment?

**The philosophical tension:** Taste is real—some designs are objectively better than others for a given purpose—but it resists explicit specification. The best designers often can't articulate _why_ something works; they just know.

**Strategies that help:**

**1. Iterative Refinement** - Let preferences emerge through feedback:

```yaml
## UI/UX - Learned Preferences
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

**2. Component-Level Approval** - Approve building blocks, not whole pages:

```yaml
### Approved Components

MetricDisplay:
  approved: true
  screenshot: "components/metric-display-approved.png"
  rules: "Large number (32px+), unit below, trend arrow right, no card"

DataTable:
  approved: true
  rules: "Compact rows (32px), alternating backgrounds, no cell borders"
```

**3. Constraint-Based Design** - Express taste as constraints:

```yaml
### Hard Constraints (measurable, must pass)

Typography:
  - font_weights_used: "<= 2" # Regular and bold only
  - font_families_used: "1" # Single font family

Elements:
  - drop_shadows: "0"
  - gradients: "0"
  - border_radius: "<= 4px"
  - decorative_icons: "0"
```

**The key insight:** Every aesthetic preference can be decomposed into concrete, testable constraints. The intent file is where you do that decomposition—not where you use subjective adjectives.

| Bad Intent (Vibes) | Good Intent (Verifiable)                                   |
| ------------------ | ---------------------------------------------------------- |
| "Clean design"     | "Content-to-chrome ratio >= 70%"                           |
| "Modern look"      | "No drop shadows, gradients, or decorative elements"       |
| "User-friendly"    | "Primary data visible without scrolling on 768px viewport" |
| "Professional"     | "Consistent font family, <= 2 font weights, <= 5 colors"   |

---

## Code Integration

### The `@implements` Annotation

Link code to intent with comments (no runtime impact):

```ntnt
import { listen, get, html } from "std/http_server"

// @implements: feature.site_selection
// @implements: feature.snow_display
fn home_handler(req) {
    let site = get_query_param(req, "site") ?? "bear_lake"
    let depth = fetch_snow_depth(site)

    return html(render_page(site, depth))
}

// @implements: feature.snow_chart
fn render_chart(site, days) {
    // Chart rendering logic
}

// @supports: constraint.data_freshness
fn fetch_snow_depth(site) {
    // Fetches with caching, ensures freshness
}
```

### Handling Undocumented Code

Not all code maps to features. IDD handles this gracefully:

```ntnt
// @utility - Helper function, no direct intent mapping needed
fn format_depth(inches) {
    return str(round(inches, 1)) + " inches"
}

// @internal - Development/debugging only
fn debug_dump_state() {
    print("Current state: ...")
}

// @infrastructure - Required plumbing
fn setup_routes(app) {
    get(app, "/", home_handler)
    get(app, "/api/data", api_handler)
}
```

**Strictness Levels:**

```bash
# Default: warn about undocumented public functions
$ ntnt intent check snowgauge.tnt
Warning: fn legacy_handler() has no intent mapping

# Strict: fail on undocumented code
$ ntnt intent check snowgauge.tnt --strict
Error: fn legacy_handler() has no intent mapping

# Relaxed: only check annotated code
$ ntnt intent check snowgauge.tnt --relaxed
# No warnings for unannotated code
```

---

## Intent History and Changelog Generation

Intent changes are inherently meaningful—they describe what changed about the product, not the implementation.

### Viewing Intent History

```bash
$ ntnt intent history feature.snow_chart

Intent History: feature.snow_chart

2026-01-14 (current)
  description: "30-day snow depth chart with daily granularity"
  changed_by: Josh

2026-01-10
  description: "7-day snow depth chart"
  changed_by: Agent (via PR #42)

2026-01-05 (created)
  description: "Snow depth visualization"
  changed_by: Josh
```

### Automatic Changelog Generation

Generate release notes from intent diffs:

```bash
$ ntnt intent changelog v1.0.0 v2.0.0

## [2.0.0] - 2026-01-14

### Added
- **Export Data**: Users can export snow data as CSV or JSON
- **Multi-Site Comparison**: Compare snow depth across multiple sites
- New monitoring site: Copeland Lake

### Changed
- **Snow Chart**: Extended from 7 days to 30 days of historical data
- **Data Refresh**: Now updates every 15 minutes (was hourly)

### Removed
- Legacy text-only view (replaced by responsive chart)
```

### Intent Archaeology

Understand the full history of a concept:

```bash
$ ntnt intent archaeology "chart"

Timeline for "chart":
─────────────────────────────────────────────────────
2026-01-05  feature.snow_visualization created
2026-01-07  renamed → feature.snow_chart
2026-01-10  changed: "5-day" → "7-day"
2026-01-14  changed: "7-day" → "30-day"
─────────────────────────────────────────────────────
```

### CI/CD Integration

```yaml
# GitHub Actions: Auto-generate changelog on release
name: Release
on:
  push:
    tags: ["v*"]

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Generate Changelog
        run: |
          PREV_TAG=$(git describe --tags --abbrev=0 HEAD^)
          ntnt intent changelog $PREV_TAG ${{ github.ref_name }} > RELEASE_NOTES.md

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v1
        with:
          body_path: RELEASE_NOTES.md
```

---

## The Workflow

### Phase 1: Intent First

Human writes intent before any code exists:

```bash
$ ntnt intent init myproject.intent
# Edit the generated intent file
# Define features, constraints, data schemas
```

### Phase 2: Generate Scaffold

Agent creates implementation skeleton:

```bash
$ ntnt intent init myproject.intent
# Generates myproject.tnt with stubs and @implements annotations
```

### Phase 3: Implement

Agent (or human) fills in the implementation:

```bash
$ ntnt intent watch myproject.tnt
# Continuous feedback as you implement
```

### Phase 4: Verify

Confirm implementation matches intent:

```bash
$ ntnt intent check myproject.tnt
# All assertions pass = intent satisfied
```

### Phase 5: Evolve

Update intent, re-verify:

```bash
# Human updates myproject.intent
$ ntnt intent check myproject.tnt
# See what's now failing, implement changes
```

---

## Intent Studio

> ✅ **Status: Implemented** - Use `ntnt intent studio <file>.intent` to try it!

### The Problem with Raw Intent Files

The `.intent` format is optimized for machine parsing and testing—but humans deserve a better experience when creating and refining intent. Even though intent files are written in natural language, reading YAML-like structure with indentation, IDs, and assertion syntax can feel tedious.

**Intent files should be:**

- Great for agents (parseable, testable) ✅
- Great for humans (enjoyable to develop) ✅ ← Fixed with Intent Studio!

### The Solution: Intent Studio

```bash
$ ntnt intent studio server.intent --port 3000

🎨 Intent Studio: http://localhost:3000
👀 Watching server.intent for changes...
```

Intent Studio is a collaborative workspace where humans and agents develop intent together. It renders your `.intent` file as a beautiful, interactive HTML page. When you save changes, the browser **automatically refreshes** every 2 seconds—making it feel like you're designing intent in real-time.

**Features:**
- 🎨 Beautiful dark theme with feature cards
- 📊 Stats dashboard showing features, test cases, and assertions
- 🔄 Auto-refresh when you save the `.intent` file
- ✨ Smart icons based on feature names (🔐 login, 📊 charts, etc.)
- ❌ Error display with auto-retry when parsing fails
- 🌐 Auto-opens your browser (use `--no-open` to disable)

### What You See

Instead of reading this:

```yaml
Feature: User Login
  id: feature.user_login
  description: "Allows users to authenticate with email/password"
  test:
    - request: POST /login
      body: '{"email": "test@example.com", "password": "secret"}'
      assert:
        - status: 200
        - body contains "token"
```

You see a beautifully rendered card:

```
┌─────────────────────────────────────────────────────────────────┐
│  🔐 User Login                                         feature  │
│                                                                 │
│  Allows users to authenticate with email/password               │
│                                                                 │
│  Acceptance Criteria:                                           │
│  ✓ POST /login → 200 with authentication token                  │
└─────────────────────────────────────────────────────────────────┘
```

### The Collaborative Development Workflow

This transforms how humans and agents collaborate on intent:

```
┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│   VS Code        │     │   Terminal       │     │   Browser        │
│                  │     │                  │     │                  │
│  server.intent   │     │  ntnt intent     │     │  Intent Studio   │
│  ─────────────   │────▶│  studio ...      │────▶│  ────────────    │
│  [editing]       │     │                  │     │  Beautiful UI    │
│                  │     │  Watching...     │     │  Auto-updates!   │
└──────────────────┘     └──────────────────┘     └──────────────────┘
        │                                                  │
        │              Save file (⌘S)                      │
        └──────────────────────────────────────────────────┘
                         Instant refresh
```

**Step-by-step:**

1. Create or open an existing `.intent` file (`ntnt intent init` for new projects)
2. Start the studio: `ntnt intent studio server.intent`
3. Human opens studio in browser (side-by-side with editor)
4. Human and agent collaborate—discussing, adding, removing, refining features
5. Agent updates `.intent` file, studio instantly refreshes
6. Repeat until human says "looks good!"
7. Agent implements the code with `@implements` annotations

This works for both **new intent development** and **modifying existing intent**.

### Future Features

- **Feature history timeline** - View how each feature evolved over time
- **Removed feature archive** - Browse features that were removed (nothing is lost)
- **Implementation status** - Show which features have `@implements` annotations
- **Team comments** - Inline commenting for async review (like Google Docs)
- **Shareable URLs** - Send preview links to stakeholders for approval

---

## Human Experience

### What Makes It Fun

1. **Conversation, not specification** - Writing intent feels like explaining to a friend
2. **Immediate feedback** - See if your app works without running it yourself
3. **Change without fear** - Update intent, agent handles the rest
4. **Understand without code** - Intent file IS the documentation

### What Humans DON'T Have to Do

- Read code to understand what the app does
- Debug implementation details
- Write formal specifications
- Learn programming concepts

---

## Agent Experience

### What Makes It Satisfying

1. **Clear success criteria** - Know exactly when done
2. **No guessing** - Intent file tells me what to build
3. **Provable correctness** - Can demonstrate I built the right thing
4. **Efficient iteration** - Don't waste cycles on wrong approaches

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

**Product Managers:**

- Forces complete thinking before development starts
- Creates reviewable artifacts stakeholders can approve
- Reduces "that's not what I meant" moments
- Clear acceptance criteria that aren't subjective

**Stakeholders:**

- Readable intent file serves as living documentation
- Verification report proves requirements are met
- Don't need to understand code to trust the software
- Can review and edit requirements directly (it's just text)

**Engineers:**

- Clear, testable requirements before coding starts
- Know exactly when a feature is complete (tests pass)
- Push back on vague requirements: "What should the test be?"
- Refactor fearlessly—intent tests catch regressions

**QA:**

- Intent file IS the test specification
- Edge cases must be specified upfront
- No ambiguity about expected behavior
- Can focus on exploratory testing beyond the intent

### Team Dynamics Transformation

**Traditional flow:**

```
Stakeholder → PM → Jira Ticket → Engineer → Code → QA → Bug?
     ↓          ↓         ↓           ↓
  (interpretation at each step, meaning drifts)
```

**IDD flow:**

```
Stakeholder + PM + Engineer → Intent File → Code → Verification
                                  ↓
                         (single source of truth)
```

### The Intent Review Meeting

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

The Glossary section creates **shared vocabulary**:

```yaml
## Glossary

Location: A geographic area with its own weather data (e.g., "Boulder", "Denver")
Active Location: The currently selected location, shown in the header
Default Location: "Denver" - used when no location specified or invalid
```

No more "when you say X, do you mean Y?" New team members learn domain language quickly. Agents understand the business domain.

---

## Architecture: Zero Runtime Impact

IDD is implemented as a completely separate module from the NTNT interpreter:

```
┌─────────────────────────────────────────────────────────────────┐
│                         NTNT                                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────────────┐       ┌─────────────────────────────┐  │
│  │   Core Runtime      │       │   Intent Module             │  │
│  │                     │       │   (src/intent/)             │  │
│  │   - Lexer           │       │                             │  │
│  │   - Parser          │  NO   │   - Intent parser           │  │
│  │   - AST             │ ───── │   - HTTP test runner        │  │
│  │   - Interpreter     │ DEPS  │   - Browser automation      │  │
│  │   - Type checker    │       │   - Coverage analyzer       │  │
│  │   - Stdlib          │       │   - Changelog generator     │  │
│  │                     │       │                             │  │
│  └─────────────────────┘       └─────────────────────────────┘  │
│            │                               │                     │
│            ▼                               ▼                     │
│     ntnt run app.tnt              ntnt intent check app.tnt     │
│     (no IDD overhead)             (IDD features available)      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key principles:**

1. **No runtime dependency** - `ntnt run` never loads IDD code
2. **Comment-based annotations** - `@implements` is just a comment, not syntax
3. **Separate entry points** - `ntnt intent *` commands are isolated
4. **Optional feature** - Projects work fine without any `.intent` files

This means:

- Zero performance impact on production code
- No risk to runtime stability
- Users opt-in to IDD features

---

## Limitations and Non-Goals

IDD is powerful but not magic. Be clear about what it doesn't solve:

### What IDD Does NOT Test

1. **Emergent behavior** - Interactions between features that weren't explicitly specified
2. **Performance under load** - Single-request tests don't catch scalability issues
3. **Security vulnerabilities** - IDD tests known attack patterns, not novel exploits
4. **Code quality** - Clean architecture, maintainability, technical debt
5. **Edge cases not in intent** - If you didn't specify it, it won't be tested

### Ambiguity Is Still Possible

```yaml
# This is still ambiguous:
description: "Fast response times"

# This is testable:
constraint: response_time < 500ms
```

IDD encourages precision but can't enforce it. Vague intent = vague tests.

### Intent Can Conflict

```yaml
Feature: Show All Data
  description: "Display all available metrics"

Constraint: Fast Load Time
  description: "Page loads in under 1 second"

# These may conflict - IDD won't resolve it for you
```

### Not a Replacement For:

- **Unit tests** - Still valuable for implementation correctness
- **Integration tests** - Still needed for system interactions
- **Manual testing** - Human judgment for subjective quality
- **Code review** - Implementation quality matters
- **Security audits** - Professional security review

### IDD Is Best For:

- Human-AI collaboration contracts
- Feature-level acceptance criteria
- Regression detection on intent changes
- Automatic documentation and changelog
- Onboarding and knowledge transfer

---

## Implementation Roadmap

### Phase 1: Core POC (2-3 weeks)

| Milestone | Description                   | Days    |
| --------- | ----------------------------- | ------- |
| M1        | Intent file parser            | 2-3     |
| M2        | HTTP test runner              | 3-4     |
| **M3**    | **POC Validation (go/no-go)** | **1-2** |
| M4        | Expanded assertions           | 2-3     |
| M5        | Output polish                 | 1-2     |
| M6        | Code annotations              | 2-3     |

**Critical checkpoint: M3**

After M3 (~1 week), answer:

- Does writing intent-first feel natural?
- Does the feedback loop help?
- Is this worth building out?

If yes → Continue. If no → Pivot before investing more.

### Phase 2: Full Features (4-6 weeks)

| Milestone | Description             | Days |
| --------- | ----------------------- | ---- |
| M7        | `ntnt intent init`      | 2-3  |
| M8        | `ntnt intent watch`     | 1-2  |
| M9        | Data validation         | 2-3  |
| M10       | `ntnt intent diff`      | 1-2  |
| M11       | Browser automation      | 5-7  |
| M12       | LLM visual verification | 3-5  |

### Phase 3: Polish (2-4 weeks)

- Intent history and changelog
- IDE integration (LSP)
- Performance optimization
- Documentation

**Total for full vision: 3-4 months**

But the POC in 1-2 weeks tells you if it's worth it.

---

## Future Possibilities

### Intent Composability

```yaml
# auth_patterns.intent - reusable patterns
Pattern: authenticated_endpoint
  test:
    - request: {endpoint} (no auth)
      assert: status 401
    - request: {endpoint} (invalid token)
      assert: status 403
    - request: {endpoint} (valid token)
      assert: status 200

# myapp.intent - uses the pattern
Feature: User Profile
  uses: authenticated_endpoint
  endpoint: GET /api/profile
```

### Cross-Project Intent Patterns

Library of intent patterns for common scenarios:

- CRUD API
- Authentication flow
- File upload
- Search/filter
- Pagination

### Intent Drift Detection

Detect when code evolves beyond the documented intent:

```bash
$ ntnt intent drift-check

Warning: Potential intent drift

  feature.snow_chart
    Intent says: "30-day chart"
    Code supports: 30, 60, and 90 day options
    Suggestion: Update intent to reflect capabilities
```

### Universal IDD (Future)

Long-term vision: IDD as a standalone tool for any language.

```bash
# Same intent file works across languages
idd check --lang python src/
idd check --lang typescript src/
idd check --lang go ./...
```

The `.intent` file format is the constant—implementation language can vary.

_See [FUTURE_VISION.md](FUTURE_VISION.md) for the full multi-language roadmap._

---

## Summary

Intent-Driven Development transforms human-AI collaboration:

**Before IDD:**

```
"Build X" → builds something → "No, I meant Y" → rebuilds → repeat
```

**After IDD:**

```
Intent file → verified implementation → confident deployment
```

The `.intent` file becomes:

- **For humans**: Plain English requirements anyone can read and edit
- **For agents**: Testable assertions to verify against
- **For teams**: Single source of truth that evolves with the project
- **For history**: Automatic changelog of what you meant to build

This is what makes NTNT truly "AI-native"—not just a language agents can write, but a development paradigm designed for human-agent collaboration.

---

_"The best documentation is documentation that tests itself."_
