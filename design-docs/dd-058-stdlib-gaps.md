# DD-058: Stdlib Gap Analysis — Eliminating Common Package Dependencies

**Status:** Draft
**Author:** Larri + Josh
**Created:** 2026-03-31
**App:** ntnt

---

## Motivation

Most npm/PyPI packages exist because the host language ships with a minimal stdlib. ntnt's batteries-included approach already covers the majority — but there are gaps that would force a developer to leave the ntnt ecosystem for common web tasks. This DD catalogs those gaps and proposes stdlib additions to close them.

The goal: **a developer building a standard web application should never need an external package for common functionality.** If 80%+ of web apps need it, it belongs in the stdlib.

---

## Current Coverage (Already Solved)

ntnt's stdlib is surprisingly complete. Here's what's already covered, mapped to the equivalent npm/PyPI packages developers would otherwise install:

| Need | npm/PyPI equivalent | ntnt module | Key functions |
|------|---------------------|-------------|---------------|
| HTTP client | axios, requests | `std/http` | `fetch()`, `download()` |
| JSON | json5, built-in | `std/json` | `parse_json()`, `stringify()` |
| UUID | uuid | `std/crypto` | `uuid()` |
| Env vars / .env | dotenv | `std/env` | `load_env()`, `get_env()` |
| Date/time + arithmetic | moment, dayjs, date-fns | `std/time` | `now()`, `format()`, `add_days()`, `diff()`, `parse_datetime()`, `to_timezone()`, timezone support, weekday/month extractors |
| String ops | lodash (strings) | `std/string` | `split()`, `trim()`, `replace()`, `to_lower()`, etc. |
| Collections | lodash (arrays/objects) | `std/collections` | `keys()`, `values()`, `sort()`, `reverse()`, `entries()`, etc. |
| Regex | built-in regex | `std/string` | `matches_pattern()`, `replace_pattern()`, `capture_pattern()`, `find_pattern()`, `split_pattern()` |
| Path manipulation | path (node) | `std/path` | Path utilities |
| File system | fs, graceful-fs | `std/fs` | `read_file()`, `write_file()`, `exists()`, `mkdir_all()`, `readdir()` |
| CSV parsing | csv-parser, Papa Parse | `std/csv` | `parse_csv()` |
| URL parsing | whatwg-url | `std/url` | URL utilities |
| Crypto/hashing | crypto, hashlib | `std/crypto` | `sha256()`, `hmac_sha256()`, `random_bytes()`, `aes_encrypt/decrypt()`, `base64_encode/decode()` |
| Password hashing | bcrypt, argon2, passlib | `std/crypto` | `hash_password()` (bcrypt), `argon2_hash()`, `argon2_verify()` |
| PostgreSQL | pg, psycopg2 | `std/db/postgres` | Full query/execute/transactions |
| SQLite | better-sqlite3 | `std/db/sqlite` | Full query/execute |
| KV store | ioredis | `std/kv` | `get()`, `set()`, typed getters |
| Templates | handlebars, ejs, Jinja2 | Built-in | `template()`, `{{}}` syntax, block helpers, filters |
| CORS | cors | Built-in | `enable_cors()` |
| CSP | helmet | Built-in | `enable_csp()` |
| Auth/sessions | passport, express-session | `std/auth` | Sessions, CSRF, OAuth (incl. OIDC discovery), JWT, TOTP/2FA, API keys, Turnstile |
| Markdown | marked, markdown-it | `std/markdown` | Markdown rendering |
| Logging | winston, pino | `std/log` | Logging |
| Background jobs | bull, celery | `std/jobs` | Full job system with priority queues, retry, cron, dedup |
| Concurrency | — | `std/concurrent` | `spawn()`, channels, `parallel()`, `race()`, `schedule()` |
| Math | — | `std/math` | Math functions |
| CSRF | csurf | `std/auth` | `csrf_token()`, `validate_csrf()`, `csrf_generate/validate()` |

**That's 30+ categories already covered.** The stdlib spans 25 modules and 357+ functions.

---

## Gaps to Fill

These are capabilities that 80%+ of web apps need, that developers commonly install packages for, and that ntnt doesn't have yet. Ordered by impact.

### Priority 1: High Impact (Most Web Apps Need These)

#### 1. Validation / Schema (`std/validate`)

**The gap:** Every web app validates incoming data — form submissions, API payloads, query params. ntnt has contracts (`requires`/`ensures`) for function-level assertions, but no declarative schema validation for incoming data.

