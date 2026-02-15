# Plan: Web Application Essentials + Database Layer + KV Store + Auth

## Executive Summary

**Goal:** Add everything needed to build production web applications in NTNT — no external dependencies, no missing pieces.

**Scope:** 57 stdlib functions, 1 global builtin, 7 CLI commands, 1 new syntax form

**Status:** Phase 2 In Progress (Features 1-5, 8, 9 Complete + Performance Optimizations)
**Status:** Phase 2 In Progress (Features 1-5, 9 Complete + Performance Optimizations)

**Key deliverables:**
- ✅ 🔐 **Password hashing** — bcrypt with intent-revealing names (DONE)
- ✅ 🍪 **Cookie management** — secure `with_cookie`/`set_cookie` with injection prevention (DONE)
- ✅ 📝 **Structured logging** — stderr output, JSON context, log levels (DONE - basic version)
- ✅ 🌐 **CORS** — one-liner `enable_cors()` for development and production (DONE)
- ✅ 📁 **File uploads** — `parse_multipart` + `save_upload` with path traversal protection (DONE)
- ✅ 🛣️ **Declarative routes** — `server 8080 { ... }` syntax with typed params + route conflict detection (DONE)
- ✅ ⚡ **Route matching optimization** — O(1) lookup by method+segment count (DONE)
- 🗄️ **Query builder** — 15 functions covering all CRUD patterns + transactions + upsert
- 📋 **Schema sync** — declarative migrations with production workflow + seeding + drift detection
- ✅ ⚡ **KV store** — SQLite + Redis/Valkey backends complete
- ⚡ **KV store** — SQLite (dev) or Redis/Valkey (prod) with same API
- 🔑 **Auth module** — `enable_oauth()` batteries-included + granular API for custom flows

**New crates:** `bcrypt` ✅, `redis`, `jsonwebtoken`, `totp-rs`, `tracing-appender`

---

## Design Philosophy

This is NTNT's chance to get these features right from scratch. Every API should be:

- **Intent-revealing** — function names describe *what you want*, not *how it works*
- **Pipe-friendly** — functions that transform data take the thing being transformed first
- **One obvious way** — don't provide 5 ways to do the same thing
- **Sensible defaults** — the zero-config version just works
- **Agent-native** — no hidden state, no magic, no "you just need to know" gotchas
- **Scale-ready** — the simple API must generate efficient SQL, not just correct SQL

---

## Implementation Order

Features are ordered by dependencies — each feature builds on the ones before it.

| # | Feature | Scope | New Crate | Depends On | Status |
|---|---------|-------|-----------|------------|--------|
| 1 | Password hashing | Small | `bcrypt` | — | ✅ Done |
| 2 | Cookie management | Medium | None | — | ✅ Done |
| 3 | Structured logging | Medium | `tracing-appender` | — | ✅ Done (basic) |
| 4 | CORS | Medium | None | — | ✅ Done |
| 5 | File uploads | Medium | None | — | ✅ Done |
| 6 | Query builder (`std/db`) | Large | None | — | Pending |
| 7 | Schema sync + seeding + migrations | Large | None | #6 | Pending |
| 8 | KV store (`std/kv`) | Large | `redis` | — | ✅ Done |
| 9 | Declarative route blocks (`server` syntax) | Large | None | #2, #4 | ✅ Done |
| 9a | Route matching optimization | Small | None | #9 | ✅ Done |
| 10 | Auth module (`std/auth`) | Large | `jsonwebtoken` | #2, #8 | ✅ Done |
| 8 | KV store (`std/kv`) | Large | `redis` | — | Pending |
| 9 | Declarative route blocks (`server` syntax) | Large | None | #2, #4 | ✅ Done |
| 9a | Route matching optimization | Small | None | #9 | ✅ Done |
| 10 | Auth module (`std/auth`) | Large | `jsonwebtoken`, `totp-rs` | #2, #8 | Pending |
| 11 | Roadmap + docs update | Small | — | All | ✅ Done (Phase 1) |

**Parallelization:** Features 1-6 and 8 can be implemented in parallel. Feature 7 requires 6. Feature 9 requires 2 and 4. Feature 10 requires 2 and 8.

---

## Feature 1: Password Hashing — `std/crypto` ✅ COMPLETE

Names describe intent, not algorithm. If we switch from bcrypt to argon2 later, the API doesn't change.

### API

```ntnt
import { hash_password, verify_password, is_valid_hash } from "std/crypto"

let hash = hash_password("secret123")?
let valid = verify_password("secret123", hash)?
print(valid)  // true

// Check if a string is a valid bcrypt hash (useful for migrations)
print(is_valid_hash(hash))              // true
print(is_valid_hash("not-a-hash"))      // false
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `hash_password` | `(password: String, cost?: Int) -> Result<String, String>` | Hash a password. Default cost 12. |
| `verify_password` | `(password: String, hash: String) -> Result<Bool, String>` | Verify password against hash. |
| `is_valid_hash` | `(hash: String) -> Bool` | Check if string is a valid bcrypt hash format. |

### Implementation

- Add `bcrypt = "0.15"` to `Cargo.toml`
- Add 3 functions to `src/stdlib/crypto.rs` with `// @ntnt` doc blocks
- Wraps `bcrypt::hash()` and `bcrypt::verify()`, returns Result enum values
- `is_valid_hash` uses regex to check bcrypt format: `$2[aby]?\$\d{2}\$[./A-Za-z0-9]{53}$`

### Tests

- Hash+verify roundtrip, wrong password -> Ok(false), invalid hash -> Err, cost out of range -> Err
- `is_valid_hash` returns true for valid hashes, false for random strings/empty/other hash formats

### Security Hardening (Added)
- ✅ Minimum cost raised from 4 to 10 (OWASP compliance)

---

## Feature 2: Cookie Management — `std/http/server` ✅ COMPLETE

**Note:** Implemented with slightly different function names for clarity: `set_cookie`, `get_cookie`, `get_cookies`, `delete_cookie`, `with_cookie`.

**Security Hardening (Added):**
- ✅ Cookie name validation — rejects invalid characters per RFC 6265
- ✅ Cookie value encoding — URL-encodes special characters to prevent header injection
- ✅ CRLF injection prevention — strips newline characters from values

### API

```ntnt
import { json, redirect, cookie, cookies, with_cookie, without_cookie } from "std/http/server"

// Read one cookie from request
let session = cookie(req, "session")     // Option<String>

// Read all cookies from request
let all = cookies(req)                   // Map<String, String>

// Set a cookie on a response (pipe-friendly)
return json(map { "ok": true })
    |> with_cookie("session", token, map { "http_only": true, "path": "/" })

// Set multiple cookies (chain pipes)
return json(data)
    |> with_cookie("session", token, map { "http_only": true })
    |> with_cookie("theme", "dark")

// Remove a cookie
return redirect("/")
    |> without_cookie("session", map { "path": "/" })
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `cookie` | `(req, name) -> Option<String>` | Read one cookie from request. |
| `cookies` | `(req) -> Map<String, String>` | Read all cookies from request. |
| `with_cookie` | `(resp, name, value, opts?) -> Response` | Add Set-Cookie header to response. |
| `without_cookie` | `(resp, name, opts?) -> Response` | Add Set-Cookie with Max-Age=0. |

**Cookie options map:**

| Key | Type | Description |
|-----|------|-------------|
| `path` | String | Cookie path (default: `/`) |
| `domain` | String | Cookie domain |
| `max_age` | Int | Lifetime in seconds |
| `secure` | Bool | HTTPS only |
| `http_only` | Bool | No JavaScript access |
| `same_site` | String | "Strict", "Lax", or "None" |
| `partitioned` | Bool | CHIPS — isolate cookie per top-level site (for embeds/iframes) |

### Prerequisite: Multi-value headers

HTTP requires multiple `Set-Cookie` headers (one per cookie). Current response headers are `Map<String, Value>` which can't have duplicate keys.

**Fix:** Support `Value::Array` as a header value. Each element emits a separate HTTP header.

**Files changed:**
- `src/stdlib/http_server.rs` — `send_response()`: handle `Value::Array` in header iteration
- `src/stdlib/http_bridge.rs` — change `BridgeResponse.headers` from `HashMap<String, String>` to `Vec<(String, String)>`, update `from_value`, `error`, `not_found`
- `src/stdlib/http_server_async.rs` — update `bridge_to_axum_response` for Vec headers

### Why not `set_cookie` / `get_cookie` / `delete_cookie`?

Those names are verbose and imperative. `cookie` and `cookies` are nouns — you're asking for the cookie. `with_cookie` and `without_cookie` are transformations — you're creating a new response with or without a cookie. Reads like English: "json with cookie session."

---

## Feature 3: Structured Logging — `std/log` ✅ COMPLETE (Basic Version)

**Note:** Basic version implemented. File output, rotation, and advanced configuration (`configure_logging`) are deferred to Phase 2.

**Implemented:**
- `log_debug`, `log_info`, `log_warn`, `log_error` — log to stderr
- `set_log_level` — filter by minimum level
- `request_logger()` — returns middleware function for HTTP request logging
- Output format: `2026-02-02T10:30:00Z [INFO] message {"context":"data"}`

**Deferred to Phase 2:**
- `configure_logging()` — file output, JSON format, rotation
- Environment variable configuration (`NTNT_LOG_*`)
- Log rotation with compression

Production-ready logging with file output, rotation, and structured JSON format. Configurable via code or environment variables.

### Basic API (Zero Config)

```ntnt
import { log_info, log_warn, log_error, log_debug, set_log_level, request_logger } from "std/log"

set_log_level("debug")

log_info("Server starting", map { "port": 8080 })
log_debug("Config loaded", map { "env": "development" })
log_warn("Slow query", map { "ms": 450, "query": "SELECT..." })
log_error("Connection failed", map { "host": "db.example.com" })

// Automatic request logging as middleware
use_middleware(request_logger())
```

With zero configuration, logs go to **stderr** in human-readable format — perfect for development and containerized deployments where the platform captures stderr.

### Production Configuration

```ntnt
import { configure_logging, log_info } from "std/log"

// Configure file output with rotation
configure_logging(map {
    "output": "logs/app.log",      // file path, "stderr", or "stdout"
    "level": "info",               // minimum level to log
    "format": "json",              // "json" or "text"
    "rotation": map {
        "max_size": "10MB",        // rotate when file exceeds this size
        "max_files": 5,            // keep this many rotated files
        "compress": true           // gzip rotated files
    }
})

log_info("Server starting", map { "port": 8080 })
```

### Environment Variable Configuration

All logging settings can be configured via environment variables, which take precedence over `configure_logging()` calls. This follows the 12-factor app principle — change logging behavior without code changes.

| Variable | Values | Default | Description |
|----------|--------|---------|-------------|
| `NTNT_LOG_OUTPUT` | file path, `stderr`, `stdout` | `stderr` | Where to write logs |
| `NTNT_LOG_LEVEL` | `debug`, `info`, `warn`, `error` | `info` | Minimum log level |
| `NTNT_LOG_FORMAT` | `json`, `text` | `text` | Output format |
| `NTNT_LOG_MAX_SIZE` | e.g., `10MB`, `100KB`, `1GB` | `10MB` | Max file size before rotation |
| `NTNT_LOG_MAX_FILES` | integer | `5` | Number of rotated files to keep |
| `NTNT_LOG_COMPRESS` | `true`, `false` | `true` | Compress rotated files |

```bash
# Production deployment
NTNT_LOG_OUTPUT=logs/app.log \
NTNT_LOG_LEVEL=info \
NTNT_LOG_FORMAT=json \
NTNT_LOG_MAX_SIZE=50MB \
NTNT_LOG_MAX_FILES=10 \
ntnt run server.tnt
```

### Output Formats

**Text format** (default) — human-readable, good for development:

```
2026-02-02T10:30:00Z [INFO] Server starting {"port":8080}
2026-02-02T10:30:01Z [INFO] GET /users 200 45ms
2026-02-02T10:30:01Z [WARN] Slow query {"ms":450,"query":"SELECT..."}
2026-02-02T10:30:02Z [ERROR] Connection failed {"host":"db.example.com","error":"timeout"}
```

**JSON format** — structured, for log aggregators (ELK, Datadog, CloudWatch):

```json
{"ts":"2026-02-02T10:30:00Z","level":"info","msg":"Server starting","port":8080}
{"ts":"2026-02-02T10:30:01Z","level":"info","msg":"GET /users","status":200,"duration_ms":45}
{"ts":"2026-02-02T10:30:01Z","level":"warn","msg":"Slow query","ms":450,"query":"SELECT..."}
{"ts":"2026-02-02T10:30:02Z","level":"error","msg":"Connection failed","host":"db.example.com","error":"timeout"}
```

### Log Rotation

When `output` is a file path, rotation happens automatically:

1. When file exceeds `max_size`, it's renamed to `app.log.1`
2. Existing rotated files are shifted: `app.log.1` → `app.log.2`, etc.
3. Files beyond `max_files` are deleted
4. If `compress: true`, rotated files are gzipped: `app.log.1.gz`

```
logs/
├── app.log           # current log file (actively written)
├── app.log.1.gz      # most recent rotation
├── app.log.2.gz      # older
├── app.log.3.gz      # older still
└── app.log.4.gz      # oldest (5th file, older ones deleted)
```

### Request Logger Middleware

`request_logger()` returns middleware that logs HTTP requests with timing:

```ntnt
use_middleware(request_logger())

// Text format:
// 2026-02-02T10:30:01Z [INFO] GET /users 200 45ms

// JSON format:
// {"ts":"...","level":"info","msg":"HTTP","method":"GET","path":"/users","status":200,"duration_ms":45,"ip":"192.168.1.1"}
```

The request logger automatically includes:
- HTTP method and path
- Response status code
- Request duration in milliseconds
- Client IP (from `X-Forwarded-For` or direct connection)
- Request ID (from `X-Request-ID` header or auto-generated)

### Function Summary

| Function | Signature | Description |
|----------|-----------|-------------|
| `log_info` | `(message, data?) -> Unit` | Log at INFO level. |
| `log_warn` | `(message, data?) -> Unit` | Log at WARN level. |
| `log_error` | `(message, data?) -> Unit` | Log at ERROR level. |
| `log_debug` | `(message, data?) -> Unit` | Log at DEBUG level. |
| `set_log_level` | `(level: String) -> Unit` | Set minimum log level. |
| `configure_logging` | `(config: Map) -> Unit` | Configure output, format, rotation. |
| `request_logger` | `() -> Function` | Returns HTTP request logging middleware. |

### Implementation

- New file `src/stdlib/log.rs`
- Add `tracing-appender = "0.2"` to `Cargo.toml` for file output and rotation
- Global config via `static CONFIG: RwLock<LogConfig>`
- Environment variables checked at startup and override programmatic config
- Text format: simple `format!()` with timestamp, level, message, JSON data
- JSON format: `serde_json` serialization of log record struct
- Rotation: `tracing-appender::rolling` with size-based trigger
- `request_logger()` returns a `NativeFunction` that wraps the handler and measures timing

### Why `log_info` not just `info`?

`error` would collide with `std/http/server`'s `error()` response builder. The `log_` prefix is explicit, avoids all collisions, and is consistent with Python's `logging.info()` pattern.

---

## Feature 4: CORS — Global Builtin ✅ COMPLETE

**Implemented exactly as planned.** One function. Call it, done.

**Implementation:**
- `CorsConfig` struct added to `http_server.rs`
- `enable_cors` registered as global builtin in `interpreter.rs`
- OPTIONS preflight handling in both sync and async server loops
- CORS headers applied to all responses automatically
- Skipped in UnitTest mode via `should_skip_server_call`

### API

```ntnt
// Development — allow everything
enable_cors()

