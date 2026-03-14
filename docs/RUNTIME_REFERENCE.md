# NTNT Runtime & CLI Reference

> **Auto-generated from [runtime.toml](runtime.toml)** - Do not edit directly.
>
> Last updated: v0.4.3

Runtime configuration, environment variables, and CLI commands for NTNT

## Table of Contents

- [Environment Variables](#environment-variables)
- [Hot-Reload](#hot-reload)
- [HTTP Server](#http-server)
- [File-Based Routing](#file-based-routing)
- [Project Structure](#project-structure)
- [CLI Commands](#cli-commands)

---

## Environment Variables

Environment variables that control NTNT runtime behavior

| Variable | Values | Default | Description |
|----------|--------|---------|-------------|
| `NTNT_ALLOW_PRIVATE_IPS` | `true` | unset (disabled — private IPs blocked) | Allow `fetch()` to connect to private/internal IP ranges (10.x, 172.16-31.x, 192.168.x, 127.x). Required for Docker inter-container communication (e.g., calling a sidecar at 172.19.0.1). Disabled by default to prevent SSRF attacks. |
| `NTNT_DB_POOL_SIZE` | `any positive integer` | 5 | Maximum number of connections per database pool. Each worker creates its own pools, so total connections = num_workers × num_databases × pool_size. For multi-worker production deployments with multiple databases, keep this low (2-5) to avoid exhausting PostgreSQL max_connections. |
| `NTNT_ENV` | `development`, `production`, `prod` | development (when unset) | Controls runtime mode. Production mode disables hot-reload for better performance. |
| `NTNT_LINT_MODE` | `default`, `warn`, `strict` | default | Controls lint strictness for type annotations. `default`: only check annotated code. `warn`: also warn about missing annotations (non-fatal). `strict`: missing annotations are errors. CLI flags (`--strict`, `--warn-untyped`) override this. |
| `NTNT_MAX_RECURSION` | integer | 256 | Maximum recursion depth for function calls. Prevents stack overflow from runaway recursion. |
| `NTNT_STRICT` | `1`, `true` | unset (disabled) | **Deprecated — use `NTNT_LINT_MODE=strict` instead.** Enable strict type checking. Still works but emits a deprecation warning. |
| `NTNT_TIMEOUT` | integer (seconds) | 30 | Request timeout for HTTP server in seconds. |
| `NTNT_TYPE_MODE` | `strict`, `warn`, `forgiving` | warn | Controls runtime behavior for type mismatches. `strict`: type mismatches crash (fail-closed, recommended for auth/payments). `warn`: log `[WARN]` and continue (default). `forgiving`: silent degradation (pre-v0.4 behavior). See [Type Safety Modes](#type-safety-modes). |

### Examples

```bash
# Allow `fetch()` to connect to private/internal IP ranges (10.x, 172.16-31.x, 192.168.x, 127.x)
NTNT_ALLOW_PRIVATE_IPS=true ntnt run server.tnt

# Maximum number of connections per database pool
NTNT_DB_POOL_SIZE=3 ntnt run server.tnt

# Controls runtime mode
NTNT_ENV=production ntnt run server.tnt

# Controls lint strictness for type annotations
NTNT_LINT_MODE=strict ntnt lint server.tnt

# Maximum recursion depth for function calls
NTNT_MAX_RECURSION=512 ntnt run server.tnt

# **Deprecated — use `NTNT_LINT_MODE=strict` instead.** Enable strict type checking
NTNT_STRICT=1 ntnt run server.tnt

# Request timeout for HTTP server in seconds.
NTNT_TIMEOUT=60 ntnt run server.tnt

# Controls runtime behavior for type mismatches
NTNT_TYPE_MODE=strict ntnt run server.tnt

```

---

## Type Safety Modes

ntnt provides two independent axes for type control, configured via environment variables.

### Axis 1: Runtime Type Mode (`NTNT_TYPE_MODE`)

Controls what happens when types mismatch at runtime (e.g., bad data from a database, wrong API response type).

| Mode | Behavior | Use Case |
|------|----------|----------|
| `strict` | Type mismatches crash the request (fail-closed) | Apps with auth, payments, safety-critical logic |
| `warn` | Log `[WARN]` to stderr and continue **(default)** | Production apps with log monitoring |
| `forgiving` | Silent degradation, no warnings | Content sites where uptime > correctness |

```bash
# Crash on type mismatches (safest)
NTNT_TYPE_MODE=strict ntnt run server.tnt

# Log warnings and continue (default)
NTNT_TYPE_MODE=warn ntnt run server.tnt

# Silent degradation (pre-v0.4 behavior)
NTNT_TYPE_MODE=forgiving ntnt run server.tnt
```

**Affected operations:** index (`[]`) type mismatch, `for..in` on non-collections, field access on wrong types, template expression errors. Warnings are deduplicated per evaluation context — the same mismatch won't spam 50 times in a loop.

**Security note:** Apps with authentication, authorization, or financial logic should use `strict`. Forgiving mode is fail-open — a type mismatch on a permission check can silently bypass it.

### Axis 2: Lint Mode (`NTNT_LINT_MODE`)

Controls how the linter treats missing type annotations.

| Mode | Behavior | CI Exit Code |
|------|----------|--------------|
| `default` | Only report errors in annotated code | 0 (if no type conflicts) |
| `warn` | Also warn about missing annotations | 0 (warnings are non-fatal) |
| `strict` | Missing annotations are errors | Non-zero on missing annotations |

```bash
# Default: only check annotated code
ntnt lint app.tnt

# Warn about missing annotations (non-fatal)
ntnt lint --warn-untyped app.tnt
NTNT_LINT_MODE=warn ntnt lint app.tnt

# Require all annotations (CI-safe)
ntnt lint --strict app.tnt
NTNT_LINT_MODE=strict ntnt lint app.tnt
```

### Precedence

```
CLI flag > Environment variable > ntnt.toml > built-in default
```

### Docker Configuration

```yaml
# SaaS app with auth + payments
environment:
  - NTNT_TYPE_MODE=strict
  - NTNT_LINT_MODE=strict
  - NTNT_ENV=production

# Content site / blog
environment:
  - NTNT_TYPE_MODE=warn
  - NTNT_LINT_MODE=default
  - NTNT_ENV=production
```

---

## Hot-Reload

Automatic code reloading during development

**Default:** enabled

**Disable:** Set NTNT_ENV=production

### Tracked Files

- Main server file (.tnt)
- Imported local modules (import from "./...")
- File-based route files (routes/*.tnt)
- Route imported modules

### Behavior

- **Trigger:** Changes detected on next HTTP request
- **Action:** Full reload of main file and all imports
- **Output:** `[hot-reload] <file> changed, reloading...`

---

## HTTP Server

Built-in HTTP server runtime behavior

### Request Object Properties

| Property | Description |
|----------|-------------|
| `req.body` | Raw request body string |
| `req.headers` | Request headers map |
| `req.id` | Request ID (from X-Request-ID header or auto-generated) |
| `req.ip` | Client IP address (supports X-Forwarded-For) |
| `req.method` | HTTP method (GET, POST, etc.) |
| `req.params` | Route parameters map (e.g., req.params["id"]) |
| `req.path` | URL path without query string |
| `req.query_params` | Query string parameters map |

### Defaults

- **port:** 8080 (convention, set in listen() call)
- **timeout:** 30 seconds (override with NTNT_TIMEOUT or --timeout)

---

## File-Based Routing

Convention-based routing from directory structure

### Conventions

| Convention | Description |
|------------|-------------|
| `dynamic_segment` | [param].tnt maps to {param} (e.g., [id].tnt -> /{id}) |
| `index_file` | index.tnt maps to parent path (e.g., routes/index.tnt -> /) |
| `middleware_dir` | middleware/ |
| `nested_dynamic` | Supports nested dynamics (e.g., users/[id]/posts/[postId].tnt) |
| `routes_dir` | routes/ |

### Handler Functions

Export functions named after HTTP methods

**Supported methods:** `get`, `post`, `put`, `delete`, `patch`, `head`, `options`

### Middleware

Middleware files in middleware/ directory are auto-applied

- **Naming:** Files are applied in alphabetical order (e.g., 01_auth.tnt, 02_logging.tnt)
- **Function:** Export a function named 'middleware' that receives the request

---

## Project Structure

Recommended project layout for NTNT applications

### Recommended Layout

```
my-app/
├── server.tnt          # Main server file
├── server.intent       # Intent specification (matches server.tnt)
├── routes/             # File-based routes
│   ├── index.tnt       # GET /
│   ├── about.tnt       # GET /about
│   └── api/
│       ├── users.tnt   # GET/POST /api/users
│       └── [id].tnt    # GET /api/users/{id}
├── middleware/         # Auto-applied middleware
│   └── 01_logging.tnt
├── lib/                # Shared library code
│   └── utils.tnt
├── views/              # HTML templates
│   └── layout.html
└── public/             # Static assets (serve_static)
```

### Intent Files

Intent files are linked by filename

- **Convention:** `server.tnt <-> server.intent`
- **Recommendation:** Use a single .intent file per application for full context

---

## CLI Commands

NTNT command-line interface

### Run

```
ntnt run <FILE>
```

Execute an NTNT source file

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--timeout` | seconds | 30 | Request timeout for HTTP server (also: NTNT_TIMEOUT) |

**Example:**
```bash
ntnt run server.tnt
```

### Lint

```
ntnt lint <PATH>
```

Check source file(s) for syntax errors and common mistakes

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--quiet`, `-q` | flag | - | Show only errors, not warnings or suggestions |
| `--fix` | flag | - | Output auto-fix suggestions as JSON patch |
| `--warn-untyped` | flag | - | Enable strict typechecker warnings without failing the build: warns on missing type annotations and other strict-mode issues (e.g., Float→Int precision-loss, complex interpolation). Exit code remains 0. Also: `NTNT_LINT_MODE=warn`. |
| `--strict` | flag | - | Require type annotations on all functions — missing annotations are errors (non-zero exit). Also: `NTNT_LINT_MODE=strict`. Replaces deprecated `NTNT_STRICT`. |

**Example:**
```bash
ntnt lint server.tnt
```

### Validate

```
ntnt validate <PATH>
```

Validate source and output results as JSON (for tooling)

**Example:**
```bash
ntnt validate server.tnt
```

### Inspect

```
ntnt inspect <PATH>
```

Output project structure as JSON (for AI agents)

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--pretty`, `-p` | flag | - | Pretty-print the JSON output |

**Example:**
```bash
ntnt inspect server.tnt --pretty
```

### Test

```
ntnt test <FILE>
```

Run HTTP tests against a server file

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--get` | PATH | - | Make a GET request to the specified path |
| `--post` | PATH | - | Make a POST request to the specified path |
| `--put` | PATH | - | Make a PUT request to the specified path |
| `--delete` | PATH | - | Make a DELETE request to the specified path |
| `--body` | JSON | - | Request body for POST/PUT requests |
| `--port` | number | 18080 | Port to run the test server on |
| `--verbose`, `-v` | flag | - | Show verbose output including headers |

**Example:**
```bash
ntnt test server.tnt --get / --get /api/users
```

### Docs

```
ntnt docs [QUERY]
```

Look up documentation for stdlib modules or functions

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--validate` | flag | - | Validate that all stdlib functions are documented |
| `--generate` | flag | - | Regenerate reference docs from source annotations and TOML, and sync agent instruction files (CLAUDE.md, copilot-instructions.md) from AI_AGENT_GUIDE.md |
| `--json` | flag | - | Output as JSON (for tooling) |

**Examples:**
```bash
ntnt docs std/string
ntnt docs split
ntnt docs --generate
```

### Completions

```
ntnt completions <SHELL>
```

Generate shell completions

**Example:**
```bash
ntnt completions zsh >> ~/.zshrc
```

### Intent Check

```
ntnt intent check <FILE>
```

Run intent tests against implementation

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--intent`, `-i` | PATH | - | Path to intent file (default: <name>.intent) |
| `--port` | number | 18081 | Port to run the test server on |
| `--verbose`, `-v` | flag | - | Show scenario pass/fail status |
| `-vv` | flag | - | Show all assertions and term resolution |
| `--json` | flag | - | Output results as JSON |

**Examples:**
```bash
ntnt intent check server.tnt
ntnt intent check server.tnt -v
ntnt intent check server.tnt -vv
```

### Intent Coverage

```
ntnt intent coverage <FILE>
```

Show which features have implementations

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--intent`, `-i` | PATH | - | Path to intent file (default: <name>.intent) |

**Example:**
```bash
ntnt intent coverage server.tnt
```

### Intent Init

```
ntnt intent init <INTENT_FILE>
```

Generate code scaffolding from intent file

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--output`, `-o` | PATH | - | Output file (default: prints to stdout) |

**Example:**
```bash
ntnt intent init project.intent -o server.tnt
```

### Intent Studio

```
ntnt intent studio <INTENT_FILE>
```

Visual preview with live test execution

**Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--port`, `-p` | number | 3001 | Port for the studio server |
| `--app-port`, `-a` | number | 8081 | Port where the application server is running |
| `--no-open` | flag | - | Don't automatically open the browser |

**Example:**
```bash
ntnt intent studio server.intent
```

