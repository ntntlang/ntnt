# NTNT Programming Language

NTNT (/ɪnˈtɛnt/) is an open-source agent-native language with Intent-Driven Development built in. You define constraints and expected behavior. Agents implement. NTNT verifies continuously.

> **Experimental**: NTNT is a research language exploring AI-assisted development. It is not ready for production use.

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

<details>
<summary><b>Manual Installation</b></summary>

```bash
# Install Rust if needed: https://rustup.rs/
git clone https://github.com/ntntlang/ntnt.git
cd ntnt
cargo build --release
cargo install --path . --locked
```

</details>

### Hello World with Intent-Driven Development

**1. Define requirements** (`hello-world.intent`):

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

**2. Implement** (`hello-world.tnt`):

```ntnt
import { text } from "std/http/server"

// @implements: feature.hello
fn home(req) {
    return text("Hello, World!")
}

get("/", home)
listen(8080)
```

**3. Verify**:

```bash
$ ntnt intent check hello-world.tnt -vv

✓ Feature: Hello World
  ✓ Greeting
      When a user visits the home page
      → the page loads
      → they see "Hello, World"
        ✓ status: 200
        ✓ body contains "Hello, World"

1/1 features passing
```

For visual development, use **Intent Studio**:

```bash
ntnt intent studio server.intent
# Opens http://127.0.0.1:3001 with live ✓/✗ indicators
```

---

## Why NTNT?

AI agents generate code quickly. The hard part is knowing whether the result satisfies the original requirements. Specs live in docs or prompts. Tests assert implementation details. When requirements change, there's no reliable signal for what is now invalid.

NTNT explores a different approach. Requirements are executable specifications written in Intent Assertion Language (IAL). IAL defines enforceable assertions that continuously check implementation compliance. The `@implements` annotation links code to requirements, and `ntnt intent check` verifies everything matches.

### Key Features

| Feature | Description |
|---------|-------------|
| **Intent-Driven Development** | Write requirements in `.intent` files. Link code with `@implements`. Run `ntnt intent check` to verify. Full traceability from requirement to implementation. |
| **Design by Contract** | `requires` and `ensures` built into function syntax. In HTTP routes, contract violations return 400/500 automatically. |
| **Agent-Native Tooling** | `ntnt inspect` outputs JSON describing every function, route, and contract. `ntnt validate` returns machine-readable errors. |
| **Batteries Included** | HTTP servers, PostgreSQL, JSON, CSV, file I/O, crypto, concurrency - all in the standard library. No package manager needed. |
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

### Standard Library

| Category | Modules | Includes |
|----------|---------|----------|
| **Web** | `std/http/server`, `std/http` | HTTP server with routing, middleware, static files; HTTP client |
| **Data** | `std/json`, `std/csv`, `std/db/postgres` | Parse/stringify; PostgreSQL with transactions |
| **I/O** | `std/fs`, `std/path`, `std/env` | File operations, path manipulation, environment variables |
| **Text** | `std/string`, `std/url` | Split, join, trim, regex; URL encode/decode |
| **Utilities** | `std/time`, `std/math`, `std/crypto` | Timestamps, trig/log/exp, SHA256/HMAC/UUID |
| **Collections** | `std/collections` | push, pop, keys, values, get_key |
| **Concurrency** | `std/concurrent` | Go-style channels: send, recv, try_recv |

---

## Who Should Use NTNT?

**Good fit:** Prototypes, AI-assisted development experiments, internal tools, learning projects.

**Not a fit:** Production systems, performance-critical code, projects needing third-party libraries.

**Limitations:** Interpreted (not compiled), no package ecosystem, no debugger (use print + contracts).

---

## CLI Commands

```bash
ntnt run <file>              # Run a .tnt file
ntnt lint <file>             # Check for errors
ntnt intent check <file>     # Verify code matches intent
ntnt intent studio <intent>  # Visual studio with live tests
ntnt intent coverage <file>  # Show feature coverage
ntnt inspect <file>          # Project structure as JSON
ntnt docs [query]            # Search stdlib documentation
ntnt completions <shell>     # Generate shell completions
```

---

## Documentation

| Document | Description |
|----------|-------------|
| [Language Guide](LANGUAGE_GUIDE.md) | Learning guide with examples |
| [AI Agent Guide](docs/AI_AGENT_GUIDE.md) | Syntax rules for AI-assisted development |
| [Stdlib Reference](docs/STDLIB_REFERENCE.md) | All standard library functions |
| [IAL Reference](docs/IAL_REFERENCE.md) | Intent Assertion Language primitives |
| [Architecture](ARCHITECTURE.md) | System design details |

---

## Editor Support

**VS Code:** Copy the extension for syntax highlighting:

```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```

---

## License

MIT + Apache 2.0