**What developers install instead:** zod, joi, yup, ajv (npm); pydantic, marshmallow, cerberus (PyPI)

**Proposed API:**
```ntnt
import { schema, validate, required, email, min, max, int, string, one_of, matches } from "std/validate"

let user_schema = schema(map {
    "email": [required, email],
    "age": [required, int, min(13), max(120)],
    "name": [required, string, min(1), max(100)],
    "role": [one_of(["admin", "user", "editor"])],
    "phone": [matches("^\\+?[0-9]{10,15}$")]
})

let result = validate(user_schema, parse_form(req))
// Ok(map { "email": "...", "age": 25, ... })  — cleaned + coerced values
// Err(map { "email": "Required", "age": "Must be at least 13" })
```

**Built-in validators:**
| Validator | Description |
|-----------|-------------|
| `required` | Field must be present and non-empty |
| `string` | Must be a string |
| `int` | Must parse as integer |
| `float` | Must parse as float |
| `bool` | Must be true/false/1/0 |
| `email` | Valid email format |
| `url` | Valid URL format |
| `min(n)` / `max(n)` | Min/max for numbers |
| `min_length(n)` / `max_length(n)` | String length bounds |
| `one_of(options)` | Must be one of the listed values |
| `matches(pattern)` | Regex match |
| `optional` | Field can be absent (default: required) |
| `default(value)` | Use default if absent |
| `trim` | Strip whitespace before validation |
| `custom(fn)` | Custom validation function |

**Why stdlib, not a package:** Validation is so fundamental that every app reinvents it. Having one idiomatic way in the stdlib means better error messages, integration with the contract system, and consistent patterns across all ntnt apps.

**Scope:** ~500-800 lines of Rust. No new dependencies (regex already in use).

#### 2. Email Sending (`std/email`)

**The gap:** Every web app with users eventually sends email — signup confirmations, password resets, notifications, contact forms. Currently requires `fetch()` to an external API (Resend, SendGrid) or shelling out.

**What developers install instead:** nodemailer (npm); smtplib is built-in for Python but needs helpers

**Proposed API:**
```ntnt
import { send_email, configure_email } from "std/email"

// Configure once at startup
configure_email(map {
    "host": get_env("SMTP_HOST"),
    "port": 587,
    "username": get_env("SMTP_USER"),
    "password": get_env("SMTP_PASS"),
    "from": "hello@myapp.com",
    "from_name": "My App"
})

// Send
let result = send_email(map {
    "to": "user@example.com",
    "subject": "Welcome!",
    "html": template("emails/welcome.html", map { "name": user.name }),
    "text": "Welcome, #{user.name}!"   // plain text fallback
})
// Result<Map, String> — Ok with message ID, Err with error
```

**Additional functions:**
| Function | Description |
|----------|-------------|
| `send_email(opts)` | Send a single email |
| `send_email_batch(emails)` | Send multiple (reuses connection) |
| `configure_email(opts)` | Set SMTP config (host, port, auth, TLS) |

**Scope:** ~400-600 lines of Rust. Needs `lettre` crate (mature Rust SMTP library).

#### 3. Rate Limiting (`std/http/rate_limit` or middleware)

**The gap:** Almost every API needs rate limiting. DD-051 covers the design. Without it, developers must implement token buckets or sliding windows by hand.

**What developers install instead:** express-rate-limit (npm); slowapi, flask-limiter (PyPI)

**Proposed API:**
```ntnt
import { rate_limit } from "std/http/rate_limit"

// As middleware
use_middleware(rate_limit(map {
    "limit": 100,
    "window": "1 minute",
    "by": "ip",                    // or "header:X-API-Key" or custom fn
    "message": "Too many requests"
}))

// Per-route
post("/api/login", rate_limit(map { "limit": 5, "window": "15 minutes" }), login_handler)
```

**Scope:** ~300-500 lines of Rust. In-memory sliding window (with optional Redis backend via `std/kv`). Covered in DD-051.

#### 4. Multipart File Uploads

**The gap:** `parse_form()` handles `application/x-www-form-urlencoded` but likely not `multipart/form-data` with binary file content. Any app with a file upload form (images, documents, CSV imports) hits this.

**What developers install instead:** multer, formidable, busboy (npm); python-multipart (PyPI)