// Production — restrict origins
enable_cors(map {
    "origins": ["https://myapp.com", "http://localhost:3000"],
    "credentials": true
})

get("/api/data", fn(req) { json(map { "hello": "world" }) })
listen(8080)
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `origins` | String or Array\<String\> | `"*"` | Allowed origins |
| `methods` | Array\<String\> | GET, POST, PUT, DELETE, PATCH, OPTIONS | Allowed methods |
| `headers` | Array\<String\> | Content-Type, Authorization, Accept | Allowed headers |
| `credentials` | Bool | false | Allow credentials |
| `max_age` | Int | 86400 | Preflight cache seconds |

### Implementation

- Add `CorsConfig` struct + `cors_config: Option<CorsConfig>` to `ServerState` in `src/stdlib/http_server.rs`
- Helper methods: `apply_cors_headers()`, `create_preflight_response()`
- `enable_cors` registered as global builtin in `src/interpreter.rs` (like `listen`, `serve_static`)
- Special-case in `eval_call`: parse options -> `CorsConfig`, store on `server_state`
- Skip in UnitTest mode via `should_skip_server_call`
- Both server loops: handle OPTIONS preflight before route matching, apply CORS headers to all responses

---

## Feature 5: File Uploads — `std/http/server` ✅ COMPLETE

**Note:** Implemented as `parse_multipart` and `save_upload` (kept technical name for clarity since "multipart" describes the HTTP content type).

**Implementation:**
- `parse_multipart(req)` — parses multipart body, returns `Result<Map<String, Any>, String>`
- `save_upload(file, path)` — saves file data to disk, returns `Result<Int, String>` (bytes written)
- Pure Rust boundary-based parsing (no external crate)

**Security Hardening (Added):**
- ✅ Path traversal prevention in `save_upload()` — rejects `..` and null bytes
- ✅ Filename sanitization during multipart parsing — strips path components and dangerous characters
- ✅ Files saved with sanitized names, never with raw user-provided filenames

Intent-revealing names: `parse_upload` (not `parse_multipart` — users think "upload", not "multipart"). Paired with `save_upload`.

### API

```ntnt
import { json, parse_upload, save_upload } from "std/http/server"

fn upload_handler(req: Request) -> Response {
    let fields = parse_upload(req) otherwise {
        return json(map { "error": "Invalid upload: {err}" }, 400)
    }

    let name = fields["name"]        // text field -> String
    let avatar = fields["avatar"]    // file field -> Map

    print(avatar.filename)           // "photo.jpg"
    print(avatar.content_type)       // "image/jpeg"
    print(avatar.size)               // 12345

    save_upload(avatar, "uploads/") otherwise {
        return json(map { "error": "Save failed: {err}" }, 500)
    }

    return json(map { "name": name, "file": avatar.filename }, 201)
}

post("/upload", upload_handler)
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `parse_upload` | `(req) -> Result<Map<String, Any>, String>` | Parse multipart body. Text fields -> String, file fields -> Map. |
| `save_upload` | `(file: Map, path: String) -> Result<Int, String>` | Save file to disk. Path ending in `/` uses original filename. Returns bytes written. |

**File field map:** `{ "filename": String, "content_type": String, "size": Int, "data": String }`

### Implementation

- Pure Rust boundary-based parsing (no new crate)
- Extract boundary from `Content-Type` header, split body on `--{boundary}`
- Parse `Content-Disposition` for field name and filename
- `save_upload` detects if path is a directory (ends with `/` or `is_dir()`) -> appends original filename

### Known limitation (v1)

Binary file data may be lossy — request body passes through String conversion. Text files, CSVs, JSON work perfectly. Binary files (images) are best-effort. Document clearly. This is acceptable for most web apps where files get saved to disk and served back as static files.

---

## Feature 6: Query Builder — `std/db`

A database-agnostic query builder that generates efficient SQL. Works with both SQLite and PostgreSQL connections. Detects the connection type and generates appropriate SQL dialect (`?` vs `$1` placeholders).

### Design Principles

1. **Simple cases stay simple** — `find(db, "users", map { "active": true })` is one line
2. **Complex cases are still readable** — joins, aggregations, subqueries are options, not separate APIs
3. **Generates efficient SQL** — proper indexing hints, batched operations, no N+1 queries
4. **Escape hatch always available** — raw `query()` and `execute()` from `std/db/sqlite` and `std/db/postgres` still work for anything the builder can't express

### Core API

```ntnt
import { connect } from "std/db/sqlite"
import { find, find_one, create, update, delete, count } from "std/db"

let db = connect("app.db")?

// Create
let user = create(db, "users", map {
    "name": "Alice",
    "email": "alice@example.com",
    "age": 30
})?
print(user.id)  // auto-generated ID returned

// Read one
let user = find_one(db, "users", map { "id": 1 })?

// Read many with conditions
let adults = find(db, "users", map { "age >=": 18, "active": true })?

// Read with ordering and limits
let recent = find(db, "users",
    map { "active": true },
    map { "order": "created_at desc", "limit": 10 }
)?

// Update (returns number of rows changed)
let changed = update(db, "users",
    map { "id": 1 },
    map { "name": "Bob", "age": 31 }
)?

// Delete (returns number of rows deleted)
let deleted = delete(db, "users", map { "active": false })?

// Count
let total = count(db, "users", map { "active": true })?
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `find` | `(db, table, where?, options?) -> Result<Array<Map>, String>` | Query rows. Returns array of maps. |
| `find_one` | `(db, table, where?) -> Result<Option<Map>, String>` | Query one row. Returns Some(map) or None. |
| `create` | `(db, table, data) -> Result<Map, String>` | Insert row. Returns the created row with ID. |
| `create_many` | `(db, table, rows) -> Result<Array<Map>, String>` | Insert multiple rows in a single statement. Returns created rows. |
| `update` | `(db, table, where, data) -> Result<Int, String>` | Update rows. Returns count changed. |
| `update_returning` | `(db, table, where, data) -> Result<Array<Map>, String>` | Update rows and return modified rows. |
| `delete` | `(db, table, where) -> Result<Int, String>` | Delete rows. Returns count deleted. |
| `delete_returning` | `(db, table, where) -> Result<Array<Map>, String>` | Delete rows and return removed rows. |
| `upsert` | `(db, table, conflict_keys, data) -> Result<Map, String>` | Insert or update on conflict. |
| `count` | `(db, table, where?) -> Result<Int, String>` | Count matching rows. |
| `exists` | `(db, table, where) -> Result<Bool, String>` | Check if any row matches (efficient). |
| `aggregate` | `(db, table, options) -> Result<Array<Map>, String>` | GROUP BY with aggregate functions. |
| `increment` | `(db, table, where, column, amount?) -> Result<Int, String>` | Atomic increment. Default amount is 1. |
| `decrement` | `(db, table, where, column, amount?) -> Result<Int, String>` | Atomic decrement. Default amount is 1. |
| `transaction` | `(db, fn) -> Result<T, String>` | Execute function atomically. Rollback on error. |

### Where Clause Syntax

Conditions are maps where keys encode the operator:

| Key Pattern | SQL Generated | Example |
|-------------|---------------|---------|
| `"name"` | `name = ?` | `map { "name": "Alice" }` |
| `"age >"` | `age > ?` | `map { "age >": 18 }` |
| `"age >="` | `age >= ?` | `map { "age >=": 18 }` |
| `"age <"` | `age < ?` | `map { "age <": 65 }` |
| `"age <="` | `age <= ?` | `map { "age <=": 65 }` |
| `"age !="` | `age != ?` | `map { "age !=": 0 }` |
| `"name like"` | `name LIKE ?` | `map { "name like": "%alice%" }` |
| `"name not like"` | `name NOT LIKE ?` | `map { "name not like": "%test%" }` |
| `"id in"` | `id IN (?, ?)` | `map { "id in": [1, 2, 3] }` |
| `"id not in"` | `id NOT IN (?, ?)` | `map { "id not in": [4, 5] }` |
| `"email"` + `None` | `email IS NULL` | `map { "email": None }` |
| `"email not"` + `None` | `email IS NOT NULL` | `map { "email not": None }` |
| `"age between"` | `age BETWEEN ? AND ?` | `map { "age between": [18, 65] }` |

Multiple conditions are AND'd together. For OR conditions, use the `"or"` key:

```ntnt
// WHERE (active = true) AND (role = 'admin' OR role = 'moderator')
let staff = find(db, "users", map {
    "active": true,
    "or": [
        map { "role": "admin" },
        map { "role": "moderator" }
    ]
})?
```

### Options Map

| Key | Type | Description | Example |
|-----|------|-------------|---------|
| `order` | String | Sort clause | `"created_at desc"`, `"name asc, age desc"` |
| `limit` | Int | Max rows to return | `10` |
| `offset` | Int | Rows to skip | `20` |
| `select` | Array\<String\> | Specific columns (default: `*`) | `["id", "name", "email"]` |
| `include` | Map | Eager-load related data (JOINs) | See below |
| `distinct` | Bool | SELECT DISTINCT | `true` |

### JOINs with `include`

The `include` option generates JOINs automatically by following foreign key relationships defined in `define_table()`. This eliminates N+1 queries — related data is loaded in a single SQL query.

```ntnt
// Belongs-to: bookmarks.user_id -> users.id (auto-detected from schema)
let bookmarks = find(db, "bookmarks",
    map { "published": true },
    map {
        "include": map {
            "user": map {}   // JOIN users ON bookmarks.user_id = users.id
        },
        "order": "created_at desc",
        "limit": 20
    }
)?
// Each bookmark now has a "user" key: bookmark.user.name

// Has-many: users.id <- bookmarks.user_id (reverse lookup)
let users = find(db, "users",
    map { "active": true },
    map {
        "include": map {
            "bookmarks": map { "limit": 5, "order": "created_at desc" }
        }
    }
)?
// Each user now has a "bookmarks" key: user.bookmarks[0].title

// Many-to-many through join table
let bookmarks = find(db, "bookmarks",
    map { "user_id": user.id },
    map {
        "include": map {
            "tags": map { "through": "bookmark_tags" }
        }
    }
)?
// Each bookmark now has a "tags" key: bookmark.tags[0].name

// Nested includes (2 levels deep max to prevent runaway queries)
let users = find(db, "users", map {}, map {
    "include": map {
        "bookmarks": map {
            "include": map {
                "tags": map { "through": "bookmark_tags" }
            }
        }
    }
})?
```

**How `include` works internally:**

1. The query builder reads the table definitions registered by `define_table()` to discover foreign key relationships
2. For **belongs-to** (e.g., `bookmarks` includes `user`): generates `LEFT JOIN users ON bookmarks.user_id = users.id`, aliases columns to avoid name collisions, and nests the joined columns into a child map on each result row
3. For **has-many** (e.g., `users` includes `bookmarks`): runs a second query `SELECT * FROM bookmarks WHERE user_id IN (?, ?, ?)` using the parent IDs, then merges results as arrays — this is the standard eager-loading strategy (2 queries total, no N+1)
4. For **many-to-many** with `through`: runs two queries — one through the join table to get the related IDs, then one to fetch the related rows. Merges as arrays.
5. Nested `include` applies recursively up to 2 levels deep

**Why not lazy loading?** Lazy loading (accessing `user.bookmarks` triggers a query) is the #1 source of N+1 bugs and is invisible to agents. Explicit `include` makes the queries visible and predictable.

### Aggregations with `aggregate`

```ntnt
import { aggregate } from "std/db"

// Count bookmarks per user
let stats = aggregate(db, "bookmarks", map {
    "group_by": "user_id",
    "count": "*"
})?
// [{ "user_id": 1, "count": 42 }, { "user_id": 2, "count": 17 }, ...]

// Multiple aggregates
let stats = aggregate(db, "bookmarks", map {
    "group_by": "user_id",
    "count": "*",
    "max": "created_at",
    "min": "created_at"
})?
// [{ "user_id": 1, "count": 42, "max_created_at": "...", "min_created_at": "..." }]

// Group by multiple columns
let stats = aggregate(db, "bookmarks", map {
    "group_by": ["user_id", "published"],
    "count": "*"
})?

// HAVING clause (filter on aggregate results)
let power_users = aggregate(db, "bookmarks", map {
    "group_by": "user_id",
    "count": "*",
    "having": map { "count >=": 10 }
})?

// With WHERE + GROUP BY + HAVING + ORDER
let stats = aggregate(db, "bookmarks", map {
    "where": map { "published": true },
    "group_by": "user_id",
    "count": "*",
    "sum": "views",
    "having": map { "sum >=": 100 },
    "order": "sum desc",
    "limit": 10
})?
```

**Aggregate functions supported:**

| Key | SQL | Result Column |
|-----|-----|---------------|
| `"count"` | `COUNT(col)` | `count` or `count_col` |
| `"sum"` | `SUM(col)` | `sum_col` |
| `"avg"` | `AVG(col)` | `avg_col` |
| `"min"` | `MIN(col)` | `min_col` |
| `"max"` | `MAX(col)` | `max_col` |

When `"count": "*"`, the result column is just `count`. When aggregating a specific column like `"sum": "views"`, the result column is `sum_views`.

### Batch Operations

```ntnt
// Insert many rows efficiently (single INSERT statement with multiple value sets)
let users = create_many(db, "users", [
    map { "name": "Alice", "email": "alice@example.com" },
    map { "name": "Bob", "email": "bob@example.com" },
    map { "name": "Charlie", "email": "charlie@example.com" }
])?
```

### Transactions

Wrap multiple operations in an atomic transaction. If any operation fails or the function returns `Err`, all changes are rolled back.

```ntnt
import { transaction, create, update } from "std/db"

// Transfer credits between users — must be atomic
let result = transaction(db, fn() {
    update(db, "users", map { "id": from_id }, map { "credits -=": amount })?
    update(db, "users", map { "id": to_id }, map { "credits +=": amount })?

    create(db, "transfers", map {
        "from_user": from_id,
        "to_user": to_id,
        "amount": amount
    })?

    return Ok(map { "transferred": amount })
})?

// Create user with profile — rollback both if either fails
let user = transaction(db, fn() {
    let user = create(db, "users", map { "name": name, "email": email })?
    create(db, "profiles", map { "user_id": user.id, "bio": "" })?
    return Ok(user)
})?
```

### Upsert (Insert or Update)

Insert a row, or update it if a conflict occurs on the specified keys.

```ntnt
import { upsert } from "std/db"

// Insert new user or update existing by email
let user = upsert(db, "users",
    ["email"],  // conflict key(s)
    map { "email": "alice@example.com", "name": "Alice", "last_seen": now() }
)?

// Multi-column conflict key
let vote = upsert(db, "votes",
    ["user_id", "post_id"],  // composite unique constraint
    map { "user_id": user.id, "post_id": post.id, "value": 1 }
)?
```

