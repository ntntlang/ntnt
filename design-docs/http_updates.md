# NTNT HTTP Library Design

> **Philosophy**: Simple things should be simple. Complex things should be possible.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        nginx                            │
│  TLS · Compression · Rate Limits · Static Files · Logs  │
└─────────────────────────┬───────────────────────────────┘
                          │ HTTP/1.1
                          ▼
┌─────────────────────────────────────────────────────────┐
│                    NTNT HTTP Server                     │
│         Routes · Handlers · Sessions · WebSocket        │
└─────────────────────────────────────────────────────────┘
```

nginx handles infrastructure. NTNT handles application logic.

---

## Current Issues

### HTTP Client (`std/http`)

1. **Too many functions** — 14 exported functions with overlapping purposes
2. **Duplicated code** — Each HTTP method repeats the same response handling
3. **No caching** — Every request hits the network

### HTTP Server (`std/http/server`)

1. **Magic built-ins** — `get`, `post`, `listen` are interpreter magic, not regular functions
2. **No session support** — Can't maintain user state
3. **No graceful shutdown** — Drops connections on restart

---

## Redesigned HTTP Client

### Principle: One Function, Smart Defaults

```ntnt
import { fetch } from "std/http"

// Simple GET
let response = fetch("https://api.example.com/users")

// With options
let response = fetch("https://api.example.com/users", map {
    "method": "POST",
    "json": map { "name": "Alice" },
    "headers": map { "Authorization": "Bearer " + token }
})
```

**That's it.** One function. Everything else is options.

### Response Object

```ntnt
match response {
    Ok(res) => {
        res.status      // 200
        res.ok          // true (status 200-299)
        res.body        // Raw body string
        res.json()      // Parsed JSON (lazy)
        res.headers     // Map of headers
    },
    Err(e) => print("Failed: " + e)
}
```

### Options Reference

| Option    | Type   | Default | Description                        |
| --------- | ------ | ------- | ---------------------------------- |
| `method`  | String | `"GET"` | HTTP method                        |
| `headers` | Map    | `{}`    | Request headers                    |
| `body`    | String | —       | Raw body                           |
| `json`    | Any    | —       | JSON body (auto sets Content-Type) |
| `form`    | Map    | —       | Form body (URL-encoded)            |
| `timeout` | Int    | `30`    | Timeout in seconds                 |
| `auth`    | Map    | —       | `{ "user": "x", "pass": "y" }`     |

### Caching (New)

```ntnt
import { fetch, Cache } from "std/http"

// Create a cache
let cache = Cache(300)  // 5-minute default TTL

// Fetch with caching
let data = cache.fetch("https://api.example.com/config")

// Override TTL per request
let data = cache.fetch(url, map { "ttl": 60 })

// Invalidate
cache.clear()                                   // Clear all
cache.delete("https://api.example.com/config")  // Clear one
```

### Implementation Changes

**Before (14 functions, 1200 lines):**

```
fetch, post, put, delete, patch, head, request,
get_json, post_json, basic_auth, post_form, download, upload
```

**After (3 exports, ~400 lines):**

```
fetch      — All HTTP requests
Cache      — Response caching
download   — File downloads (streams)
```

Everything consolidates into `fetch` with options. No more `get_json` (use `res.json()`), no more `post_form` (use `form` option), no more `basic_auth` (use `auth` option).

---

## Redesigned HTTP Server

### Principle: Express-like Simplicity

```ntnt
import { json, html, redirect } from "std/http/server"

fn home(req) {
    return html("<h1>Hello World</h1>")
}

fn get_user(req) {
    let id = req.params["id"]
    return json(map { "id": id, "name": "Alice" })
}

fn create_user(req) {
    let data = req.json()
    return json(map { "created": true }, 201)
}

get("/", home)
get(r"/users/{id}", get_user)
post("/users", create_user)
listen(8080)
```

**This already works.** The syntax is clean. Keep it.

### Request Object

```ntnt
req.method       // "GET", "POST", etc.
req.path         // "/users/123"
req.params       // Route params: { "id": "123" }
req.query        // Query params: { "page": "1" }
req.headers      // Request headers
req.body         // Raw body string
req.json()       // Parse body as JSON
req.form()       // Parse body as form data
req.ip           // Client IP (from X-Forwarded-For)
req.id           // Request ID (from X-Request-ID)
req.session      // Session data (if sessions enabled)
```

### Response Helpers

```ntnt
import { json, html, text, redirect, status } from "std/http/server"

json(data)              // 200 + application/json
json(data, 201)         // Custom status
html("<h1>Hi</h1>")     // 200 + text/html
text("plain text")      // 200 + text/plain
redirect("/other")      // 302 redirect
status(404, "Not found") // Custom status + body
```

### Sessions (New)

```ntnt
import { json, redirect } from "std/http/server"

// Enable sessions (call once at startup)
use_session(map {
    "secret": get_env("SESSION_SECRET"),
    "max_age": 86400 * 7
})

