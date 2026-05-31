# NTNT Programming Language

NTNT (/ɪnˈtɛnt/) is an open-source agent-native language with Intent-Driven Development built in. You define constraints and expected behavior. Agents implement. NTNT verifies continuously.

> **Early Stage:** NTNT is in active development. The language is stable and running production apps, but expect breaking changes between minor versions.

## Quick Start

### The Agentic Path (Recommended)

From install to verified app in five prompts. Pick your tool, copy the prompt. Your agent handles the install, the syntax, and the demo.

**Prompt 1 — Your First App:**

```
Install NTNT (curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash),
then run `ntnt learn <your-platform>` and read the generated rules before writing any code.

Build a web app in server.tnt: a GET / that returns a greeting and a GET /api/status that returns
JSON. Add a requires contract on one handler. Make the app look good. Write a server.intent file
with scenarios for both routes. Run `ntnt intent check server.tnt` until all pass, then
`ntnt run server.tnt`. Respond with the URL of the running app.
```

Replace `<your-platform>` with `claude-code`, `codex`, `cursor`, or `copilot`. The `ntnt learn` command generates platform-native config files so your agent loads the rules every session.

**Prompt 2 — Add a Database:**

```
Add a SQLite database to the app. Create a messages table, add POST /messages to save a message
and GET /messages to list them. Update server.intent with scenarios for both endpoints. Run
`ntnt intent check server.tnt` until all pass. Add a link to the homepage to /messages and a
form that posts a new message.
```

**Prompt 3 — Add Authentication:**

```
Add session-based authentication using cookies and SQLite. Create a login page, protect the
homepage and /messages so only logged-in users can access the site, and redirect unauthenticated
users to /login. Add a logout link to the app. Add contracts on the protected handlers. Update
server.intent to test login page renders, unauthenticated access redirects and POSTS and run
`ntnt intent check server.tnt` until all pass.
```

**Prompt 4 — Refactor:**

```
Refactor the app to use middleware, file-based routing, external HTML templates in a views/
directory for all pages, partials for nav and footer and add a static CSS file. Move functions
to lib folder. Make sure it still looks decent. Verify `ntnt intent check server.tnt` still
passes.
```

Four prompts. You've got a full-stack web app with a database, auth, templates, middleware, and file-based routing. You didn't write any NTNT. Your agent did, and `ntnt intent check` proved it's correct at every step.

<details>
<summary><b>Advanced Demo Prompts</b></summary>

These prompts extend the app further. Docker is required for Prompt 5.

**Prompt 5 — PostgreSQL & Docker** *(requires Docker)*:

```
Migrate from SQLite to PostgreSQL. Create a Dockerfile and docker-compose.yml with the ntnt app
and a postgres container. Use environment variables for the connection string. Verify
`ntnt intent check server.tnt` still passes. Leave the app running when you're done.
```

**Prompt 6 — Background Jobs:**

```
Add a background job that fetches a new random cat fact from an API every 10 minutes and stores
it in the database. Display the latest cat fact on the homepage with a button that schedules an
immediate job to fetch a new one. Add a job log page that shows a record of all jobs run with
their status and timestamps. Verify `ntnt intent check server.tnt` still passes.
```

</details>

<details>
<summary><b>Hand-Crafted Code Path</b></summary>

### Installation

**macOS / Linux:**

```bash
curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex
```

### Hello World

```bash
echo 'print("Hello, World!")' > hello.tnt
ntnt run hello.tnt
```

### A Web API

```ntnt
import { json } from "std/http/server"

fn home(req) {
    return json(map { "message": "Hello!" })
}

fn get_user(req) {
    return json(map { "id": req.params["id"] })
}

get("/", home)
get("/users/{id}", get_user)

listen(3000)
```

```bash
ntnt run api.tnt
# Visit http://localhost:3000
```

> **Hot-reload:** HTTP servers automatically reload when you edit the source file.

### Intent-Driven Development

Write `.intent` files describing features, link code with `@implements`, and verify everything matches.

**1. Write an intent file** (`server.intent`):

```yaml
## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see {text} | body contains {text} |

---

Feature: Hello World
  id: feature.hello

  Scenario: Greeting
    When a user visits the home page
    → the page loads
    → they see "Hello, World"
```

**2. Annotate your implementation** (`server.tnt`):

```ntnt
import { html } from "std/http/server"

// @implements: feature.hello
fn home(req) {
    let name = req.query_params["name"] ?? "World"
    return html("<h1>Hello, #{name}!</h1>")
}

get("/", home)
listen(8080)
```

**3. Verify:**