**Proposed API:**
```ntnt
// parse_form already works for url-encoded
// Extend it to handle multipart:
let form = parse_form(req)
// For multipart, file fields become maps:
// form["avatar"] => map { "filename": "photo.jpg", "content_type": "image/jpeg", "size": 204800, "data": <bytes> }

// Or a dedicated function for clarity:
import { parse_multipart } from "std/http/server"
let files = parse_multipart(req)
// files["avatar"] => map { "filename": "photo.jpg", "data": <bytes>, ... }

// Save to disk
write_file("uploads/#{files.avatar.filename}", files.avatar.data)
```

**Config:**
```ntnt
// Set upload limits globally
configure_uploads(map {
    "max_file_size": 10 * 1024 * 1024,  // 10MB
    "max_files": 5,
    "allowed_types": ["image/jpeg", "image/png", "application/pdf"]
})
```

**Scope:** ~400-600 lines of Rust. `multer` crate exists for Rust/Axum integration.

### Priority 2: Medium Impact (Many Apps Need These)

#### 5. WebSocket Support (`std/ws`)

**The gap:** Real-time features — chat, live notifications, collaborative editing, live dashboards. HTTP polling works but is inefficient and laggy.

**What developers install instead:** ws, socket.io (npm); websockets (PyPI)

**Proposed API:**
```ntnt
import { websocket } from "std/ws"

websocket("/ws/chat", fn(ws) {
    ws.on("message", fn(msg) {
        // Broadcast to all connected clients
        ws.broadcast("chat", msg)
    })

    ws.on("close", fn() {
        log("Client disconnected")
    })

    ws.send("Welcome!")
})
```

**Scope:** Larger effort (~1000+ lines). Axum has built-in WebSocket support via `axum::extract::ws`, so the plumbing exists. Main work is the ntnt API design and managing connection state.

#### 6. XML Parsing (`std/xml`)

**The gap:** RSS feeds, SOAP APIs, sitemaps, SVG manipulation, data imports from legacy systems. Less common than JSON but still needed regularly.

**What developers install instead:** xml2js, cheerio (npm); lxml, BeautifulSoup, xml.etree (PyPI)

**Proposed API:**
```ntnt
import { parse_xml, to_xml } from "std/xml"

let doc = parse_xml("<root><item id='1'>Hello</item></root>")
// doc => map { "root": map { "item": map { "@id": "1", "#text": "Hello" } } }

let xml_str = to_xml(map { "root": map { "items": [1, 2, 3] } })
```

**Scope:** ~400-600 lines. `quick-xml` crate (fast, well-maintained).

#### 7. Cron Expression Parsing

**The gap:** The job system supports scheduling, but a standalone cron parser would be useful for: next occurrence calculation, human-readable descriptions, validation.

**What developers install instead:** node-cron, cron-parser (npm); croniter, APScheduler (PyPI)

**Proposed API:**
```ntnt
import { cron_next, cron_matches, cron_describe } from "std/time"

let next = cron_next("0 9 * * MON")      // Next Monday at 9:00 AM (timestamp)
let matches = cron_matches("*/5 * * * *", now())  // Does current time match?
let desc = cron_describe("0 9 * * MON")   // "Every Monday at 9:00 AM"
```

**Scope:** ~200-300 lines. `cron` crate exists in Rust.

#### 8. HTML Sanitization

**The gap:** Any app that accepts user-generated HTML content (rich text editors, comments, blog posts) needs to strip dangerous tags/attributes to prevent XSS. Different from template auto-escaping — this is for content that's *supposed* to contain HTML.

**What developers install instead:** DOMPurify, sanitize-html (npm); bleach, nh3 (PyPI)

**Proposed API:**
```ntnt
import { sanitize_html } from "std/string"

let clean = sanitize_html(user_input, map {
    "allowed_tags": ["p", "b", "i", "a", "ul", "li", "br"],
    "allowed_attrs": map { "a": ["href", "title"] }
})
// Strips <script>, onclick=, javascript:, etc.
```

**Scope:** ~300-400 lines. `ammonia` crate (Rust, well-maintained, used by docs.rs).

### Priority 3: Nice to Have

#### 9. Slug Generation

**The gap:** URL-friendly slugs from titles. Every blog, CMS, or content site needs this.

**Proposed:** `slugify("Hello World! 🌍")` → `"hello-world"` — add to `std/string`.

**Scope:** ~50-100 lines. Straightforward.

#### 10. Pagination Helpers

**The gap:** Every list view needs pagination math — offset, limit, total pages, page links.