fn login(req) {
    let user = authenticate(req.json())
    req.session["user_id"] = user.id
    return json(map { "success": true })
}

fn dashboard(req) {
    if req.session["user_id"] == None {
        return redirect("/login")
    }
    return html(render_dashboard())
}

fn logout(req) {
    req.session.clear()
    return redirect("/")
}
```

### Middleware

```ntnt
fn auth_required(req, next) {
    if req.session["user_id"] == None {
        return redirect("/login")
    }
    return next(req)
}

fn log_requests(req, next) {
    let start = now()
    let res = next(req)
    print("{req.method} {req.path} {res.status} {now() - start}ms")
    return res
}

// Global middleware
use_middleware(log_requests)

// Per-route middleware
get("/admin", admin_handler, map { "middleware": [auth_required] })
```

### WebSocket (New)

```ntnt
fn chat_handler(ws) {
    ws.on_message(fn(msg) {
        ws.room.broadcast(msg)  // Send to all in room
    })

    ws.on_close(fn() {
        print("Disconnected: " + ws.id)
    })
}

websocket("/ws/chat", chat_handler)
listen(8080)
```

### Graceful Shutdown

```ntnt
// Cleanup on shutdown
on_shutdown(fn() {
    close(db)
})

// Health check for nginx
get("/health", fn(req) { json(map { "status": "ok" }) })

// Start with graceful shutdown support
listen(8080, map { "shutdown_timeout": 30 })
```

---

## Implementation Plan

### Phase 0: Migration Preparation (1 day)

**Audit existing usage and prepare for breaking changes.**

1. Find all HTTP client usage:

   ```bash
   grep -r "get_json\|post_json\|post_form\|put\|patch\|delete\|head\|basic_auth" examples/ tests/
   ```

2. Identify files to update:
   - [ ] `examples/http_client.tnt`
   - [ ] `examples/http_client_demo.tnt`
   - [ ] Any test files using HTTP client
   - [ ] Documentation examples

3. **Strategy: Hard break** (pre-1.0, acceptable)
   - Remove old functions entirely
   - Update all examples in same PR
   - Document breaking changes in CHANGELOG.md

### Phase 1: HTTP Client Cleanup (3 days)

Consolidate 14 functions → 3:

```rust
// Keep
fetch(url_or_options) -> Result<Response>  // All HTTP
Cache::new(ttl) -> Cache                   // Caching
download(url, path) -> Result<FileInfo>    // File streaming

// Remove (absorbed into fetch)
post, put, delete, patch, head, request,
get_json, post_json, basic_auth, post_form, upload
```

### Phase 2: Server Polish (3 days)

1. Trust proxy headers → `req.ip`, `req.id`, `req.protocol`
2. Add `req.json()` and `req.form()` helpers
3. Graceful shutdown with `on_shutdown()` hook

### Phase 3: Sessions (4 days)

1. `use_session(options)` global setup
2. `req.session` map on each request
3. Signed cookies (HMAC)
4. In-memory store (default) + Redis option

### Phase 4: WebSocket (5 days)

1. `websocket(pattern, handler)` builtin
2. `ws.send()`, `ws.on_message()`, `ws.on_close()`
3. Room abstraction for broadcast

### Phase 5: Documentation & Agent Instructions (3 days)

1. Update `.github/copilot-instructions.md` with new HTTP patterns
2. Update `docs/AI_AGENT_GUIDE.md` with full API reference
3. Update `LANGUAGE_SPEC.md` with new builtins
4. Update `CLAUDE.md` with HTTP client/server usage

### Phase 6: Testing (4 days)

1. HTTP client tests (fetch, Cache, error handling)
2. HTTP server tests (req helpers, shutdown)
3. Session tests (cookies, persistence, expiry)
4. WebSocket tests (connect, message, broadcast)
5. Integration tests (full auth flow, chat)

**Total: ~4 weeks**

---

## Phase 5: Documentation & Agent Instructions (3 days)

### 5.1 Update Agent Instructions

Update `.github/copilot-instructions.md` with:

```markdown
## HTTP Client

### fetch() - All HTTP Requests

// Simple GET
let response = fetch("https://api.example.com/data")

// POST with JSON
let response = fetch(url, map {
"method": "POST",
"json": map { "name": "Alice" }
})

// With authentication
let response = fetch(url, map {
"auth": map { "user": "admin", "pass": "secret" }
})

### Response Handling

match fetch(url) {
Ok(res) => {
let data = res.json() // Parse JSON
print(res.status) // Status code
},
Err(e) => print("Error: " + e)
}

### Caching API Responses

import { fetch, Cache } from "std/http"