Generates:
- PostgreSQL: `INSERT ... ON CONFLICT (email) DO UPDATE SET ...`
- SQLite: `INSERT ... ON CONFLICT (email) DO UPDATE SET ...`

### Atomic Increment/Decrement

Update numeric columns atomically without read-then-write race conditions.

```ntnt
import { increment, decrement } from "std/db"

// Increment page views (default amount is 1)
increment(db, "posts", map { "id": post_id }, "views")?
// Generates: UPDATE posts SET views = views + 1 WHERE id = ?

// Increment by specific amount
increment(db, "users", map { "id": user_id }, "credits", 100)?
// Generates: UPDATE users SET credits = credits + 100 WHERE id = ?

// Decrement stock
decrement(db, "products", map { "id": product_id }, "stock", quantity)?
// Generates: UPDATE products SET stock = stock - ? WHERE id = ?
```

### Existence Check

Efficiently check if any rows match without fetching data.

```ntnt
import { exists } from "std/db"

// Check if email is taken (faster than find_one)
if exists(db, "users", map { "email": email })? {
    return json(map { "error": "Email already registered" }, 409)
}

// Check if user has any posts
let has_posts = exists(db, "posts", map { "user_id": user.id })?
```

Generates: `SELECT EXISTS(SELECT 1 FROM users WHERE email = ?)`

### Returning Modified Rows

Get back the rows that were actually modified by an update or delete.

```ntnt
import { update_returning, delete_returning } from "std/db"

// Deactivate dormant users and get their info for notification
let dormant = update_returning(db, "users",
    map { "last_login <": thirty_days_ago, "active": true },
    map { "active": false, "status": "dormant" }
)?
for user in dormant {
    send_reactivation_email(user.email)
}

// Delete expired sessions and log them
let expired = delete_returning(db, "sessions",
    map { "expires_at <": now() }
)?
log_info("Cleaned sessions", map { "count": len(expired) })
```

PostgreSQL: Uses `RETURNING *` clause.
SQLite: Runs SELECT before the modification (within a transaction for consistency).

### Raw SQL Escape Hatch

For anything the query builder can't express (subqueries, window functions, CTEs, complex joins with custom conditions), the existing `query()` and `execute()` functions from `std/db/sqlite` and `std/db/postgres` are always available:

```ntnt
import { query } from "std/db/sqlite"

// Subquery
let active = query(db, """
    SELECT * FROM users
    WHERE id IN (SELECT user_id FROM bookmarks WHERE created_at > ?)
""", [cutoff_date])?

// Window function
let ranked = query(db, """
    SELECT *, ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY created_at DESC) as rn
    FROM bookmarks
    WHERE rn <= 5
""", [])?

// CTE
let tree = query(db, """
    WITH RECURSIVE tag_tree AS (
        SELECT id, name, parent_id, 0 as depth FROM tags WHERE parent_id IS NULL
        UNION ALL
        SELECT t.id, t.name, t.parent_id, tt.depth + 1
        FROM tags t JOIN tag_tree tt ON t.parent_id = tt.id
    )
    SELECT * FROM tag_tree ORDER BY depth, name
""", [])?
```

The query builder handles 90%+ of queries. Raw SQL handles the rest. No abstraction gap.

### Implementation

- New file `src/stdlib/db.rs` — 15 functions total
- Register as `"std/db"` in `src/stdlib/mod.rs`
- Connection type detection: check if the connection value carries a `"type"` field (`"sqlite"` or `"postgres"`) — add this field to connection values in the sqlite and postgres modules
- SQL generation: build `SELECT`/`INSERT`/`UPDATE`/`DELETE` strings from table name + where map + options
- Parameter binding: collect values in order, pass to underlying `query()`/`execute()` from the appropriate db module
- `create()` returns the inserted row by doing `INSERT...RETURNING *` (postgres) or `INSERT` + `SELECT last_insert_rowid()` then `SELECT * WHERE id = ?` (sqlite)
- `create_many()` generates `INSERT INTO t (cols) VALUES (?, ?), (?, ?), ...` — single statement, single round trip
- `include` reads table definitions from the global `define_table` registry to discover foreign keys and generate appropriate JOIN/subquery strategies
- `aggregate()` builds `SELECT group_cols, AGG(col) FROM table WHERE ... GROUP BY ... HAVING ... ORDER BY ... LIMIT ...`
- OR conditions: `"or"` key generates `(cond1 OR cond2 OR ...)` wrapped in parens
- `transaction()` wraps the callback in `BEGIN`/`COMMIT`, catches errors and runs `ROLLBACK`
- `upsert()` generates `INSERT ... ON CONFLICT (keys) DO UPDATE SET ...`
- `increment()`/`decrement()` generate `UPDATE ... SET col = col + ?`
- `exists()` generates `SELECT EXISTS(SELECT 1 FROM ...)`
- `update_returning()`/`delete_returning()` use `RETURNING *` (postgres) or SELECT-before-modify (sqlite)

---

## Feature 7: Schema Sync — Declarative Database Migrations

This is the big one. Traditional migrations are the #1 thing that trips agents up. The problem isn't the concept — it's the execution model.

### The Problem with Migration Files

Migration files encode *transitions* (how to get from version N to N+1). Agents think in *states* (what the schema should be). This mismatch causes:

1. **Ordering conflicts** — multiple migrations touch the same table
2. **State blindness** — to understand the current schema, you have to replay all migrations mentally
3. **Destructive panic** — when stuck, agents add `DROP TABLE` to a migration
4. **Dev/staging/prod drift** — `migrate dev` vs `migrate deploy` vs `db push`
5. **Recovery is archaeology** — fixing a bad migration requires understanding the full history

### The Solution: Schema-as-Code

Inspired by Terraform's `plan` + `apply` model. **The schema definition IS the source of truth.** No migration files. No ordering. The tool diffs declared state against actual state and applies changes.

### API (NTNT code)

```ntnt
// db/schema.tnt — the single source of truth

import { define_table } from "std/db/schema"

define_table("users", map {
    "name": "text required",
    "email": "text unique",
    "age": "integer default:0",
    "admin": "boolean default:false",
    "created_at": "timestamp default:now",
    "updated_at": "timestamp default:now"
})

define_table("posts", map {
    "title": "text required",
    "body": "text",
    "user_id": "integer references:users",
    "published": "boolean default:false",
    "views": "integer default:0",
    "created_at": "timestamp default:now"
})

define_table("tags", map {
    "name": "text required unique"
})

define_table("post_tags", map {
    "post_id": "integer references:posts",
    "tag_id": "integer references:tags"
})
```

Every table automatically gets:
- `id` column (integer, primary key, auto-increment) — always, no need to declare

Columns are defined as `"type modifiers..."` strings for maximum readability.

### Column Type Syntax

```
"text"                       -> TEXT / VARCHAR
"text required"              -> TEXT NOT NULL
"text unique"                -> TEXT UNIQUE
"text required unique"       -> TEXT NOT NULL UNIQUE
"integer"                    -> INTEGER
"integer default:0"          -> INTEGER DEFAULT 0
"integer references:users"   -> INTEGER REFERENCES users(id)
"boolean default:false"      -> BOOLEAN DEFAULT FALSE
"float"                      -> REAL / FLOAT
"timestamp default:now"      -> TIMESTAMP DEFAULT CURRENT_TIMESTAMP
"blob"                       -> BLOB
```

Modifiers: `required`, `unique`, `default:<value>`, `references:<table>`, `index`

### CLI Commands

```bash
# Show what would change (safe — read-only)
ntnt db diff schema.tnt --db app.db

# Output:
#   CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, ...)
#   CREATE TABLE posts (...)
#   --- or ---
#   ALTER TABLE users ADD COLUMN admin BOOLEAN DEFAULT FALSE
#   ALTER TABLE users DROP COLUMN legacy_field    <- highlighted in red

# Apply changes
ntnt db sync schema.tnt --db app.db

# Output:
#   Comparing schema to database...
#   + Add column: users.admin (boolean default:false)
#   + Create table: tags
#   + Create table: post_tags
#   - Drop column: users.legacy_field    <- requires --allow-destructive
#
#   Apply 3 changes? [y/n]

# Apply changes non-interactively (CI/CD)
ntnt db sync schema.tnt --db app.db --yes

# Destructive changes require explicit flag
ntnt db sync schema.tnt --db app.db --allow-destructive

# Reset database (drop all, recreate from schema)
ntnt db reset schema.tnt --db app.db

# Generate SQL file for review (production workflow)
ntnt db plan schema.tnt --db app.db > migration.sql
# Then: ntnt db apply migration.sql --db app.db
```

### Safety Model

| Change Type | Default Behavior | Override |
|-------------|-----------------|----------|
| Create table | Applied | — |
| Add column | Applied | — |
| Add index | Applied | — |
| Drop column | **Blocked** | `--allow-destructive` |
| Drop table | **Blocked** | `--allow-destructive` |
| Change column type | **Blocked** | `--allow-destructive` |
| Rename column | Detected as drop+add, **blocked** | `--allow-destructive` |

Before any destructive change, `ntnt db sync` creates an automatic backup: `app.db.backup.2026-02-02T10-30-00`

### How sync_schema Works Internally — Step by Step

The sync process has 5 distinct phases:

#### Phase 1: Parse Schema Definitions

Execute the `.tnt` file, collecting all `define_table()` calls. Each call registers a `TableDef` into a global vector:

```rust
struct TableDef {
    name: String,
    columns: Vec<ColumnDef>,
}

struct ColumnDef {
    name: String,
    col_type: ColumnType,    // Text, Integer, Boolean, Float, Timestamp, Blob
    required: bool,
    unique: bool,
    default: Option<String>,
    references: Option<String>,  // foreign key target table
    index: bool,
}
```

The `"text required unique"` strings are parsed by splitting on whitespace: first token is the type, remaining tokens are modifiers parsed as key or key:value pairs.

#### Phase 2: Read Actual Database Schema

Query the database to discover what currently exists:

**SQLite:**
```sql
-- Get all tables
SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';

-- For each table, get columns
PRAGMA table_info(users);
-- Returns: cid, name, type, notnull, dflt_value, pk

-- Get indexes
PRAGMA index_list(users);
PRAGMA index_info(index_name);

-- Get foreign keys
PRAGMA foreign_key_list(users);
```

**PostgreSQL:**
```sql
-- Get all tables
SELECT table_name FROM information_schema.tables WHERE table_schema = 'public';

-- Get columns
SELECT column_name, data_type, is_nullable, column_default
FROM information_schema.columns WHERE table_name = 'users';

-- Get constraints
SELECT constraint_name, constraint_type
FROM information_schema.table_constraints WHERE table_name = 'users';

-- Get foreign keys
SELECT kcu.column_name, ccu.table_name AS foreign_table
FROM information_schema.key_column_usage kcu
JOIN information_schema.constraint_column_usage ccu
    ON kcu.constraint_name = ccu.constraint_name
WHERE kcu.table_name = 'users';
```

Both paths build the same `ActualSchema` struct — a normalized representation of what the database currently looks like.

#### Phase 3: Diff

Compare declared tables against actual tables and produce a list of `SchemaChange` operations:

```rust
enum SchemaChange {
    CreateTable { name: String, columns: Vec<ColumnDef> },
    DropTable { name: String },                              // destructive
    AddColumn { table: String, column: ColumnDef },
    DropColumn { table: String, column: String },            // destructive
    ChangeColumnType { table: String, column: String, ... }, // destructive
    AddIndex { table: String, column: String },
    DropIndex { table: String, index: String },              // destructive
    AddForeignKey { table: String, column: String, references: String },
}
```

The diff algorithm:
1. Tables in declared but not in actual → `CreateTable`
2. Tables in actual but not in declared → `DropTable` (destructive)
3. For each table in both: compare columns
   - Column in declared but not actual → `AddColumn`
   - Column in actual but not declared → `DropColumn` (destructive)
   - Column in both but type/modifiers differ → `ChangeColumnType` (destructive)
4. Compare indexes and foreign keys similarly

#### Phase 4: Safety Check + Display

Display the diff with color coding:
- Green `+` for additive changes
- Red `-` for destructive changes (blocked unless `--allow-destructive`)

If any destructive changes exist and `--allow-destructive` is not set, print a warning and stop. No changes are applied.

If running interactively and there are non-destructive changes, prompt `Apply N changes? [y/n]` (skip with `--yes`).

#### Phase 5: Apply

Generate and execute DDL statements in dependency order:
1. Create new tables (ordered by foreign key dependencies — if `posts` references `users`, create `users` first)
2. Add columns to existing tables
3. Add indexes
4. Add foreign keys
5. (If `--allow-destructive`) Drop foreign keys, drop indexes, drop columns, drop tables (reverse order)

Each DDL statement is executed individually. If any fails, the error is reported with the specific SQL that failed. SQLite operations use a transaction so partial failures can be rolled back. PostgreSQL uses a transaction as well (DDL is transactional in Postgres).

### Programmatic API (in .tnt code)

```ntnt
import { define_table, sync_schema } from "std/db/schema"
import { connect } from "std/db/sqlite"

define_table("users", map { "name": "text required", "email": "text unique" })
define_table("posts", map { "title": "text required", "user_id": "integer references:users" })

let db = connect("app.db")?
sync_schema(db)  // Apply all define_table declarations to this database
```

`sync_schema(db)` runs Phases 1-5 programmatically. It only applies non-destructive changes (equivalent to running without `--allow-destructive`). If destructive changes would be needed, it logs a warning to stderr but does not block execution — the additive changes still apply. This makes it safe to call on every server start.

### Why This Is Better for Agents

| Traditional Migrations | Schema Sync |
|----------------------|-------------|
| Agent must create numbered files | Agent edits one schema file |
| Agent must track migration history | Tool tracks actual DB state |
| Ordering conflicts between features | No ordering — declarative |
| `dev` vs `deploy` vs `push` commands | One command: `sync` |
| Rollback is a separate concept | Just change the schema and sync |
| Recovery requires migration archaeology | `reset` recreates from scratch |
| Agent can write destructive migrations | Destructive changes blocked by default |

### Known Limitations (Be Honest With Users)

Schema sync handles ~85% of real-world schema changes cleanly. Here's what it can't do:

| Limitation | Why It Happens | Workaround |
|------------|----------------|------------|
| **Rename detection** | Renaming `name` → `full_name` looks like "drop + add" | Use raw SQL: `ALTER TABLE users RENAME COLUMN name TO full_name` |
| **Data migrations** | "Split column X into Y and Z" needs `UPDATE` statements | Write a `.tnt` script with raw SQL |
| **Column type changes with data** | Changing `age: text` → `age: integer` needs conversion | Raw SQL with `CAST()` or temp column |
| **Large table index creation** | Adding index on 10M rows can lock table | Use `CREATE INDEX CONCURRENTLY` (Postgres) via raw SQL |
| **Circular foreign keys** | A → B → C → A creates dependency loop | Create tables first, add FKs after with raw SQL |

**These are intentionally left to raw SQL** because they're inherently imperative operations. Trying to encode them declaratively adds the exact complexity we're avoiding.

### Production Workflow — Migration Generation

For production deployments where you want reviewable, auditable changes in git:

