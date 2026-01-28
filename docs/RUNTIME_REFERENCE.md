# NTNT Runtime & CLI Reference

> **Auto-generated from [runtime.toml](runtime.toml)** - Do not edit directly.
>
> Last updated: v0.3.7

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
| `NTNT_ENV` | `development`, `production`, `prod` | development (when unset) | Controls runtime mode. Production mode disables hot-reload for better performance. |
| `NTNT_TIMEOUT` | integer (seconds) | 30 | Request timeout for HTTP server in seconds. |

### Examples

```bash
# Development (default) - hot-reload enabled
ntnt run server.tnt

# Production - hot-reload disabled
NTNT_ENV=production ntnt run server.tnt

# Custom timeout (60 seconds)
NTNT_TIMEOUT=60 ntnt run server.tnt
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
| `--generate` | flag | - | Regenerate docs/STDLIB_REFERENCE.md from stdlib.toml |
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

