# NTNT Programming Language

NTNT (pronounced "Intent") is an experimental Agent-Native programming language designed for AI-assisted software development. It introduces Intent-Driven Development (IDD), where human requirements become executable specifications that AI agents implement and the system verifies. Design-by-contract syntax, machine-readable introspection, and `@implements` annotations create full traceability from intent to code.

## Quick Start

### Installation

**macOS / Linux:**

```bash
curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex
```

This automatically installs Rust if needed, clones the repo, and builds NTNT.

<details>
<summary><b>Manual Installation</b></summary>

**1. Install Rust via [rustup.rs](https://rustup.rs/) (if you don't have it):**

macOS/Linux:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Windows: Download and run [rustup-init.exe](https://win.rustup.rs/x86_64)

**2. Clone and build:**

```bash
git clone https://github.com/ntntlang/ntnt.git
cd ntnt
cargo build --release
cargo install --path . --locked
```

**3. Verify installation:**

```bash
ntnt --version
```

If `ntnt` isn't found, add cargo's bin directory to your PATH:

macOS/Linux:

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Windows (PowerShell):

```powershell
[Environment]::SetEnvironmentVariable("PATH", "$env:USERPROFILE\.cargo\bin;$env:PATH", "User")
```

</details>

<details>
<summary><b>For Contributors (faster builds)</b></summary>

```bash
cargo build --profile dev-release
cargo install --path . --profile dev-release --locked
```

This skips link-time optimization for ~2x faster builds.

</details>

### Hello World

**macOS / Linux:**

```bash
echo 'print("Hello, World!")' > hello.tnt
ntnt run hello.tnt
```

**Windows (PowerShell):**

```powershell
Set-Content -Path hello.tnt -Value 'print("Hello, World!")' -Encoding UTF8
ntnt run hello.tnt
```

### A Complete Web API

```ntnt
// api.tnt
import { json } from "std/http/server"

fn home(req) {
    return json(map { "message": "Hello!" })
}

fn get_user(req) {
    return json(map { "id": req.params["id"] })
}

get("/", home)
get(r"/users/{id}", get_user)

listen(3000)
```

```bash
ntnt run api.tnt
# Visit http://localhost:3000
```

---

## Why NTNT?

NTNT is a general-purpose language with modern features: contracts (`requires`/`ensures`), pattern matching, generics, enums, and a comprehensive standard library covering HTTP servers, databases, JSON, file I/O, and concurrency. No package manager needed - batteries included.

What makes NTNT different is **integrated Intent-Driven Development (IDD) tooling**. Write human-readable `.intent` files describing what your software should do, link code with `@implements` annotations, and verify everything matches with `ntnt intent check`.

### Language Features

```ntnt
fn withdraw(amount: Int) -> Int
    requires amount > 0
    requires amount <= self.balance
    ensures result >= 0
{
    self.balance = self.balance - amount
    return self.balance
}
```

Contracts are machine-readable specifications for agents and executable documentation for humans. In HTTP handlers, a failed precondition returns 400 Bad Request automatically.

### Standard Library

| Category        | Modules                                  | What's Included                                                                 |
| --------------- | ---------------------------------------- | ------------------------------------------------------------------------------- |
| **Web**         | `std/http/server`, `std/http`            | HTTP server with routing, middleware, static files; HTTP client with fetch/post |
| **Data**        | `std/json`, `std/csv`, `std/db/postgres` | Parse and stringify; PostgreSQL with transactions                               |
| **I/O**         | `std/fs`, `std/path`, `std/env`          | File operations, path manipulation, environment variables                       |
| **Text**        | `std/string`, `std/url`                  | Split, join, trim, replace; URL encode/decode/parse                             |
| **Utilities**   | `std/time`, `std/math`, `std/crypto`     | Timestamps, formatting, sleep; trig, log, exp; SHA256, HMAC, UUID               |
| **Collections** | `std/collections`                        | Array and map operations: push, pop, keys, values, get_key                      |
| **Concurrency** | `std/concurrent`                         | Go-style channels: send, recv, try_recv                                         |

---

## Intent-Driven Development

Write `.intent` files describing features and scenarios in plain English. The system verifies your code fulfills those intentions.

```yaml
# server.intent

Feature: User Greeting
  id: feature.greeting
  description: "Display a personalized greeting"

  Scenario: Greet by name
    When: GET /?name=Alice
    Then:
      - status 200
      - body contains "Hello, Alice"

  Scenario: Default greeting
    When: GET /
    Then:
      - status 200
      - body contains "Hello, World"
```

Link your implementation with `@implements`:

```ntnt
// server.tnt
import { html } from "std/http/server"

// @implements: feature.greeting
fn home(req) {
    let name = req.query_params["name"] ?? "World"
    return html("<h1>Hello, {name}!</h1>")
}

get(r"/", home)
listen(8080)
```

Verify with `ntnt intent check`:

```bash
$ ntnt intent check server.tnt

Feature: User Greeting
  Scenario: Greet by name
    ✓ GET /?name=Alice returns status 200
    ✓ body contains "Hello, Alice"
  Scenario: Default greeting
    ✓ GET / returns status 200
    ✓ body contains "Hello, World"

1/1 features passing (4/4 assertions)
```

For visual development, use **Intent Studio** to see live test results as you code:

```bash
ntnt intent studio server.intent
# Opens http://127.0.0.1:3001 with live ✓/✗ indicators
```

| Command                       | Description                                   |
| ----------------------------- | --------------------------------------------- |
| `ntnt intent check <file>`    | Verify code matches intent, run tests         |
| `ntnt intent studio <intent>` | Launch visual studio with live test execution |
| `ntnt intent coverage <file>` | Show feature implementation coverage          |
| `ntnt intent init <intent>`   | Generate code scaffolding from intent         |

> 📖 See [docs/INTENT_DRIVEN_DEVELOPMENT.md](docs/INTENT_DRIVEN_DEVELOPMENT.md) for the complete design document.

---

## Who Should Use NTNT?

> ⚠️ **NTNT is experimental and not production-ready.** Use it for learning, prototyping, and exploring Intent-Driven Development. Do not use it for systems that require stability, security audits, or long-term maintenance.

**Good fit:**

- Prototypes and proof-of-concepts where learning matters more than longevity
- Experiments with AI-assisted development and Intent-Driven Development
- Internal tools and scripts where you control the environment
- Learning projects where contracts make expected behavior explicit
- Exploring what agent-native programming could look like

**Not a good fit:**

- Production applications of any kind
- Performance-critical systems (use Rust, Go, or C++)
- Projects requiring third-party libraries or a package ecosystem
- Teams that need mature IDE support and debugging tools

### Limitations

**Experimental**: NTNT is a research language. The API will change. There is no stability guarantee.

**Performance**: Interpreted, not compiled. Handles hundreds of requests per second, sufficient for demos and prototypes.

**Ecosystem**: No package manager. No third-party libraries. The standard library covers common tasks; everything else requires writing code or calling external services.

**Tooling**: No debugger. Debugging is done with print statements and contracts. IDE support is syntax highlighting only.

---

## Current Status

**Version 0.3.0** - Intent Assertion Language + IDD

NTNT includes:

- ✅ Full contract system (`requires`, `ensures`, struct invariants)
- ✅ Type system with generics, enums, pattern matching
- ✅ Standard library (HTTP, PostgreSQL, JSON, CSV, time, crypto, etc.)
- ✅ File-based routing with hot-reload
- ✅ **Native hot-reload** for single-file apps
- ✅ **Intent Studio** with live test execution
- ✅ IDD commands (`intent check`, `intent coverage`, `intent init`, `intent studio`)
- ✅ Agent tooling (`inspect`, `validate`, `test`)
- ✅ **Intent Assertion Language (IAL)** - term rewriting engine for natural language tests
- 🔄 Intent diff and watch (coming soon)

See [ROADMAP.md](ROADMAP.md) for the complete 11-phase implementation plan.

---

## Documentation

For detailed information, see the following documents:

| Document | Description |
| -------- | ----------- |
| [Language Spec](LANGUAGE_SPEC.md) | Complete language syntax, types, contracts, and features |
| [AI Agent Guide](docs/AI_AGENT_GUIDE.md) | Syntax reference, HTTP server patterns, database patterns, and common idioms for AI agents |
| [IDD Design](docs/INTENT_DRIVEN_DEVELOPMENT.md) | Intent-Driven Development design document and workflow |
| [IAL Specification](docs/INTENT_ASSERTION_LANGUAGE.md) | Intent Assertion Language term rewriting engine |
| [Architecture](ARCHITECTURE.md) | System design and components |
| [Whitepaper](whitepaper.md) | Technical specification and motivation |
| [Roadmap](ROADMAP.md) | 11-phase implementation plan |

---

## Editor Support

### VS Code

Install the NTNT Language extension for syntax highlighting:

```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```

Then restart VS Code. The extension provides:

- Syntax highlighting for `.tnt` files
- Code snippets for common patterns
- Bracket matching and auto-closing

---

## CLI Commands

```bash
ntnt run <file>              # Run a .tnt file
ntnt repl                    # Interactive REPL
ntnt lint <file|dir>         # Check for errors and warnings
ntnt validate <file|dir>     # Validate with JSON output
ntnt inspect <file>          # Project structure as JSON
ntnt test <file> [options]   # Test HTTP endpoints
ntnt intent check <file>     # Verify code matches intent
ntnt intent coverage <file>  # Show feature coverage
ntnt intent init <intent>    # Generate scaffolding from intent
ntnt intent studio <intent>  # Launch visual studio with live tests
ntnt --help                  # See all commands
```