**Proposed:** `paginate(total_items, page, per_page)` → `map { "offset": 20, "limit": 10, "total_pages": 5, "has_next": true, ... }` — add to `std/collections` or `std/http/server`.

**Scope:** ~50-100 lines. Pure logic, no dependencies.

#### 11. Human-Friendly Time (`from_now`, `time_ago`)

**The gap:** "3 hours ago", "in 2 days", "just now". `std/time` has `diff()` but not the human-readable relative format.

**Proposed:** `from_now(timestamp)` → `"3 hours ago"` or `"in 2 days"` — add to `std/time`.

**Scope:** ~100-150 lines. Pure logic.

#### 12. QR Code Generation

**The gap:** Useful for 2FA setup (TOTP URI → QR), payment links, mobile deep links. The `totp_uri()` function in `std/auth` generates the URI but not the QR image.

**Proposed:** `qr_code(data, opts?)` → PNG bytes — add to `std/crypto` or new `std/qr`.

**Scope:** ~200 lines. `qrcode` crate.

---

## What We're NOT Adding (and Why)

| Category | Examples | Why skip |
|----------|----------|----------|
| CLI arg parsing | commander, yargs, argparse | ntnt apps are web servers, not CLIs |
| Build tools / bundling | webpack, rollup, esbuild, vite | ntnt doesn't need a build step |
| Linting / formatting | eslint, prettier, black | ntnt has `ntnt lint` and `ntnt fmt` built in |
| Test frameworks | jest, mocha, chai, pytest | ntnt has intent-driven testing (IDD) |
| ORM / query builders | prisma, sequelize, SQLAlchemy | ntnt's raw SQL + `pg_query` is idiomatic. ORMs add abstraction that fights the simplicity goal. |
| React/Vue/frontend | react, vue, svelte | ntnt is server-rendered. Frontend framework integration is out of scope for stdlib. |
| TypeScript tooling | ts-node, tsx | ntnt has its own type system |
| Process managers | pm2, forever | ntnt runs in Docker with health checks |
| Logging transports | winston-transport, pino-pretty | `std/log` covers it; transport to external services via `fetch()` |
| GraphQL | apollo-server, graphql-yoga | REST/JSON is ntnt's paradigm. GraphQL could be a future package, not stdlib. |

---

## Implementation Order

Based on impact, effort, and dependencies:

| Order | Module | Effort | Impact | Notes |
|-------|--------|--------|--------|-------|
| 1 | `std/validate` | Medium (500-800 LOC) | Very High | Every app needs it. No new deps. |
| 2 | `std/email` | Medium (400-600 LOC) | High | `lettre` crate. Every app with users. |
| 3 | Rate limiting | Medium (300-500 LOC) | High | Covered in DD-051. Middleware-level. |
| 4 | Multipart uploads | Medium (400-600 LOC) | High | `multer` crate. File upload forms. |
| 5 | `from_now()` / `slugify()` / `paginate()` | Small (200 LOC total) | Medium | Quick wins, extend existing modules. |
| 6 | HTML sanitization | Small (300-400 LOC) | Medium | `ammonia` crate. UGC apps. |
| 7 | Cron expressions | Small (200-300 LOC) | Medium | `cron` crate. Extend `std/time`. |
| 8 | `std/xml` | Medium (400-600 LOC) | Medium | `quick-xml` crate. Integrations. |
| 9 | `std/ws` (WebSocket) | Large (1000+ LOC) | Medium | Axum has WS support. Real-time apps. |
| 10 | QR codes | Small (200 LOC) | Low | `qrcode` crate. 2FA UX. |

**Total:** ~3,500-5,500 lines of Rust across all additions. Items 1-4 cover the critical gaps. Items 5-7 are quick wins. Items 8-10 are nice-to-haves.

---

## Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| Validation errors format? | Map of field→message vs structured error objects | Map of field→message (simple, matches how forms render errors) |
| Email: SMTP only or also API? | SMTP only vs SMTP + provider APIs (Resend, SendGrid) | SMTP only in stdlib. Provider APIs are just `fetch()` calls. |
| File upload storage? | Return bytes vs stream to disk | Return bytes for small files (<10MB), stream option for large |
| WebSocket: full duplex or SSE first? | WebSocket vs Server-Sent Events | SSE first (DD-041 exists). WebSocket after. |
| Should validation integrate with contracts? | Separate systems vs unified | Separate — contracts are for function invariants, validation is for external input |

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-31 | Initial draft — gap analysis against top npm/PyPI packages |