```bash
# Generate a SQL migration file (doesn't apply changes)
ntnt db diff schema.tnt --db prod.db --save migrations/003_add_profiles.sql

# Output file: migrations/003_add_profiles.sql
# -- Generated by ntnt db diff at 2026-02-02T10:30:00Z
# -- Schema: schema.tnt -> postgres://prod-db:5432/app
#
# ALTER TABLE users ADD COLUMN avatar_path TEXT;
# CREATE TABLE profiles (...);
# CREATE INDEX idx_profiles_user_id ON profiles(user_id);

# Review in PR, then apply in CI/CD:
ntnt db apply migrations/ --db prod.db
```

**How `ntnt db apply` works:**
1. Reads all `.sql` files in the migrations directory
2. Checks `_ntnt_migrations` table to see which have been applied
3. Applies pending migrations in filename order (sort alphabetically)
4. Records each successful migration in `_ntnt_migrations`

This gives you the best of both worlds:
- **Development**: Fast iteration with `ntnt db sync`
- **Production**: Reviewable SQL files, auditable history, CI/CD friendly

### Database Seeding

For reference data, test fixtures, and initial setup:

```bash
# Run all seed files
ntnt db seed seeds/ --db app.db

# Run specific seed file
ntnt db seed seeds/reference_data.tnt --db app.db

# Run seeds only if table is empty (idempotent)
ntnt db seed seeds/ --db app.db --if-empty
```

**Seed file example:**

```ntnt
// seeds/reference_data.tnt
import { connect } from "std/db/sqlite"
import { upsert, count } from "std/db"

let db = connect(get_env("DATABASE_URL"))?

// Idempotent — uses upsert so safe to run multiple times
let countries = [
    map { "code": "US", "name": "United States" },
    map { "code": "CA", "name": "Canada" },
    map { "code": "GB", "name": "United Kingdom" },
    // ...
]

for country in countries {
    upsert(db, "countries", ["code"], country)?
}

// Seed admin user only if no users exist
if count(db, "users", map {})? == 0 {
    create(db, "users", map {
        "email": "admin@example.com",
        "name": "Admin",
        "admin": true
    })?
}

print("Seeded {len(countries)} countries")
```

**Seed files are just `.tnt` files** — full language access, conditional logic, can read env vars.

### Drift Detection

Check if a deployed database matches the expected schema:

```bash
# Exit 0 if schema matches, exit 1 if drift detected
ntnt db check schema.tnt --db prod.db

# Output on drift:
#   Schema drift detected!
#   - Missing column: users.avatar_path
#   - Extra table: legacy_logs (not in schema)
#   Run 'ntnt db diff' to see required changes.

# Use in CI/CD to catch forgotten migrations:
ntnt db check schema.tnt --db prod.db || echo "Schema out of sync!"
```

**Why this matters:**
- Catch "forgot to run migrations in production" errors
- Verify staging matches production
- Pre-deployment safety check in CI/CD pipelines

### Updated CLI Commands

```bash
# Development workflow
ntnt db diff schema.tnt --db app.db          # Show what would change (read-only)
ntnt db sync schema.tnt --db app.db          # Apply changes interactively
ntnt db sync schema.tnt --db app.db --yes    # Apply without prompts (CI)
ntnt db reset schema.tnt --db app.db         # Drop all, recreate from schema

# Production workflow
ntnt db diff schema.tnt --db prod.db --save migrations/003_name.sql  # Generate migration
ntnt db apply migrations/ --db prod.db       # Apply pending migrations
ntnt db check schema.tnt --db prod.db        # Verify schema matches (CI gate)

# Seeding
ntnt db seed seeds/ --db app.db              # Run all seed files
ntnt db seed seeds/users.tnt --db app.db     # Run specific seed
ntnt db seed seeds/ --db app.db --if-empty   # Only seed empty tables
```

---

## Feature 8: KV Store — `std/kv` ✅ COMPLETE

A unified key-value interface that works with both a built-in SQLite backend (zero-config, good for development and small deployments) and Redis/Valkey (production-grade, distributed).

**Implemented:**
- ✅ SQLite backend — 9 functions (open, get, set, del, has, list, expire, ttl, flush)
- ✅ Redis/Valkey backend — Same 9 functions, uses redis crate
- ✅ URL-based backend selection: file paths → SQLite, `redis://` or `valkey://` → Redis protocol
- ✅ Type-preserving serialization (strings, ints, floats, bools, maps, arrays)
- ✅ TTL support on both backends

## Feature 8: KV Store — `std/kv`

A unified key-value interface that works with both a built-in SQLite backend (zero-config, good for development and small deployments) and Redis/Valkey (production-grade, distributed).

### Redis and Valkey Compatibility

**Valkey is a fork of Redis 7.2.4** (created in March 2024 by the Linux Foundation after Redis changed its license). Both use the identical RESP (Redis Serialization Protocol), so:

- The `redis` Rust crate works with **both Redis and Valkey** servers
- Same connection URLs, same commands, same data formats
- Users can switch between Redis and Valkey without code changes
- We support both `redis://` and `valkey://` URL schemes (they're aliases)

This means NTNT's KV store works with:
- **Redis** (original, now source-available license)
- **Valkey** (open source fork, BSD-3 license)
- **AWS ElastiCache** (supports both Redis and Valkey modes)
- **Any Redis-compatible service** (Upstash, Redis Cloud, etc.)

### Design Principles

1. **Same API, swap backends** — `open("cache.db")` vs `open("redis://host:6379")` — the code doesn't change
2. **Everything is a string or a map** — no complex serialization concerns; maps are JSON-serialized transparently
3. **TTL is a first-class concept** — every key can have an expiration
4. **Prefix listing** — the most common KV pattern is namespace-based access: `list(kv, "session:")` returns all session keys

### API

```ntnt
import { open, get, set, del, has, list, expire, ttl, flush } from "std/kv"

// SQLite backend (bundled, zero-config)
let cache = open("cache.db")?

// Redis/Valkey backend (production) - both URL schemes work identically
let cache = open("redis://localhost:6379")?
let cache = open("valkey://localhost:6379")?  // alias for redis://

// With authentication:
let cache = open("redis://user:password@host:6379/0")?
let cache = open("valkey://user:password@host:6379/0")?

// Basic operations
set(cache, "user:123", map { "name": "Alice", "role": "admin" })?
set(cache, "config:theme", "dark")?

let user = get(cache, "user:123")?       // Some(map { "name": "Alice", ... })
let missing = get(cache, "nonexistent")? // None

let exists = has(cache, "user:123")?     // true
del(cache, "user:123")?

// TTL support (seconds)
set(cache, "session:abc", token, map { "ttl": 3600 })?     // expires in 1 hour
set(cache, "rate:192.168.1.1", 1, map { "ttl": 60 })?      // rate limit window

let remaining = ttl(cache, "session:abc")?   // Some(3542) seconds remaining
expire(cache, "user:123", 600)?              // set TTL on existing key

// Prefix listing
let sessions = list(cache, "session:")?      // ["session:abc", "session:def", ...]
let all_users = list(cache, "user:")?        // ["user:123", "user:456", ...]
let everything = list(cache)?                // all keys (use sparingly)

// Flush all keys (for tests/reset)
flush(cache)?
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `open` | `(url: String) -> Result<KVStore, String>` | Open a KV store. `"file.db"` for SQLite, `"redis://..."` or `"valkey://..."` for Redis/Valkey. |
| `get` | `(kv, key: String) -> Result<Option<Any>, String>` | Get value by key. None if not found or expired. |
| `set` | `(kv, key: String, value: Any, opts?) -> Result<Unit, String>` | Set key to value. Opts: `{ "ttl": Int }`. |
| `del` | `(kv, key: String) -> Result<Bool, String>` | Delete key. Returns true if existed. |
| `has` | `(kv, key: String) -> Result<Bool, String>` | Check if key exists and is not expired. |
| `list` | `(kv, prefix?: String) -> Result<Array<String>, String>` | List keys matching prefix (or all keys). |
| `expire` | `(kv, key: String, seconds: Int) -> Result<Bool, String>` | Set TTL on existing key. |
| `ttl` | `(kv, key: String) -> Result<Option<Int>, String>` | Get remaining TTL in seconds. None if no expiry. |
| `flush` | `(kv) -> Result<Unit, String>` | Delete all keys. |

### Why `del` not `delete`?

`delete` is already used by the query builder (`std/db`) for SQL DELETE. `del` is the standard KV convention (Redis uses `DEL`, Python dicts use `del`). Short, unambiguous, no collision.

### Value Serialization

- **Strings** → stored as-is
- **Integers, Floats, Bools** → stored as string representation, restored to original type on `get()`
- **Maps and Arrays** → JSON-serialized on `set()`, JSON-deserialized on `get()`
- **None** → deletes the key (setting a key to None is equivalent to `del`)

Type information is stored alongside the value so `get()` returns the correct type:

```
key: "user:123"
value: '{"name":"Alice","role":"admin"}'
type: "map"
ttl: 1706886000  (unix timestamp when it expires, or NULL for no expiry)
```

### SQLite Backend Schema

```sql
CREATE TABLE IF NOT EXISTS _kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'string',
    expires_at INTEGER  -- unix timestamp, NULL = no expiry
);