```bash
$ ntnt intent check server.tnt

✓ Feature: Hello World
  ✓ Greeting
      When a user visits the home page
      → the page loads
      → they see "Hello, World"

Features: 1 passed | Scenarios: 1 passed | Assertions: 2 passed
```

For visual development, use **Intent Studio**:

```bash
ntnt intent studio server.intent
# Opens http://127.0.0.1:3001 with live ✓/✗ indicators
```

</details>

<details>
<summary><b>Build from Source</b></summary>

```bash
# Install Rust if needed: https://rustup.rs/
git clone https://github.com/ntntlang/ntnt.git
cd ntnt
cargo build --release
cargo install --path . --locked
ntnt --version
```

</details>

---

## Why NTNT?

NTNT started as a question: if an AI agent is going to write most of your code, what does the language need to do differently?

Traditional languages are designed for humans writing code by hand. Syntax is crafted for readability. Error messages assume the author made a typo. Documentation is a nice-to-have. None of that is quite right when your primary developer is an LLM.

NTNT was built for human-AI collaboration. Requirements aren't separate from the code — they're verifiable tests woven into the development loop. Contracts aren't just good practice — they're how agents understand what to build. And a comprehensive standard library means no package manager, no dependency resolution, no supply chain attacks.

### Key Features

| Feature | Description |
|---------|-------------|
| **Intent-Driven Development** | Write requirements in `.intent` files. Link code with `@implements`. Run `ntnt intent check` to verify. Full traceability from requirement to implementation. |
| **Design by Contract** | `requires` and `ensures` built into function syntax. Agents read them as specs. Humans read them as docs. In HTTP routes, contract violations return 400/500 automatically. |
| **Agent-Native Tooling** | `ntnt inspect` outputs JSON describing every function, route, and contract. `ntnt validate` returns machine-readable errors. An agent can understand an entire codebase in one call. |
| **Gradual Type System** | Two independent axes: `NTNT_LINT_MODE` (static) and `NTNT_TYPE_MODE` (runtime). Start untyped, add annotations where they help, enable full type checking when you want it. |
| **Built-In Auth** | OAuth/OIDC, JWT, CSRF, session management, local credentials, password reset, TOTP, staged auth challenges, and route/API protection — all in `std/auth`. Add Google/GitHub login or local email/password auth without dependency sprawl. |
| **Background Jobs** | Language-native job DSL with priority queues, cron, unique jobs, and dead letter handling. Memory, PostgreSQL, and Redis backends. |
| **Secure by Design** | No package manager means no supply chain attacks. Auto-escaping, SSRF protection, and security headers ship with the language. |
| **Batteries Included** | HTTP servers, PostgreSQL, SQLite, Redis, JSON, CSV, file I/O, crypto, concurrency — all in the standard library. |
| **Hot Reload** | HTTP servers reload automatically when you save. Edit code, refresh browser, see changes. |

### Design by Contract

```ntnt
fn withdraw(amount: Int) -> Int
    requires amount > 0
    requires amount <= self.balance
    ensures result >= 0
{
    self.balance = self.balance - amount
    return self.balance
}

// In HTTP routes:
// - Failed requires → 400 Bad Request
// - Failed ensures → 500 Internal Server Error
```

---

## Design Philosophy

- **Simple & Intuitive** — Readable by humans reviewing it, predictable for agents writing it.
- **Strong & Robust** — Catch mistakes early, fail clearly, never silently corrupt.
- **Consistent** — One pattern for everything. `len()` works on strings, arrays, and maps. `query()` works on PostgreSQL, SQLite, and Redis.
- **Secure by Default** — A comprehensive standard library reduces the need for third-party dependencies, and with fewer dependencies comes less exposure to supply chain attacks. Auto-escaping, SSRF protection, and security headers ship with the language.
- **Progressive Types** — Start untyped, add annotations where they help, enable full type checking when you want it.
- **Agent-Native Tooling** — `ntnt inspect` returns structured JSON of every function, route, and contract. `ntnt validate` returns machine-readable errors. An agent can understand an entire codebase in one call.
- **Verification Built In** — Intent files, `@implements` annotations, and `ntnt intent check` are part of the language itself, not a separate test framework bolted on after the fact.
- **Batteries Included** — HTTP, auth, databases, crypto, jobs, concurrency, CSV, markdown. All in the standard library, all shipping with the binary. No package manager by design.
- **Documentation as a Constraint** — The compiler refuses to build until every public function is documented. Documentation stays accurate because the build depends on it.
- **Less Code, More Done** — Routes, database queries, auth flows, and job definitions that take dozens of lines in other languages take a few in NTNT.

---

## Standard Library

Everything's built in. No package manager needed.