let cache = Cache(300) // 5-minute TTL
let data = cache.fetch("https://api.example.com/config")
```

### 5.2 Update AI_AGENT_GUIDE.md

Add new sections:

- HTTP Client: `fetch()` options reference
- HTTP Server: Sessions, WebSocket, middleware patterns
- Production deployment: nginx configuration examples

### 5.3 Update LANGUAGE_SPEC.md

Document new builtins:

- `websocket(pattern, handler)`
- `use_session(options)`
- `on_shutdown(fn)`

---

## Phase 6: Testing (4 days)

### 6.1 HTTP Client Tests

| Test                        | Description                     |
| --------------------------- | ------------------------------- |
| `test_fetch_get`            | Simple GET request              |
| `test_fetch_post_json`      | POST with JSON body             |
| `test_fetch_with_headers`   | Custom headers                  |
| `test_fetch_with_auth`      | Basic authentication            |
| `test_fetch_timeout`        | Request timeout                 |
| `test_fetch_error_handling` | Network errors return Err       |
| `test_cache_hit`            | Cached response returned        |
| `test_cache_miss`           | Cache miss fetches from network |
| `test_cache_ttl_expiry`     | Expired entries re-fetched      |
| `test_cache_invalidation`   | Manual cache.delete() works     |

### 6.2 HTTP Server Tests

| Test                      | Description                    |
| ------------------------- | ------------------------------ |
| `test_req_json_parsing`   | `req.json()` parses body       |
| `test_req_form_parsing`   | `req.form()` parses form data  |
| `test_req_ip_from_proxy`  | `req.ip` reads X-Forwarded-For |
| `test_req_id_propagation` | `req.id` reads X-Request-ID    |
| `test_graceful_shutdown`  | In-flight requests complete    |
| `test_on_shutdown_hook`   | Cleanup function called        |

### 6.3 Session Tests

| Test                           | Description                      |
| ------------------------------ | -------------------------------- |
| `test_session_set_get`         | Store and retrieve session data  |
| `test_session_persistence`     | Session persists across requests |
| `test_session_clear`           | `session.clear()` removes data   |
| `test_session_cookie_signed`   | Cookie is HMAC-signed            |
| `test_session_cookie_httponly` | Cookie has HttpOnly flag         |
| `test_session_expiry`          | Session expires after max_age    |

### 6.4 WebSocket Tests

| Test                            | Description                   |
| ------------------------------- | ----------------------------- |
| `test_websocket_connect`        | Client can connect            |
| `test_websocket_send_receive`   | Messages sent and received    |
| `test_websocket_on_close`       | Close handler called          |
| `test_websocket_room_broadcast` | Broadcast reaches all clients |
| `test_websocket_room_leave`     | Client removed from room      |

### 6.5 Integration Tests

| Test                    | Description                                |
| ----------------------- | ------------------------------------------ |
| `test_full_auth_flow`   | Login → session → protected route → logout |
| `test_api_with_caching` | Cached API client in server handler        |
| `test_websocket_chat`   | Multi-client chat scenario                 |

---

## Checklist Before Merge

- [ ] All new functions documented in `docs/AI_AGENT_GUIDE.md`
- [ ] Agent instructions updated in `.github/copilot-instructions.md`
- [ ] `CLAUDE.md` updated with HTTP patterns
- [ ] `LANGUAGE_SPEC.md` updated with new builtins
- [ ] Unit tests for HTTP client (10+ tests)
- [ ] Unit tests for HTTP server (6+ tests)
- [ ] Unit tests for sessions (6+ tests)
- [ ] Unit tests for WebSocket (5+ tests)
- [ ] Integration tests (3+ tests)
- [ ] Example files in `examples/` directory
- [ ] CHANGELOG.md updated

---

## API Summary

### HTTP Client (`std/http`)

```ntnt
fetch(url) -> Result<Response>
fetch(url, options) -> Result<Response>

Cache(ttl) -> Cache
cache.fetch(url) -> Result<Response>
cache.delete(url)
cache.clear()

download(url, path) -> Result<FileInfo>
```

### HTTP Server (builtins + `std/http/server`)

```ntnt
// Response builders (import from std/http/server)
json(data, status?) -> Response
html(content, status?) -> Response
text(content, status?) -> Response
redirect(url) -> Response
status(code, body) -> Response

// Builtins (no import needed)
get(pattern, handler)
post(pattern, handler)
put(pattern, handler)
delete(pattern, handler)
patch(pattern, handler)
websocket(pattern, handler)
use_middleware(fn)
use_session(options)
on_shutdown(fn)
serve_static(prefix, dir)
listen(port, options?)
```

---

## What nginx Handles

| Feature       | nginx Config           |
| ------------- | ---------------------- |
| TLS/HTTPS     | `ssl_certificate`      |
| Compression   | `gzip on`              |
| Rate limiting | `limit_req_zone`       |
| Body limits   | `client_max_body_size` |
| Static files  | `location /static`     |
| Access logs   | `access_log`           |
| HTTP/2        | `http2 on`             |

---

## Success Criteria

- **Simple**: Hello world = 5 lines
- **Familiar**: Express/Flask patterns
- **Fast**: <1ms routing overhead
- **Secure**: Signed session cookies, HttpOnly default
- **Maintainable**: <2000 lines total