CREATE INDEX IF NOT EXISTS _kv_expires ON _kv(expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS _kv_prefix ON _kv(key);  -- for prefix listing
```

Expired keys are cleaned up lazily (on access) and periodically (every 60 seconds when the KV store is in use). This matches Redis/Valkey's behavior.

### Redis/Valkey Backend

Uses the `redis` crate (add `redis = { version = "0.25", features = ["tokio-comp"] }` to Cargo.toml). The crate works with both Redis and Valkey servers since they use the same RESP protocol.

**URL Scheme Handling:**
- `redis://host:port` → connects via Redis protocol
- `valkey://host:port` → also connects via Redis protocol (alias)
- Both support: `user:password@`, `/database_number`, `?params`

Maps directly to Redis/Valkey commands:

| NTNT Function | Redis/Valkey Command |
|---------------|----------------------|
| `get(kv, key)` | `GET key` |
| `set(kv, key, val)` | `SET key val` |
| `set(kv, key, val, map { "ttl": 60 })` | `SETEX key 60 val` |
| `del(kv, key)` | `DEL key` |
| `has(kv, key)` | `EXISTS key` |
| `list(kv, "session:")` | `SCAN 0 MATCH session:*` |
| `expire(kv, key, 60)` | `EXPIRE key 60` |
| `ttl(kv, key)` | `TTL key` |
| `flush(kv)` | `FLUSHDB` |

Maps/arrays are stored as JSON strings with a type prefix (`map:` or `array:`) to distinguish from plain strings on retrieval.

### The Caching Pattern

The KV store is designed to work as a transparent caching layer in front of the database. Here's the pattern:

```ntnt
import { open, get, set, del } from "std/kv"
import { find_one, update } from "std/db"

let cache = open("cache.db")?

// Cache-aside pattern: check cache first, fall back to DB
fn get_user(db, cache, user_id) {
    let key = "user:{user_id}"

    // Try cache first
    let cached = get(cache, key)?
    if cached != None {
        return Ok(cached)
    }

    // Cache miss — query DB
    let user = find_one(db, "users", map { "id": user_id })?
    match user {
        Some(u) => {
            set(cache, key, u, map { "ttl": 300 })?  // cache for 5 min
            return Ok(Some(u))
        },
        None => return Ok(None)
    }
}

// Invalidate on write
fn update_user(db, cache, user_id, data) {
    let changed = update(db, "users", map { "id": user_id }, data)?
    del(cache, "user:{user_id}")?  // bust cache
    return Ok(changed)
}
```

### Implementation

- New file `src/stdlib/kv.rs`
- Register as `"std/kv"` in `src/stdlib/mod.rs`
- Add `redis = { version = "0.27", features = ["tokio-comp"] }` to `Cargo.toml`
- SQLite backend: uses `rusqlite` (already a dependency) with the schema above
- Redis/Valkey backend: uses `redis` crate with sync connection (blocking commands)
- Backend selection: parse the URL passed to `open()`:
  - File paths (no `://`) → SQLite backend
  - `redis://` URLs → Redis protocol via `redis` crate
  - `valkey://` URLs → also Redis protocol (Valkey is wire-compatible)
- The `KVStore` value carries a `"backend"` field so functions know which implementation to call
- URL parsing: strip `valkey://` → `redis://` before passing to `redis` crate (it only recognizes `redis://`)

### Roadmap: `std/kv` is Built to Grow

The initial implementation covers the core operations. Future additions (not in this plan):
- `incr(kv, key)` / `decr(kv, key)` — atomic counters
- `list_push` / `list_pop` / `list_range` — list operations
- `set_add` / `set_members` / `set_remove` — set operations
- Pub/sub via Valkey channels
- Connection pooling for Valkey

---

## Feature 9: Declarative Route Blocks — `server` Syntax ✅ COMPLETE

> Reference: `plans/syntax-analysis-and-innovation.md` section 3.2

**Status:** Implemented with all planned features plus performance optimization.

**What was implemented:**
- ✅ `server PORT { ... }` block syntax
- ✅ `GET/POST/PUT/PATCH/DELETE /path -> handler` route syntax
- ✅ `static "/prefix" from "./dir"` for static files
- ✅ `cors map { ... }` for CORS configuration
- ✅ `middleware [fn1, fn2]` for middleware
- ✅ `group "/prefix" { ... }` for nested route groups
- ✅ Typed route parameters `{id: Int}` with automatic 400 on type mismatch
- ✅ Route conflict detection at parse time
- ✅ **Bonus:** Route matching optimization (HashMap by method+segment count)

**Files changed:**
- `src/lexer.rs` — Added `Server`, `Group` tokens
- `src/ast.rs` — Added `ServerRoute`, `ServerDirective`, `ServerGroup`, `Statement::Server`
- `src/parser.rs` — Added `server_declaration()` and related parsing
- `src/interpreter.rs` — Added `eval_server_block()` desugaring
- `src/stdlib/http_server.rs` — Added typed params to `RouteSegment`, route index for O(1) lookup

Current route definitions are function calls scattered through a file. They're opaque to the parser — it can't validate route patterns, detect conflicts, or generate API documentation.

### The `server` Block

```ntnt
server 8080 {
    static "/assets" from "./public"

    cors map {
        "origins": ["https://myapp.com"],
        "credentials": true
    }

    middleware [request_logger(), authenticate]

    GET    /                    -> home
    GET    /users/{id: Int}     -> get_user
    POST   /users               -> create_user
    PUT    /users/{id: Int}     -> update_user
    DELETE /users/{id: Int}     -> delete_user

    // Grouped routes with shared middleware
    group "/admin" {
        middleware [require_admin]

        GET    /                -> admin_dashboard
        GET    /users           -> admin_users
        DELETE /users/{id: Int} -> admin_delete_user
    }
}
```

### Benefits

1. **Compile-time route validation** — the parser can check for conflicting patterns (`GET /users/{id}` vs `GET /users/{name}` is a conflict)
2. **Type-safe parameters** — `{id: Int}` means the runtime converts and validates before calling the handler. If the parameter isn't a valid integer, the request gets a 400 response automatically.
3. **Self-documenting** — the server block IS the API surface. One glance shows every route.
4. **Middleware scoping** — middleware can be applied globally (top-level) or to a route group
5. **Visual clarity** — HTTP method, path, and handler are aligned in columns

### Syntax Details

| Element | Syntax | Description |
|---------|--------|-------------|
| Port | `server 8080 { }` | The port to listen on |
| Static files | `static "/prefix" from "./dir"` | Serve static files |
| CORS | `cors map { ... }` | Same options as `enable_cors()` |
| Middleware | `middleware [fn1, fn2]` | Applied to all routes below in scope |
| Route | `METHOD /path -> handler` | Map HTTP method + path to handler function |
| Typed param | `{name: Type}` | Auto-convert and validate path parameter |
| Route group | `group "/prefix" { ... }` | Nested routes sharing a prefix and middleware |
| Shutdown hook | `on_shutdown fn_name` | Cleanup handler |

### Type-Safe Parameters

```ntnt
// {id: Int} — runtime auto-converts the path segment to Int
// If conversion fails, returns 400 Bad Request before the handler runs
GET /users/{id: Int} -> get_user

// Handler receives typed params
fn get_user(req: Request) -> Response {
    let id = req.params.id    // Already an Int, not a String
    let user = find_one(db, "users", map { "id": id })?
    // ...
}

// Without type annotation, param is a String (current behavior)
GET /users/{slug} -> get_user_by_slug
```

Supported parameter types: `Int`, `String` (default), `Float`.

### Coexistence with Function Calls

The `server` block is syntactic sugar — it compiles down to the same internal route registrations as `get()`/`post()`/`listen()`. Both forms coexist:

- **`server` block** — the default for static route tables (90% of apps). Use when routes are known at write time.
- **Function calls** — escape hatch for dynamic/programmatic route registration. Use when building routes from config files, conditional routes at runtime, etc.

```ntnt
// These produce identical results:

// server block (preferred for static routes)
server 8080 {
    GET /health -> health_check
}

// function calls (for dynamic routes)
get("/health", health_check)
listen(8080)
```

### Implementation ✅ COMPLETE

- ✅ Added `server` block to the parser in `src/parser.rs` — new AST node `Statement::Server`
- ✅ Parse `METHOD /path -> handler` as route entries, `static`, `cors`, `middleware`, `group` as block directives
- ✅ Parse `{name: Type}` in route paths as typed parameters (`TypedRouteParam` struct)
- ✅ In `src/interpreter.rs`, `eval_server_block()` desugars to existing route registration calls
- ✅ Route conflict detection via `detect_route_conflict()` in `ServerState`
- ✅ Parameter type coercion returns 400 on mismatch via `RouteMatchResult::TypeMismatch`
- ✅ Context-sensitive keyword parsing (static/cors/middleware only recognized inside server blocks)

### Performance Optimization ✅ COMPLETE

Route matching optimized from O(n) to O(n/m/k) via two-level HashMap:
- Routes indexed by `(method, segment_count)` tuple
- Typical lookup scans 1-5 routes instead of all routes
- Added `route_index: HashMap<(String, usize), Vec<usize>>` to `ServerState`
- Zero overhead for route registration, significant speedup for matching

### Tests ✅ PASSING

- ✅ Parse valid server blocks
- ✅ Detect route conflicts at parse time
- ✅ Type-safe parameter coercion (valid Int, invalid Int -> 400)
- ✅ Middleware scoping within groups
- ✅ All existing route tests pass with optimization

---

## Feature 10: Auth Module — `std/auth` ✅ COMPLETE

A complete authentication module supporting OAuth 2.0/OIDC federation and JWT tokens. Designed for the common case: NTNT apps as OAuth **clients** (Login with Google/GitHub/Okta), not OAuth servers.

> **Implementation Note:** Full OAuth 2.0 + OIDC support was implemented with a slightly different API than originally planned. TOTP/MFA was deferred to a future release.

### What Was Implemented

| Feature | Status | Notes |
|---------|--------|-------|
| OAuth 2.0 Authorization Code flow | ✅ | With PKCE support |
| OAuth 2.0 Client Credentials flow | ✅ | For M2M authentication |
| OAuth 2.0 Refresh Token flow | ✅ | `oauth_refresh()` function |
| OIDC ID Token extraction/validation | ✅ | Nonce + issuer/audience validation |
| OIDC Discovery | ✅ | `oauth_discover()` for enterprise providers |
| 10 built-in providers | ✅ | Google, GitHub, Facebook, Microsoft, Discord, Twitter, LinkedIn, Apple, Okta, Auth0 |
| JWT signing/verification | ✅ | HS256/HS384/HS512 |
| Session management | ✅ | In-memory, SQLite, or PostgreSQL backends |
| Token validation for APIs | ✅ | `oauth_validate_token()`, `oauth_introspect()` |
| TOTP/MFA | ❌ Deferred | Planned for future release |

### Actual API (as implemented)

```ntnt
import { get_env } from "std/env"
import { json } from "std/http/server"

// Configure OAuth providers using oauth() helper
let google = oauth("google", map {
    "client_id": get_env("GOOGLE_CLIENT_ID"),
    "client_secret": get_env("GOOGLE_CLIENT_SECRET"),
    "redirect_uri": "http://localhost:8080/auth/google/callback"
})

let github = oauth("github", map {
    "client_id": get_env("GITHUB_CLIENT_ID"),
    "client_secret": get_env("GITHUB_CLIENT_SECRET"),
    "redirect_uri": "http://localhost:8080/auth/github/callback"
})

// Register providers with enable_auth (auto-registers routes)
enable_auth([google, github], map {
    "session_secret": get_env("SESSION_SECRET"),
    "after_login": "/dashboard",
    "after_logout": "/",
    // Session storage (optional, defaults to "memory")
    // "session_store": "memory"                        // In-memory (default)
    // "session_store": "sqlite:./sessions.db"          // SQLite file
    // "session_store": "postgres://user:pass@host/db"  // PostgreSQL
    // "session_store": "redis://localhost:6379"        // Redis/Valkey
})

// Auto-registered routes:
// GET  /auth/google          -> Redirect to Google
// GET  /auth/google/callback -> Handle callback
// GET  /auth/github          -> Redirect to GitHub
// GET  /auth/github/callback -> Handle callback
// POST /auth/logout          -> Clear session
// GET  /auth/me              -> Current user as JSON

listen(8080)
```

### Exported Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `oauth` | `(provider: String, config: Map) -> Map` | Create provider config for built-in providers |
| `oauth_discover` | `(issuer: String, config: Map) -> Result<Map, String>` | Fetch OIDC discovery and create provider config |
| `oauth_client_credentials` | `(provider: Map, scopes?: Array) -> Result<Map, String>` | M2M authentication (client credentials flow) |
| `oauth_refresh` | `(provider: Map, refresh_token: String) -> Result<Map, String>` | Refresh an access token |
| `oauth_validate_token` | `(token: String, opts: Map) -> Result<Map, String>` | Validate a JWT token (for APIs as resource servers) |
| `oauth_introspect` | `(token: String, provider: Map) -> Result<Map, String>` | Token introspection (RFC 7662) |
| `enable_auth` | `(providers: Array, opts: Map) -> Unit` | Register providers and auto-create routes |
| `get_user` | `(req: Request) -> Option<Map>` | Get current user from session |
| `get_session` | `(req: Request) -> Option<Map>` | Get full session data |
| `logout_user` | `(req: Request) -> Response` | Clear session and redirect |
| `jwt_sign` | `(claims: Map, secret: String, opts?: Map) -> Result<String, String>` | Sign a JWT |
| `jwt_verify` | `(token: String, secret: String, opts?: Map) -> Result<Map, String>` | Verify and decode a JWT |
| `jwt_decode` | `(token: String) -> Result<Map, String>` | Decode JWT without verification |

### Session Storage Backends

Sessions can be stored in-memory (default), SQLite, PostgreSQL, or Redis/Valkey. Configure via `session_store` option:

```ntnt
// In-memory (default) — sessions lost on server restart
enable_auth([github], map {
    "session_secret": "...",
    "session_store": "memory"
})

// SQLite — persistent, file-based storage
enable_auth([github], map {
    "session_secret": "...",
    "session_store": "sqlite:./sessions.db"
})

// PostgreSQL — for distributed/clustered deployments
enable_auth([github], map {
    "session_secret": "...",
    "session_store": "postgres://user:pass@localhost/myapp"
})

// Redis/Valkey — for high-performance distributed sessions
enable_auth([github], map {
    "session_secret": "...",
    "session_store": "redis://localhost:6379"
})
```

| Backend | Format | Use Case |
|---------|--------|----------|
| Memory | `"memory"` | Development, single-instance |
| SQLite | `"sqlite:./path.db"` | Production, single-instance |
| PostgreSQL | `"postgres://..."` | Production, clustered |
| Redis | `"redis://..."` | Production, high-performance clustered |
| Valkey | `"valkey://..."` | Production, Redis-compatible (AWS ElastiCache) |

The database backends auto-create the `auth_sessions` table with proper indexes. Redis uses `ntnt:session:{id}` keys with automatic TTL expiration.

### Built-in Providers

| Provider | Protocol | PKCE | Notes |
|----------|----------|------|-------|
| `google` | OIDC | Optional | Full OIDC support |
| `github` | OAuth 2.0 | No | Requires email scope for email |
| `facebook` | OAuth 2.0 | Optional | |
| `microsoft` | OIDC | Optional | Azure AD |
| `discord` | OAuth 2.0 | No | |
| `twitter` | OAuth 2.0 | Required | Always uses PKCE |
| `linkedin` | OAuth 2.0 | No | Uses OpenID Connect scopes |
| `apple` | OIDC | Optional | Uses ID token (no userinfo endpoint) |
| `okta` | OIDC | Optional | Requires `oauth_discover()` with issuer |
| `auth0` | OIDC | Optional | Requires `oauth_discover()` with issuer |

### Enterprise OIDC (Okta, Auth0, Keycloak)

```ntnt
// Use oauth_discover for any OIDC-compliant provider
let okta = oauth_discover("https://mycompany.okta.com", map {
    "client_id": get_env("OKTA_CLIENT_ID"),
    "client_secret": get_env("OKTA_CLIENT_SECRET"),
    "redirect_uri": "http://localhost:8080/auth/okta/callback",
    "scopes": ["openid", "email", "profile", "groups"]
})?

enable_auth([okta], map { ... })
```

### Machine-to-Machine (M2M) Authentication

```ntnt
// Client credentials flow for backend services
let tokens = oauth_client_credentials(google, ["https://www.googleapis.com/auth/cloud-platform"])?
// tokens = { access_token, token_type, expires_in }

// Use token to call APIs
let resp = fetch(map {
    "url": "https://api.example.com/data",
    "headers": map { "Authorization": "Bearer {tokens.access_token}" }
})
```

### Token Validation for APIs

```ntnt
// Validate incoming bearer tokens (API as resource server)
fn api_handler(req: Request) -> Response {
    let auth_header = req.headers["authorization"]
    let token = replace(auth_header, "Bearer ", "")

    let claims = oauth_validate_token(token, map {
        "issuer": "https://accounts.google.com",
        "audience": "my-api-client-id"
    })?

    // claims contains validated token data
    return json(map { "user_id": claims["sub"] })
}
```

---

### Original Plan (preserved for reference)
## Feature 10: Auth Module — `std/auth`

A complete authentication module supporting OAuth 2.0/OIDC federation, JWT tokens, and TOTP-based MFA. Designed for the common case: NTNT apps as OAuth **clients** (Login with Google/GitHub/Okta), not OAuth servers.

### Design Philosophy

1. **Simple by default** — One function call for standard OAuth flows
2. **Granular when needed** — Full control available for custom integrations
3. **OIDC-native** — Built on OpenID Connect with OAuth 2.0 fallback
4. **MFA as separate concern** — TOTP works with any auth system (local accounts or federated)

---

### High-Level API: `enable_oauth()` — The Simple Path

For 90% of apps, this is all you need. One function configures everything: providers, routes, sessions, redirects.

```ntnt
import { enable_oauth } from "std/auth"

enable_oauth(map {
    // ─── Configure providers (just add credentials) ────────────
    "google": map {
        "client_id": get_env("GOOGLE_CLIENT_ID"),
        "client_secret": get_env("GOOGLE_CLIENT_SECRET")
    },
    "github": map {
        "client_id": get_env("GITHUB_CLIENT_ID"),
        "client_secret": get_env("GITHUB_CLIENT_SECRET")
    },

    // ─── Your one callback: return the local user ──────────────
    "on_login": fn(provider, user_info, tokens) {
        // provider: "google", "github", etc.
        // user_info: { sub, email, name, picture, ... }
        // tokens: { access_token, refresh_token?, id_token? }
        return find_or_create_user(provider, user_info)
    },

    // ─── Where to go after login/logout ────────────────────────
    "after_login": "/dashboard",
    "after_logout": "/"
})

// That's it! Routes are auto-registered:
// GET  /auth/google          → redirect to Google
// GET  /auth/google/callback → handle callback, create session
// GET  /auth/github          → redirect to GitHub
// GET  /auth/github/callback → handle callback, create session
// POST /auth/logout          → clear session
// GET  /auth/me              → current user as JSON (for SPAs)

listen(8080)
```

**Full configuration options:**

```ntnt
enable_oauth(map {
    // ─── Built-in providers ────────────────────────────────────
    "google": map {
        "client_id": get_env("GOOGLE_CLIENT_ID"),
        "client_secret": get_env("GOOGLE_CLIENT_SECRET"),
        "scopes": ["openid", "email", "profile", "https://www.googleapis.com/auth/drive.readonly"]
    },

    "github": map {
        "client_id": get_env("GITHUB_CLIENT_ID"),
        "client_secret": get_env("GITHUB_CLIENT_SECRET"),
        "scopes": ["read:user", "user:email", "repo"]
    },

    "microsoft": map {
        "client_id": get_env("MICROSOFT_CLIENT_ID"),
        "client_secret": get_env("MICROSOFT_CLIENT_SECRET"),
        "tenant": "common"  // or specific tenant ID
    },

    "apple": map {
        "client_id": get_env("APPLE_CLIENT_ID"),
        "team_id": get_env("APPLE_TEAM_ID"),
        "key_id": get_env("APPLE_KEY_ID"),
        "private_key": get_env("APPLE_PRIVATE_KEY")
    },

    // ─── Enterprise OIDC (Okta, Auth0, Keycloak, etc.) ─────────
    "okta": map {
        "type": "oidc",
        "issuer": "https://mycompany.okta.com",
        "client_id": get_env("OKTA_CLIENT_ID"),
        "client_secret": get_env("OKTA_CLIENT_SECRET"),
        "scopes": ["openid", "email", "profile", "groups"]
    },

    "auth0": map {
        "type": "oidc",
        "issuer": "https://mycompany.auth0.com",
        "client_id": get_env("AUTH0_CLIENT_ID"),
        "client_secret": get_env("AUTH0_CLIENT_SECRET")
    },

    // ─── Generic OAuth 2.0 (non-OIDC providers) ────────────────
    "custom": map {
        "type": "oauth2",
        "authorize_url": "https://provider.com/oauth/authorize",
        "token_url": "https://provider.com/oauth/token",
        "userinfo_url": "https://provider.com/api/user",
        "client_id": get_env("CUSTOM_CLIENT_ID"),
        "client_secret": get_env("CUSTOM_CLIENT_SECRET"),
        "scopes": ["user", "email"]
    },

    // ─── Global settings ───────────────────────────────────────
    "routes_prefix": "/auth",           // default: /auth
    "after_login": "/dashboard",        // redirect after successful login
    "after_logout": "/",                // redirect after logout
    "session_duration": 86400,          // session TTL in seconds (default: 24h)
    "cookie_name": "session",           // session cookie name
    "cookie_secure": true,              // HTTPS only (default: true in production)
    "cookie_same_site": "Lax",          // SameSite attribute

    // ─── Security constraints ──────────────────────────────────
    "allowed_domains": ["company.com", "subsidiary.com"],
    "require_verified_email": true,

    // ─── Callbacks ─────────────────────────────────────────────
    "on_login": fn(provider, user_info, tokens) {
        // Called after successful OAuth
        // user_info fields (OIDC): sub, email, email_verified, name, picture, groups?
        // tokens fields: access_token, refresh_token?, id_token?, expires_in

        // Return the local user to create session for
        // Return None to deny login
        let user = find_or_create_user(provider, user_info)

        // Optional: store tokens for API calls on user's behalf
        if tokens.refresh_token {
            update(db, "users", map { "id": user.id }, map {
                "oauth_refresh_token": tokens.refresh_token
            })?
        }

        return user
    },

    "on_logout": fn(user) {
        // Optional: cleanup (revoke tokens, audit log, etc.)
        log_info("User logged out", map { "user_id": user.id })
    },

    "on_error": fn(provider, error, error_description) {
        // Custom error handling (default: redirect to /auth/error?...)
        log_error("OAuth error", map { "provider": provider, "error": error })
        return redirect("/login?error={error}")
    },

    "on_link": fn(provider, user_info, existing_user) {
        // Called when email matches existing user from different provider
        // Return true to link accounts, false to deny, or handle custom
        return true  // auto-link by default
    }
})
```

**Built-in providers:**

| Provider | Protocol | What's Pre-configured |
|----------|----------|----------------------|
| `google` | OIDC | Issuer, discovery, standard scopes |
| `github` | OAuth 2.0 | Endpoints, user API, email API |
| `microsoft` | OIDC | Azure AD endpoints, tenant support |
| `apple` | OIDC | Apple's JWT client secret, special requirements |
| `gitlab` | OIDC | Works with gitlab.com or self-hosted |
| `discord` | OAuth 2.0 | Endpoints, user API |
| `slack` | OIDC | Workspace OAuth |
| `type: "oidc"` | OIDC | Any provider — just supply issuer URL |
| `type: "oauth2"` | OAuth 2.0 | Any provider — supply endpoints manually |

**Auto-registered routes:**

| Route | Method | Description |
|-------|--------|-------------|
| `/auth/{provider}` | GET | Initiate OAuth flow (redirect to provider) |
| `/auth/{provider}/callback` | GET | Handle OAuth callback |
| `/auth/logout` | POST | Clear session, redirect to `after_logout` |
| `/auth/me` | GET | Return current user as JSON (for SPAs) |
| `/auth/refresh` | POST | Refresh session (extend TTL) |

**What `enable_oauth` handles automatically:**
- ✅ PKCE (always enabled for security)
- ✅ State parameter (CSRF protection)
- ✅ Nonce validation (OIDC replay protection)
- ✅ Session creation in KV store
- ✅ Secure cookie settings
- ✅ Token refresh (background, automatic)
- ✅ Email domain validation
- ✅ Email verification check
- ✅ Account linking (same email, different providers)
- ✅ Error handling with sensible defaults

---

### Granular API — Full Control When You Need It

For custom flows, non-standard providers, or when `enable_oauth` doesn't fit your needs, use the granular API directly.

### OAuth/OIDC Client API (Granular)

```ntnt
import {
    oidc_provider,
    oauth_provider,
    auth_url,
    exchange_code,
    refresh_token,
    get_userinfo,
    verify_id_token
} from "std/auth"

// Configure an OIDC provider (auto-discovers endpoints from .well-known)
let google = oidc_provider(map {
    "issuer": "https://accounts.google.com",
    "client_id": get_env("GOOGLE_CLIENT_ID"),
    "client_secret": get_env("GOOGLE_CLIENT_SECRET"),
    "redirect_uri": "http://localhost:8080/auth/callback",
    "scopes": ["openid", "email", "profile"]
})?

// Or configure OAuth 2.0 manually (for providers without OIDC discovery)
let github = oauth_provider(map {
    "authorize_url": "https://github.com/login/oauth/authorize",
    "token_url": "https://github.com/login/oauth/access_token",
    "userinfo_url": "https://api.github.com/user",
    "client_id": get_env("GITHUB_CLIENT_ID"),
    "client_secret": get_env("GITHUB_CLIENT_SECRET"),
    "redirect_uri": "http://localhost:8080/auth/github/callback",
    "scopes": ["read:user", "user:email"]
})

// Step 1: Generate auth URL (with PKCE for public clients)
fn login_with_google(req: Request) -> Response {
    let url = auth_url(google, map { "pkce": true })?
    // Store state + code_verifier in session for CSRF protection
    set(cache, "auth_state:{url.state}", url.code_verifier, map { "ttl": 600 })?
    return redirect(url.url)
}

// Step 2: Handle callback, exchange code for tokens
fn google_callback(req: Request) -> Response {
    let code = req.query_params.code
    let state = req.query_params.state

    // Verify state and get code_verifier (CSRF protection)
    let code_verifier = get(cache, "auth_state:{state}")?
        otherwise return json(map { "error": "Invalid state" }, 400)
    del(cache, "auth_state:{state}")?

    // Exchange code for tokens
    let tokens = exchange_code(google, code, map { "code_verifier": code_verifier })?
    // tokens = { access_token, refresh_token?, id_token?, expires_in }

    // Verify and decode ID token (OIDC) — checks signature, issuer, audience, expiry
    let claims = verify_id_token(google, tokens.id_token)?
    // claims = { sub, email, email_verified, name, picture, ... }

    // Or fetch user info via API (OAuth 2.0 without OIDC)
    let user_info = get_userinfo(google, tokens.access_token)?

    // Find or create local user from federated identity
    let local_user = find_one(db, "users", map { "provider": "google", "provider_id": claims.sub })?
    let user = match local_user {
        Some(u) => u,
        None => create(db, "users", map {
            "email": claims.email,
            "name": claims.name,
            "provider": "google",
            "provider_id": claims.sub,
            "avatar_url": claims.picture
        })?
    }

    // Create session
    let session_id = uuid()
    set(cache, "session:{session_id}", str(user.id), map { "ttl": 86400 })?

    return redirect("/dashboard")
        |> with_cookie("session", session_id, map { "http_only": true, "path": "/" })
}

// Refresh tokens when access_token expires
fn refresh_access_token(stored_refresh_token) {
    let new_tokens = refresh_token(google, stored_refresh_token)?
    // Store new access_token (and possibly new refresh_token)
    return new_tokens
}
```

### Provider Configuration Options

**OIDC Provider** (`oidc_provider`) — Auto-discovers endpoints from `{issuer}/.well-known/openid-configuration`:

| Option | Type | Required | Description |
|--------|------|----------|-------------|
| `issuer` | String | Yes | OIDC issuer URL (e.g., `https://accounts.google.com`) |
| `client_id` | String | Yes | OAuth client ID |
| `client_secret` | String | No | OAuth client secret (not needed for PKCE-only flows) |
| `redirect_uri` | String | Yes | Callback URL registered with provider |
| `scopes` | Array\<String\> | No | Scopes to request (default: `["openid"]`) |

**OAuth Provider** (`oauth_provider`) — Manual configuration for non-OIDC providers:

| Option | Type | Required | Description |
|--------|------|----------|-------------|
| `authorize_url` | String | Yes | Authorization endpoint |
| `token_url` | String | Yes | Token endpoint |
| `userinfo_url` | String | No | User info endpoint (for fetching profile) |
| `client_id` | String | Yes | OAuth client ID |
| `client_secret` | String | Yes | OAuth client secret |
| `redirect_uri` | String | Yes | Callback URL |
| `scopes` | Array\<String\> | No | Scopes to request |

### Auth URL Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `pkce` | Bool | false | Use PKCE (recommended for all flows) |
| `state` | String | auto | Custom state parameter (auto-generated if not provided) |
| `nonce` | String | auto | Nonce for ID token validation (OIDC) |
| `prompt` | String | — | `"login"`, `"consent"`, `"select_account"` |
| `login_hint` | String | — | Pre-fill email in provider's login form |

### JWT Utilities

For apps that need to issue their own tokens (API authentication, microservices):

```ntnt
import { jwt_sign, jwt_verify, jwt_decode } from "std/auth"

// Sign a JWT (HS256 with shared secret)
let token = jwt_sign(
    map { "user_id": 123, "role": "admin" },
    get_env("JWT_SECRET"),
    map { "alg": "HS256", "exp": 3600 }  // expires in 1 hour
)?

// Sign with RS256 (asymmetric) for public verification
let token = jwt_sign(
    map { "user_id": 123 },
    get_env("JWT_PRIVATE_KEY"),
    map { "alg": "RS256", "exp": 3600, "kid": "key-2024" }
)?

// Verify and decode (checks signature + expiration + nbf)
let claims = jwt_verify(token, get_env("JWT_SECRET"))?
// claims = { user_id: 123, role: "admin", iat: 1706886000, exp: 1706889600 }

// Verify with public key
let claims = jwt_verify(token, get_env("JWT_PUBLIC_KEY"), map { "alg": "RS256" })?

// Verify with issuer/audience validation
let claims = jwt_verify(token, secret, map {
    "iss": "https://myapp.com",
    "aud": "myapp-api"
})?

// Decode without verification (for debugging, reading headers)
let parts = jwt_decode(token)?
// parts = { header: { alg, typ, kid? }, payload: { ... }, signature: "..." }
```

**JWT Sign Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `alg` | String | "HS256" | Algorithm: `HS256`, `HS384`, `HS512`, `RS256`, `RS384`, `RS512`, `ES256`, `ES384` |
| `exp` | Int | — | Expiration time in seconds from now |
| `nbf` | Int | — | Not-before time in seconds from now |
| `iss` | String | — | Issuer claim |
| `aud` | String | — | Audience claim |
| `sub` | String | — | Subject claim (overrides payload.sub) |
| `kid` | String | — | Key ID for key rotation |

**JWT Verify Options:**

| Option | Type | Description |
|--------|------|-------------|
| `alg` | String | Expected algorithm (security: always specify for RS*/ES*) |
| `iss` | String | Required issuer value |
| `aud` | String | Required audience value |
| `leeway` | Int | Clock skew tolerance in seconds (default: 60) |

### TOTP for MFA

Time-based One-Time Passwords (RFC 6238) — compatible with Google Authenticator, Authy, 1Password, etc.

```ntnt
import { totp_secret, totp_url, totp_verify } from "std/auth"

// Step 1: Generate secret and QR code URL when user enables MFA
fn enable_mfa(req: Request) -> Response {
    let user = require_auth(req) otherwise return unauthorized()

    // Generate cryptographically secure secret (base32 encoded)
    let secret = totp_secret()

    // Generate otpauth:// URL for authenticator apps
    let url = totp_url(secret, map {
        "issuer": "MyApp",
        "account": user.email
    })
    // url = "otpauth://totp/MyApp:alice@example.com?secret=JBSWY3DPEHPK3PXP&issuer=MyApp"

    // Store secret temporarily until user confirms with a valid code
    set(cache, "mfa_setup:{user.id}", secret, map { "ttl": 600 })?

    // Return URL for QR code generation (frontend renders QR)
    return json(map { "secret": secret, "otpauth_url": url })
}

// Step 2: Verify code and activate MFA
fn confirm_mfa(req: Request) -> Response {
    let user = require_auth(req) otherwise return unauthorized()
    let data = parse_json(req)?
    let code = data["code"]

    let secret = get(cache, "mfa_setup:{user.id}")?
        otherwise return json(map { "error": "MFA setup expired, please restart" }, 400)

    // Verify the code matches
    if totp_verify(secret, code) {
        // Store secret permanently (encrypt in production!)
        update(db, "users", map { "id": user.id }, map {
            "mfa_secret": secret,
            "mfa_enabled": true
        })?
        del(cache, "mfa_setup:{user.id}")?

        // Generate backup codes
        let backup_codes = generate_backup_codes(user.id)

        return json(map { "mfa_enabled": true, "backup_codes": backup_codes })
    } else {
        return json(map { "error": "Invalid code, please try again" }, 400)
    }
}

// Step 3: Verify MFA during login
fn verify_mfa(req: Request) -> Response {
    let data = parse_json(req)?
    let pending_user_id = data["pending_user_id"]
    let code = data["code"]

    let user = find_one(db, "users", map { "id": pending_user_id })?
        otherwise return json(map { "error": "User not found" }, 404)

    // Verify TOTP code (window: 1 allows ±30 seconds for clock drift)
    if totp_verify(user.mfa_secret, code, map { "window": 1 }) {
        // MFA passed — create session
        let session_id = uuid()
        set(cache, "session:{session_id}", str(user.id), map { "ttl": 86400 })?

        return json(map { "ok": true })
            |> with_cookie("session", session_id, map { "http_only": true, "path": "/" })
    } else {
        // Check backup codes as fallback
        if verify_backup_code(user.id, code) {
            let session_id = uuid()
            set(cache, "session:{session_id}", str(user.id), map { "ttl": 86400 })?
            return json(map { "ok": true, "backup_code_used": true })
                |> with_cookie("session", session_id, map { "http_only": true, "path": "/" })
        }

        return json(map { "error": "Invalid code" }, 401)
    }
}
```

**TOTP Options:**

| Function | Option | Type | Default | Description |
|----------|--------|------|---------|-------------|
| `totp_url` | `issuer` | String | Required | App name shown in authenticator |
| `totp_url` | `account` | String | Required | User identifier (usually email) |
| `totp_url` | `digits` | Int | 6 | Code length (6 or 8) |
| `totp_url` | `period` | Int | 30 | Code rotation period in seconds |
| `totp_verify` | `window` | Int | 0 | Number of periods before/after to accept (for clock drift) |

### Complete Auth Flow Example

Login with local account + MFA, or federated identity:

```ntnt
// Login: check password, then require MFA if enabled
fn login(req: Request) -> Response {
    let form = parse_form(req)
    let email = form["email"]
    let password = form["password"]

    let user = find_one(db, "users", map { "email": email })?
        otherwise return json(map { "error": "Invalid credentials" }, 401)

    // Local account — verify password
    if user.password_hash != None {
        let valid = verify_password(password, user.password_hash)?
        if !valid {
            return json(map { "error": "Invalid credentials" }, 401)
        }
    }

    // Check if MFA is enabled
    if user.mfa_enabled {
        // Return pending state — frontend must collect MFA code
        let pending_token = uuid()
        set(cache, "mfa_pending:{pending_token}", str(user.id), map { "ttl": 300 })?
        return json(map { "mfa_required": true, "pending_token": pending_token })
    }

    // No MFA — create session directly
    let session_id = uuid()
    set(cache, "session:{session_id}", str(user.id), map { "ttl": 86400 })?

    return json(map { "ok": true })
        |> with_cookie("session", session_id, map { "http_only": true, "path": "/" })
}
```

### Function Summary

| Function | Signature | Description |
|----------|-----------|-------------|
| `oidc_provider` | `(config: Map) -> Result<Provider, String>` | Configure OIDC provider with auto-discovery |
| `oauth_provider` | `(config: Map) -> Provider` | Configure OAuth 2.0 provider manually |
| `auth_url` | `(provider, opts?) -> Result<AuthUrl, String>` | Generate authorization URL with state/PKCE |
| `exchange_code` | `(provider, code, opts?) -> Result<Tokens, String>` | Exchange auth code for tokens |
| `refresh_token` | `(provider, refresh_token) -> Result<Tokens, String>` | Refresh access token |
| `get_userinfo` | `(provider, access_token) -> Result<Map, String>` | Fetch user info from provider |
| `verify_id_token` | `(provider, id_token) -> Result<Claims, String>` | Verify OIDC ID token (signature + claims) |
| `jwt_sign` | `(payload, secret, opts?) -> Result<String, String>` | Create a signed JWT |
| `jwt_verify` | `(token, secret, opts?) -> Result<Claims, String>` | Verify JWT signature and claims |
| `jwt_decode` | `(token) -> Result<JwtParts, String>` | Decode JWT without verification |
| `totp_secret` | `() -> String` | Generate random TOTP secret (base32) |
| `totp_url` | `(secret, opts) -> String` | Generate otpauth:// URL for authenticator apps |
| `totp_verify` | `(secret, code, opts?) -> Bool` | Verify TOTP code |
| `bearer_token` | `(req, secret, opts?) -> Result<Claims, String>` | Extract and verify JWT from Authorization header |
| `enable_oauth` | `(config: Map) -> Unit` | **High-level:** Configure OAuth with auto routes/sessions |

### Bearer Token Helper

The most common pattern for SPA authentication — extract JWT from `Authorization: Bearer <token>` header and verify it:

```ntnt
import { bearer_token } from "std/auth"

fn api_handler(req: Request) -> Response {
    let claims = bearer_token(req, get_env("JWT_SECRET")) otherwise {
        return json(map { "error": "Unauthorized: {err}" }, 401)
    }

    // claims.user_id, claims.roles, etc. are now available
    let user_id = claims["user_id"]
    // ...
}

// With options (algorithm validation, issuer/audience checks)
fn strict_api_handler(req: Request) -> Response {
    let claims = bearer_token(req, get_env("JWT_PUBLIC_KEY"), map {
        "alg": "RS256",
        "iss": "https://myapp.com",
        "aud": "myapp-api"
    }) otherwise {
        return json(map { "error": "Unauthorized" }, 401)
    }
    // ...
}
```

`bearer_token` is equivalent to:
```ntnt
let auth_header = req.headers["authorization"] otherwise return Err("Missing Authorization header")
let token = replace(auth_header, "Bearer ", "")
let claims = jwt_verify(token, secret, opts)?
```

### What About SAML?

SAML 2.0 is intentionally **not included** in v1:

1. **OIDC covers most enterprise needs** — Okta, Azure AD, Google Workspace, Auth0, Keycloak all support OIDC. Most enterprises are migrating from SAML to OIDC.

2. **SAML complexity is high** — XML parsing, certificate management, signature verification, assertion decryption. The API surface would be 3x larger than OIDC.

3. **Agent-unfriendly** — SAML metadata files and certificate rotation are exactly the kind of "hidden state" that confuses agents.

If SAML becomes a hard requirement, it can be added as `std/auth/saml` in a future version without affecting the OIDC/JWT/TOTP APIs.

### Implementation

- New file `src/stdlib/auth.rs`
- Register as `"std/auth"` in `src/stdlib/mod.rs`
- Add to `Cargo.toml`:
  - `jsonwebtoken = "9"` — JWT signing/verification
  - `totp-rs = { version = "5", features = ["gen_secret", "otpauth"] }` — TOTP
- OIDC discovery: HTTP GET to `{issuer}/.well-known/openid-configuration`, parse JSON for endpoint URLs
- OAuth flows: use `reqwest` (already a dependency) for HTTP requests to token/userinfo endpoints
- PKCE: generate `code_verifier` (43-128 char random string), derive `code_challenge` via SHA256
- ID token verification: decode JWT, verify signature against provider's JWKS (fetched from discovery), check `iss`, `aud`, `exp`, `nonce`

### Tests

- OIDC discovery parsing (mock `.well-known` response)
- Auth URL generation with state and PKCE
- Code exchange (mock token endpoint)
- ID token verification (test vectors with known keys)
- JWT sign/verify roundtrip for HS256, RS256
- JWT expiration and claim validation
- TOTP generation and verification (test vectors from RFC 6238)
- TOTP window tolerance

---

## Feature 11: Roadmap + Docs Update

- **ROADMAP.md** — add sections:
  - `### 7.17 Web Application Essentials` (features 1-5)
  - `### 7.18 Database Layer` (features 6-7)
  - `### 7.19 KV Store` (feature 8)
  - `### 7.20 Declarative Route Blocks` (feature 9)
  - `### 7.21 Auth Module` (feature 10)
- **docs/AI_AGENT_GUIDE.md** — update Common Imports, add cookie/logging/CORS/upload/query builder/KV/server block/auth patterns
- Run `ntnt docs --generate` to regenerate STDLIB_REFERENCE.md and sync agent files

---

## Complete Function Summary

### 57 stdlib functions + 1 global builtin + 7 CLI commands + 1 new syntax form

| Module | Function | Pipe-friendly? |
|--------|----------|---------------|
| `std/crypto` | `hash_password(password, cost?)` | — |
| `std/crypto` | `verify_password(password, hash)` | — |
| `std/crypto` | `is_valid_hash(hash)` | — |
| `std/http/server` | `cookie(req, name)` | — |
| `std/http/server` | `cookies(req)` | — |
| `std/http/server` | `with_cookie(resp, name, value, opts?)` | Yes |
| `std/http/server` | `without_cookie(resp, name, opts?)` | Yes |
| `std/http/server` | `parse_upload(req)` | — |
| `std/http/server` | `save_upload(file, path)` | — |
| `std/log` | `log_info(message, data?)` | — |
| `std/log` | `log_warn(message, data?)` | — |
| `std/log` | `log_error(message, data?)` | — |
| `std/log` | `log_debug(message, data?)` | — |
| `std/log` | `set_log_level(level)` | — |
| `std/log` | `configure_logging(config)` | — |
| `std/log` | `request_logger()` | — |
| `std/db` | `find(db, table, where?, options?)` | — |
| `std/db` | `find_one(db, table, where?)` | — |
| `std/db` | `create(db, table, data)` | — |
| `std/db` | `create_many(db, table, rows)` | — |
| `std/db` | `update(db, table, where, data)` | — |
| `std/db` | `update_returning(db, table, where, data)` | — |
| `std/db` | `delete(db, table, where)` | — |
| `std/db` | `delete_returning(db, table, where)` | — |
| `std/db` | `upsert(db, table, conflict_keys, data)` | — |
| `std/db` | `count(db, table, where?)` | — |
| `std/db` | `exists(db, table, where)` | — |
| `std/db` | `aggregate(db, table, options)` | — |
| `std/db` | `increment(db, table, where, column, amount?)` | — |
| `std/db` | `decrement(db, table, where, column, amount?)` | — |
| `std/db` | `transaction(db, fn)` | — |
| `std/db/schema` | `define_table(name, columns)` | — |
| `std/db/schema` | `sync_schema(db)` | — |
| `std/kv` | `open(url)` | — |
| `std/kv` | `get(kv, key)` | — |
| `std/kv` | `set(kv, key, value, opts?)` | — |
| `std/kv` | `del(kv, key)` | — |
| `std/kv` | `has(kv, key)` | — |
| `std/kv` | `list(kv, prefix?)` | — |
| `std/kv` | `expire(kv, key, seconds)` | — |
| `std/kv` | `ttl(kv, key)` | — |
| `std/kv` | `flush(kv)` | — |
| `std/auth` | `oidc_provider(config)` | — |
| `std/auth` | `oauth_provider(config)` | — |
| `std/auth` | `auth_url(provider, opts?)` | — |
| `std/auth` | `exchange_code(provider, code, opts?)` | — |
| `std/auth` | `refresh_token(provider, token)` | — |
| `std/auth` | `get_userinfo(provider, access_token)` | — |
| `std/auth` | `verify_id_token(provider, id_token)` | — |
| `std/auth` | `jwt_sign(payload, secret, opts?)` | — |
| `std/auth` | `jwt_verify(token, secret, opts?)` | — |
| `std/auth` | `jwt_decode(token)` | — |
| `std/auth` | `totp_secret()` | — |
| `std/auth` | `totp_url(secret, opts)` | — |
| `std/auth` | `totp_verify(secret, code, opts?)` | — |
| `std/auth` | `bearer_token(req, secret, opts?)` | — |
| `std/auth` | `enable_oauth(config)` | — |
| global builtin | `enable_cors(options?)` | — |

| CLI Command | Description |
|-------------|-------------|
| `ntnt db diff` | Show schema changes (read-only) |
| `ntnt db diff --save` | Generate migration SQL file |
| `ntnt db sync` | Apply schema changes (development) |
| `ntnt db apply` | Apply pending migration files (production) |
| `ntnt db check` | Verify schema matches database (CI gate) |
| `ntnt db reset` | Drop all and recreate from schema |
| `ntnt db seed` | Run seed files to populate data |

| Syntax | Description |
|--------|-------------|
| `server PORT { ... }` | Declarative route block with type-safe params |

---

## Files Modified

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `bcrypt = "0.15"`, `redis = { version = "0.25", features = ["tokio-comp"] }`, `jsonwebtoken = "9"`, `totp-rs = { version = "5", features = ["gen_secret", "otpauth"] }`, `tracing-appender = "0.2"` |
| `src/stdlib/crypto.rs` | Add `hash_password`, `verify_password`, `is_valid_hash` |
| `src/stdlib/http_server.rs` | Array header support, cookie functions (4), upload functions (2), `CorsConfig` |
| `src/stdlib/http_bridge.rs` | `BridgeResponse.headers` -> `Vec<(String, String)>` |
| `src/stdlib/http_server_async.rs` | Update `bridge_to_axum_response` for Vec headers |
| `src/stdlib/log.rs` | **New** — `std/log` module (7 functions) with file output, rotation, JSON format |
| `src/stdlib/db.rs` | **New** — `std/db` query builder (15 functions) |
| `src/stdlib/db_schema.rs` | **New** — `std/db/schema` sync (2 functions) |
| `src/stdlib/kv.rs` | **New** — `std/kv` key-value store (9 functions) |
| `src/stdlib/auth.rs` | **New** — `std/auth` OAuth/OIDC/JWT/TOTP (15 functions including `enable_oauth`) |
| `src/stdlib/mod.rs` | Register `log`, `db`, `db/schema`, `kv`, `auth` modules |
| `src/parser.rs` | `server` block syntax, typed route params, route groups |
| `src/ast.rs` | `Statement::ServerBlock` AST node |
| `src/interpreter.rs` | `enable_cors` builtin, CORS in both server loops, `eval_server_block` desugaring, typed param coercion |
| `src/main.rs` | `ntnt db diff|sync|apply|check|reset|seed` CLI commands |
| `ROADMAP.md` | Sections 7.17-7.21 |
| `docs/AI_AGENT_GUIDE.md` | New patterns and imports |

---

## How It All Fits Together — Bookmark Manager

This example uses every feature from this plan: password hashing, cookies, logging, CORS, file uploads, the query builder with JOINs, schema sync, KV caching, OAuth/OIDC login, and the declarative server block.

```ntnt
// bookmarks.tnt — Complete bookmark manager

import { json, html, redirect, parse_form, parse_json, cookie, with_cookie, without_cookie, parse_upload, save_upload } from "std/http/server"
import { hash_password, verify_password, uuid } from "std/crypto"
import { connect } from "std/db/sqlite"
import { find, find_one, create, update, delete, count, aggregate } from "std/db"
import { define_table, sync_schema } from "std/db/schema"
import { open, get, set, del } from "std/kv"
import { log_info, log_warn, log_error, request_logger, set_log_level, configure_logging } from "std/log"
import { enable_oauth } from "std/auth"
import { get_env } from "std/env"

// ─── Schema (single source of truth) ─────────────────────────────

define_table("users", map {
    "email": "text required unique",
    "password_hash": "text",  // nullable for OAuth-only users
    "name": "text",
    "avatar_path": "text",
    "provider": "text",       // "local", "google", "github"
    "provider_id": "text",    // external user ID from OAuth provider
    "created_at": "timestamp default:now"
})

define_table("bookmarks", map {
    "url": "text required",
    "title": "text",
    "description": "text",
    "user_id": "integer references:users",
    "public": "boolean default:false",
    "views": "integer default:0",
    "created_at": "timestamp default:now"
})

define_table("tags", map {
    "name": "text required unique"
})

define_table("bookmark_tags", map {
    "bookmark_id": "integer references:bookmarks",
    "tag_id": "integer references:tags"
})

// ─── Database + Cache ─────────────────────────────────────────────

let db = connect("bookmarks.db")?
sync_schema(db)

let cache = open("cache.db")?

set_log_level(get_env("LOG_LEVEL") otherwise "info")

// ─── OAuth (batteries-included) ──────────────────────────────────

enable_oauth(map {
    "google": map {
        "client_id": get_env("GOOGLE_CLIENT_ID"),
        "client_secret": get_env("GOOGLE_CLIENT_SECRET")
    },
    "on_login": fn(provider, user_info, tokens) {
        // Find or create user from federated identity
        let existing = find_one(db, "users", map {
            "provider": provider,
            "provider_id": user_info.sub
        })?
        return match existing {
            Some(u) => u,
            None => create(db, "users", map {
                "email": user_info.email,
                "name": user_info.name,
                "provider": provider,
                "provider_id": user_info.sub,
                "avatar_path": user_info.picture
            })?
        }
    },
    "on_session": fn(user) {
        // Called after on_login — store session in our KV cache
        let session_id = uuid()
        set(cache, "session:{session_id}", str(user.id), map { "ttl": 86400 })?
        log_info("OAuth login", map { "provider": user.provider, "user_id": user.id })
        return session_id
    },
    "session_cookie": "session",
    "after_login": "/dashboard",
    "after_logout": "/"
})
// Auto-registers: GET /auth/google, GET /auth/google/callback

// ─── Cache helpers ────────────────────────────────────────────────

fn cached_user(user_id) {
    let key = "user:{user_id}"
    let hit = get(cache, key)?
    if hit != None { return Ok(hit) }

    let user = find_one(db, "users", map { "id": user_id })?
    match user {
        Some(u) => {
            set(cache, key, u, map { "ttl": 300 })?
            return Ok(Some(u))
        },
        None => return Ok(None)
    }
}

fn cached_bookmarks(user_id) {
    let key = "bookmarks:{user_id}"
    let hit = get(cache, key)?
    if hit != None { return Ok(hit) }

    let bookmarks = find(db, "bookmarks",
        map { "user_id": user_id },
        map {
            "include": map {
                "tags": map { "through": "bookmark_tags" }
            },
            "order": "created_at desc",
            "limit": 50
        }
    )?

    set(cache, key, bookmarks, map { "ttl": 60 })?
    return Ok(bookmarks)
}

fn bust_bookmark_cache(user_id) {
    del(cache, "bookmarks:{user_id}")?
}

// ─── Auth helpers ─────────────────────────────────────────────────

fn require_auth(req) {
    let session_id = cookie(req, "session") otherwise return None
    let user_id_str = get(cache, "session:{session_id}")? otherwise return None
    let user = cached_user(int(user_id_str))? otherwise return None
    return user
}

// ─── Handlers ─────────────────────────────────────────────────────

fn home(req: Request) -> Response {
    let stats = aggregate(db, "bookmarks", map {
        "where": map { "public": true },
        "count": "*"
    })?
    return html(template("views/home.html", map {
        "bookmark_count": stats[0].count
    }))
}

fn register(req: Request) -> Response {
    let form = parse_form(req)
    let hash = hash_password(form["password"]) otherwise {
        return json(map { "error": "Hash failed: {err}" }, 500)
    }

    let user = create(db, "users", map {
        "email": form["email"],
        "name": form["name"],
        "password_hash": hash
    }) otherwise {
        return json(map { "error": "Email already registered" }, 409)
    }

    log_info("User registered", map { "email": form["email"] })

    let session_id = uuid()
    set(cache, "session:{session_id}", str(user.id), map { "ttl": 86400 })?

    return json(map { "user": map { "id": user.id, "email": user.email } }, 201)
        |> with_cookie("session", session_id, map {
            "http_only": true,
            "same_site": "Lax",
            "max_age": 86400,
            "path": "/"
        })
}

fn login(req: Request) -> Response {
    let form = parse_form(req)

    let user = find_one(db, "users", map { "email": form["email"] })?
    match user {
        None => return json(map { "error": "Invalid credentials" }, 401),
        Some(u) => {
            let valid = verify_password(form["password"], u.password_hash)?
            if !valid {
                log_warn("Failed login", map { "email": form["email"] })
                return json(map { "error": "Invalid credentials" }, 401)
            }

            let session_id = uuid()
            set(cache, "session:{session_id}", str(u.id), map { "ttl": 86400 })?
            log_info("User logged in", map { "user_id": u.id })

            return json(map { "ok": true })
                |> with_cookie("session", session_id, map {
                    "http_only": true,
                    "same_site": "Lax",
                    "max_age": 86400,
                    "path": "/"
                })
        }
    }
}

fn logout(req: Request) -> Response {
    let session_id = cookie(req, "session")
    match session_id {
        Some(sid) => del(cache, "session:{sid}")?,
        None => {}
    }
    return json(map { "ok": true })
        |> without_cookie("session", map { "path": "/" })
}

fn list_bookmarks(req: Request) -> Response {
    let user = require_auth(req)
        otherwise return json(map { "error": "Not logged in" }, 401)

    let bookmarks = cached_bookmarks(user.id)?
    return json(map { "bookmarks": bookmarks })
}

fn create_bookmark(req: Request) -> Response {
    let user = require_auth(req)
        otherwise return json(map { "error": "Not logged in" }, 401)

    let data = parse_json(req) otherwise {
        return json(map { "error": "Invalid JSON: {err}" }, 400)
    }

    let bookmark = create(db, "bookmarks", map {
        "url": data["url"],
        "title": data["title"],
        "description": data["description"],
        "user_id": user.id
    })?

    bust_bookmark_cache(user.id)?
    log_info("Bookmark created", map { "url": data["url"], "user": user.id })

    return json(bookmark, 201)
}

fn delete_bookmark(req: Request) -> Response {
    let user = require_auth(req)
        otherwise return json(map { "error": "Not logged in" }, 401)

    let id = int(req.params.id)
    let bookmark = find_one(db, "bookmarks", map { "id": id, "user_id": user.id })?
    match bookmark {
        None => return json(map { "error": "Not found" }, 404),
        Some(_) => {
            delete(db, "bookmark_tags", map { "bookmark_id": id })?
            delete(db, "bookmarks", map { "id": id })?
            bust_bookmark_cache(user.id)?
            return json(map { "deleted": true })
        }
    }
}

fn upload_avatar(req: Request) -> Response {
    let user = require_auth(req)
        otherwise return json(map { "error": "Not logged in" }, 401)

    let fields = parse_upload(req) otherwise {
        return json(map { "error": "Invalid upload: {err}" }, 400)
    }

    let avatar = fields["avatar"]
    save_upload(avatar, "uploads/avatars/") otherwise {
        return json(map { "error": "Save failed: {err}" }, 500)
    }

    update(db, "users", map { "id": user.id }, map {
        "avatar_path": "uploads/avatars/{avatar.filename}"
    })?
    del(cache, "user:{user.id}")?

    return json(map { "avatar": avatar.filename })
}

fn bookmark_stats(req: Request) -> Response {
    let user = require_auth(req)
        otherwise return json(map { "error": "Not logged in" }, 401)

    let stats = aggregate(db, "bookmarks", map {
        "where": map { "user_id": user.id },
        "group_by": "public",
        "count": "*"
    })?

    let total = count(db, "bookmarks", map { "user_id": user.id })?

    return json(map { "total": total, "by_visibility": stats })
}

// ─── Server ───────────────────────────────────────────────────────

server 8080 {
    static "/assets" from "./public"
    static "/uploads" from "./uploads"

    cors map {
        "origins": [get_env("CORS_ORIGIN") otherwise "*"],
        "credentials": true
    }

    middleware [request_logger()]

    GET  /               -> home
    POST /auth/register  -> register
    POST /auth/login     -> login
    POST /auth/logout    -> logout
    // OAuth routes auto-registered by enable_oauth()

    GET    /bookmarks       -> list_bookmarks
    POST   /bookmarks       -> create_bookmark
    DELETE /bookmarks/{id: Int} -> delete_bookmark

    POST /profile/avatar -> upload_avatar
    GET  /stats          -> bookmark_stats

    on_shutdown fn() {
        log_info("Server shutting down")
    }
}
```

### What This Example Demonstrates

| Feature | Where Used |
|---------|-----------|
| **Password hashing** | `register` handler — `hash_password`, `verify_password` |
| **Cookies** | Session management — `with_cookie`, `without_cookie`, `cookie` |
| **Logging** | Throughout — `log_info`, `log_warn`, `request_logger()` middleware |
| **CORS** | `cors` directive in server block |
| **File uploads** | `upload_avatar` — `parse_upload`, `save_upload` |
| **Query builder** | All CRUD operations — `find`, `find_one`, `create`, `update`, `delete`, `count` |
| **JOINs** | `cached_bookmarks` — includes tags through join table |
| **Aggregations** | `bookmark_stats` — GROUP BY with COUNT |
| **Schema sync** | Top of file — `define_table` + `sync_schema` |
| **KV caching** | `cached_user`, `cached_bookmarks`, session storage, cache busting on writes |
| **OAuth/OIDC** | `enable_oauth()` — one call configures Google login with auto-registered routes |
| **Declarative routes** | `server 8080 { ... }` block with typed params, middleware, static files, CORS |

---

## Documentation Requirements (Mandatory)

Every function added MUST follow the NTNT documentation system. The build will fail if documentation is missing or incomplete.

### Per-Function Checklist

For each new function, add a `// @ntnt` doc block directly above the `module.insert()` call:

```rust
// @ntnt hash_password
// @module std/crypto
// @signature hash_password(password: String, cost?: Int) -> Result<String, String>
// Hash a password using bcrypt with configurable cost factor.
//
// Returns a bcrypt hash string that can be stored in the database.
// The hash includes the salt, so no separate salt storage is needed.
// @param password The plaintext password to hash
// @param cost Work factor (4-31). Default 12. Higher = slower but more secure.
// @returns Ok(hash_string) on success, Err(message) on failure
// @example hash_password("secret123") => Ok("$2b$12$...") ~ "Hash with default cost"
// @example hash_password("secret123", 14) => Ok("$2b$14$...") ~ "Hash with higher cost"
// @error InvalidCost ~ "Cost must be between 4 and 31" fix: "Use a cost value in valid range"
// @see_also verify_password, is_valid_hash
// @since v0.4.0
module.insert(
    "hash_password".to_string(),
    Value::NativeFunction { /* ... */ },
);
```

**Required directives:**
- `// @ntnt <name>` — must match function name
- `// @module <path>` — e.g., `std/crypto`, `std/db`
- `// @signature <sig>` — full typed signature
- Summary line(s) — first non-`@` lines
- `// @example` — at least one working example

**Build enforcement:** `build.rs` scans all stdlib files and fails if:
- A `module.insert()` lacks a `// @ntnt` block
- A `// @ntnt` block lacks a matching `module.insert()`
- Required directives are missing

### After Implementation

1. **Build and verify:**
   ```bash
   cargo build --profile dev-release  # Fails if docs incomplete
   ```

2. **Test in REPL:**
   ```
   ntnt> :doc hash_password     # Verify docs render correctly
   ntnt> :search "password"     # Verify searchable
   ```

3. **Regenerate reference docs:**
   ```bash
   ntnt docs --generate         # Updates STDLIB_REFERENCE.md
   ntnt docs --validate         # Verify 100% coverage
   ```

4. **Update agent guides:**
   - Add new imports to `docs/AI_AGENT_GUIDE.md` Common Imports table
   - Add usage patterns to relevant sections
   - These sync to CLAUDE.md automatically via `ntnt docs --generate`

### Testing Against Examples

Before marking a feature complete:

1. **Verify examples from the plan work:**
   - Copy each code example from the plan into a `.tnt` file
   - Run `ntnt lint` — must pass
   - Run `ntnt run` — must produce expected output

2. **Test the bookmark manager example:**
   - After all features, the complete example must run end-to-end
   - Each feature should be testable incrementally as it's added

---

## What's NOT Included (Intentional Omissions)

These are explicitly out of scope for v1:

| Omission | Rationale |
|----------|-----------|
| **SAML 2.0** | OIDC covers 95% of enterprise SSO. SAML adds 3x complexity (XML, certs, metadata). Can add later as `std/auth/saml`. |
| **Chunked/resumable uploads** | Simple multipart covers 90% of cases. Large files should use S3 presigned URLs. |
| **WebSockets** | Separate feature requiring different server architecture. Planned for future. |
| **GraphQL** | REST + JSON covers most needs. GraphQL can be a separate `std/graphql` module. |
| **Email sending** | Many options (SMTP, SendGrid, SES). Better as `std/email` in future. |
| **Background jobs** | Requires job queue infrastructure. Better as `std/jobs` with Redis/Valkey backend. |
| **Full-text search** | Database-specific (Postgres has it built-in, SQLite needs FTS5). Document as raw SQL. |
| **Rate limiting** | Can be built with KV store + middleware. May add `rate_limit()` helper later. |
| **CSRF tokens** | OAuth uses state param. Traditional forms can use double-submit cookie pattern with `with_cookie`. |

---

## Verification

### Per-Feature Checklist

For each feature, verify:

- [ ] `cargo build --profile dev-release` succeeds (build.rs enforces doc coverage)
- [ ] `cargo test` — all existing + new tests pass
- [ ] `// @ntnt` doc blocks complete with `@signature`, `@param`, `@example`
- [ ] Unit tests cover happy path, error cases, edge cases
- [ ] Integration test with real usage pattern

### Final Acceptance Criteria

After all features implemented:

1. **Documentation**
   - [ ] `ntnt docs --generate` regenerates STDLIB_REFERENCE.md
   - [ ] `ntnt docs --validate` shows 100% coverage
   - [ ] AI_AGENT_GUIDE.md updated with new imports and patterns
   - [ ] ROADMAP.md sections 7.17-7.21 added and checked off

2. **Bookmark Manager App**
   - [ ] `ntnt lint bookmarks.tnt` passes
   - [ ] `ntnt db sync` creates all tables from schema
   - [ ] `ntnt db seed seeds/` populates reference data
   - [ ] Local registration + login works
   - [ ] OAuth login with Google works
   - [ ] CRUD operations on bookmarks work
   - [ ] File upload for avatar works
   - [ ] Caching layer reduces DB queries
   - [ ] Logs appear in correct format
   - [ ] CORS headers present on responses

3. **Comparison Test**
   - [ ] Build equivalent app in Go (net/http + sqlx)
   - [ ] Build equivalent app in Python (Flask + SQLAlchemy)
   - [ ] NTNT version has fewer lines of code
   - [ ] NTNT version has fewer files
   - [ ] NTNT version has no "you just need to know" gotchas

4. **Agent Test**
   - [ ] Give Claude/GPT-4 the updated AI_AGENT_GUIDE.md
   - [ ] Ask it to build a simple CRUD app with auth
   - [ ] Verify it produces working code on first attempt
   - [ ] No "undefined function" or "wrong import" errors

---

## Quick Reference Card

### Imports Cheat Sheet

```ntnt
// Auth & Security
import { hash_password, verify_password, is_valid_hash, uuid } from "std/crypto"
import { enable_oauth, jwt_sign, jwt_verify, bearer_token } from "std/auth"

// HTTP & Cookies
import { json, html, redirect, parse_form, parse_json, parse_upload, save_upload } from "std/http/server"
import { cookie, cookies, with_cookie, without_cookie } from "std/http/server"

// Database
import { connect } from "std/db/sqlite"        // or "std/db/postgres"
import { find, find_one, create, update, delete, upsert, count, exists, transaction } from "std/db"
import { increment, decrement, aggregate, create_many, update_returning, delete_returning } from "std/db"
import { define_table, sync_schema } from "std/db/schema"

// KV Store
import { open, get, set, del, has, list, expire, ttl, flush } from "std/kv"

// Logging
import { log_info, log_warn, log_error, log_debug, set_log_level, configure_logging, request_logger } from "std/log"

// Utilities
import { get_env, load_env } from "std/env"
```

### Common Patterns

```ntnt
// 1. Authenticated handler
fn my_handler(req: Request) -> Response {
    let user = require_auth(req) otherwise return json(map { "error": "Unauthorized" }, 401)
    // ... use user
}

// 2. Parse JSON with error handling
let data = parse_json(req) otherwise {
    return json(map { "error": "Invalid JSON: {err}" }, 400)
}

// 3. Database transaction
let result = transaction(db, fn() {
    let user = create(db, "users", map { "email": email })?
    create(db, "profiles", map { "user_id": user.id })?
    return Ok(user)
})?

// 4. Cache-aside pattern
fn get_user(db, cache, id) {
    let cached = get(cache, "user:{id}")?
    if cached != None { return Ok(cached) }
    let user = find_one(db, "users", map { "id": id })?
    if user != None { set(cache, "user:{id}", user, map { "ttl": 300 })? }
    return Ok(user)
}

// 5. Response with cookie
return json(map { "ok": true })
    |> with_cookie("session", token, map { "http_only": true, "path": "/" })
```

### CLI Commands

```bash
# Development
ntnt lint app.tnt                    # Check for errors
ntnt run app.tnt                     # Run with hot-reload
ntnt db sync schema.tnt --db app.db  # Apply schema changes

# Production
ntnt db diff schema.tnt --db prod.db --save migrations/001.sql
ntnt db apply migrations/ --db prod.db
ntnt db check schema.tnt --db prod.db  # CI gate

# Utilities
ntnt db seed seeds/ --db app.db      # Populate data
ntnt db reset schema.tnt --db app.db # Drop and recreate
ntnt docs --generate                 # Regenerate docs
```