| Category | Modules | Includes |
|----------|---------|----------|
| **Web** | `std/http/server`, `std/http` | HTTP server with routing, middleware, static files; HTTP client |
| **Data** | `std/json`, `std/csv`, `std/db/postgres`, `std/db/sqlite` | Parse/stringify; PostgreSQL and SQLite with transactions |
| **Key-Value** | `std/kv` | Unified KV store (Redis, Valkey, DragonflyDB, SQLite, in-memory) |
| **Auth** | `std/auth` | OAuth/OIDC, JWT, CSRF, sessions, local credentials, password reset, TOTP, staged challenges, route/API protection |
| **Jobs** | `std/jobs` | Background job DSL with priority queues, cron, unique jobs, retry, dead letters. Memory, PostgreSQL, and Redis backends |
| **Concurrency** | `std/concurrent` | Spawn, typed channels (Tx/Rx), select, parallel, race, schedule, after |
| **Collections** | `std/collections` | push, pop, sort, reverse, keys, values, entries, map, filter, reduce, find, any, all |
| **I/O** | `std/fs`, `std/path`, `std/env` | File operations, path manipulation, environment variables |
| **Text** | `std/string`, `std/url`, `std/markdown` | Split, join, trim, replace, regex; URL encode/decode/parse; Markdown → HTML |
| **Utilities** | `std/time`, `std/math`, `std/crypto` | Timestamps, formatting; trig, log, exp; SHA256, AES-256-GCM, Argon2, UUID |
| **Logging** | `std/log` | Structured request logging with levels and JSON context |

---

## CLI Commands

| Command | Description |
|---------|-------------|
| `ntnt run file.tnt` | Run a program |
| `ntnt test server.tnt --get /health` | Test HTTP endpoints (start, request, stop) |
| `ntnt lint file.tnt` | Check for errors |
| `ntnt intent check file.tnt` | Verify code matches intent |
| `ntnt intent studio file.intent` | Live visual test feedback |
| `ntnt intent coverage file.tnt` | Feature coverage report |
| `ntnt validate file.tnt` | Validate with JSON output |
| `ntnt inspect file.tnt` | Project structure as JSON |
| `ntnt learn <platform>` | Set up AI agent config (claude-code, cursor, codex, copilot) |
| `ntnt check file.tnt` | Quick syntax check |
| `ntnt worker file.tnt` | Start background job workers for jobs defined in a file |
| `ntnt jobs status` | Queue depths, completed/failed/dead counts |
| `ntnt jobs list` | List jobs by status, queue, or type |
| `ntnt jobs inspect <id>` | Detailed view of a single job |
| `ntnt jobs retry <id>` | Re-queue a failed or dead job |
| `ntnt workers status` | Live worker status and active jobs |
| `ntnt workers scale` | Scale worker pools at runtime via control socket |
| `ntnt migrate .` | Migrate old `{expr}` interpolation to `#{expr}` |
| `ntnt docs std/string` | Look up module/function docs |
| `ntnt completions` | Generate shell completion scripts (bash, zsh, fish) |
| `ntnt repl` | Interactive REPL |

---

## Documentation

| Document | Description |
|----------|-------------|
| [AI Agent Guide](docs/AI_AGENT_GUIDE.md) | The guide your agent reads — syntax rules, patterns, gotchas |
| [Language Specification](docs/SYNTAX_REFERENCE.md) | Complete syntax, types, contracts, and features |
| [Stdlib Reference](docs/STDLIB_REFERENCE.md) | Every function across all stdlib modules |
| [Runtime Reference](docs/RUNTIME_REFERENCE.md) | CLI, environment variables, server configuration |
| [IAL Reference](docs/IAL_REFERENCE.md) | Intent Assertion Language primitives |
| [Deployment Guide](docs/DEPLOYMENT_GUIDE.md) | Docker, production config, hot-reload, environment setup |
| [Architecture](ARCHITECTURE.md) | System internals and design decisions |
| [IDD Design Document](design-docs/INTENT_DRIVEN_DEVELOPMENT.md) | Intent-Driven Development philosophy and workflow |
| [Whitepaper](whitepaper.md) | Technical motivation and language design philosophy |
| **Website** | [ntnt-lang.org](https://ntnt-lang.org) — learn, docs, benchmarks |

---

## Editor Support

**VS Code:** Copy the extension for syntax highlighting:

```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```

---

## Get Involved

NTNT is open source under the MIT license. The best way to contribute is to use it, break it, and file issues.

- [Contributing Guide](CONTRIBUTING.md) — how to get started
- [Roadmap](ROADMAP.md) — what's coming next
- [Issues](https://github.com/ntntlang/ntnt/issues) — bugs, feature requests, discussions

---

## License

MIT + Apache 2.0
