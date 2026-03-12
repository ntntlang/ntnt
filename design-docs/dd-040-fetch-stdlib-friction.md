# DD-040: fetch() & SQL Stdlib Friction Points

**Status:** Draft  
**Author:** Larri  
**Date:** 2026-03-12  
**Context:** Building Larri Design domain management feature (DD-010)

## Summary

During the DD-010 implementation, six friction points in ntnt's stdlib caused repeated runtime errors and hours of debugging. Each issue silently failed or produced cryptic errors, making root-cause analysis difficult. This doc proposes fixes to eliminate each friction point.

---

## Issue 1: `fetch()` Returns Result — Must Unwrap

**Problem:** `fetch()` returns a `Value::Ok(response_map)` Result type. Accessing `resp["body"]` on the Result wrapper returns `None` instead of the response body. No error is thrown — it silently returns nothing.

**Current (broken):**
```
let resp = fetch(map { "url": "https://example.com/api" })
let body = resp["body"]   // → None (accessing Result, not inner map)
```

**Workaround:**
```
let resp = unwrap(fetch(map { "url": "https://example.com/api" }))
let body = resp["body"]   // → works
```

**Proposed Fix:** Either:
- (A) Make `fetch()` return the response map directly, throwing on network errors (like `query()` with `unwrap()`)
- (B) Make `Value::Ok(map)["key"]` transparently delegate to the inner map (auto-unwrap on index)
- (C) At minimum, add a clear error message when indexing into a Result type: `"Cannot index Result type — did you forget unwrap()?"` instead of silent None

**Impact:** Every `fetch()` call requires `unwrap()`. Easy to forget, silent failure.

---

## Issue 2: `fetch()` Takes 1 Argument, Not 2

**Problem:** The function signature is `fetch(url_or_options)` — a single argument that's either a URL string or an options map. But the natural pattern is `fetch(url, options)` like JavaScript.

**Current (broken):**
```
let resp = fetch(url, map { "headers": map { "Accept": "application/json" } })
// Error: function 'fetch' expected 1 arguments, got 2
```

**Workaround:**
```
let resp = fetch(map { "url": url, "headers": map { "Accept": "application/json" } })
```

**Proposed Fix:** Accept both signatures:
- `fetch("https://example.com")` — simple GET
- `fetch(map { "url": ..., "headers": ... })` — options map (current)
- `fetch("https://example.com", map { "headers": ... })` — URL + options (new, merge url into opts)

**Impact:** Confusing DX. Every other language/framework uses `fetch(url, options)`.

---

## Issue 3: JSONB Bind Parameters Don't Work

**Problem:** Passing a JSON string as a bind parameter with `::jsonb` cast fails with `unsupported jsonb version number 123`. PG's binary protocol can't handle text→jsonb conversion via bind params in ntnt's driver.

**Current (broken):**
```
let json_str = r#"{"key": "value"}"#
execute(db, "INSERT INTO t (data) VALUES ($1::jsonb)", [json_str])
// Error: unsupported jsonb version number 123
```

**Workaround:** Inline the JSON literal in the SQL using raw strings:
```
execute(db, r#"INSERT INTO t (data) VALUES ('{"key": "value"}'::jsonb)"#, [])
```

**Proposed Fix:** 
- Detect `::jsonb` cast on bind parameters and send the value as text with explicit `::text::jsonb` double cast
- Or add a `jsonb()` helper: `execute(db, "INSERT INTO t (data) VALUES ($1)", [jsonb(json_str)])`

**Impact:** Can't use dynamic JSON values as bind params. Must inline, which prevents parameterization.

---

## Issue 4: UUID Bind Parameters Need `::text::uuid` Double Cast

**Problem:** `$1::uuid` fails with `incorrect binary data format in bind parameter 1` because ntnt sends all params as text, but PG expects binary UUID format when `::uuid` is specified.

**Current (broken):**
```
execute(db, "INSERT INTO t (id) VALUES ($1::uuid)", [my_uuid])
// Error: incorrect binary data format in bind parameter 1
```

**Workaround:**
```
execute(db, "INSERT INTO t (id) VALUES ($1::text::uuid)", [my_uuid])
```

**Proposed Fix:**
- Detect `::uuid` cast and automatically send as `::text::uuid`
- Or configure the PG driver to always send params in text mode (not binary)

**Impact:** Every UUID INSERT/UPDATE needs the double cast. Easy to forget, cryptic error.

---

## Issue 5: String Interpolation in SQL Strings

**Problem:** ntnt's `{expr}` string interpolation triggers inside SQL strings containing `jsonb_set(checklist, '{key}', 'true')`. The `{key}` is interpreted as a variable reference, causing `Undefined variable` errors.

**Current (broken):**
```
execute(db, "UPDATE t SET data = jsonb_set(data, '{my_key}', 'true')", [])
// Error: Undefined variable: my_key
```

**Workaround:** Use raw strings `r#"..."#`:
```
execute(db, r#"UPDATE t SET data = jsonb_set(data, '{my_key}', 'true')"#, [])
```

**Proposed Fix:**
- Don't interpolate inside SQL strings passed to `execute()`/`query()` (they have `$N` params for that)
- Or add a lint/warning when `{identifier}` appears inside a `query()`/`execute()` string that isn't a `$N` placeholder

**Impact:** Every JSONB path expression in SQL needs raw strings. Especially painful with `jsonb_set`, `jsonb_extract_path`, etc.

---

## Issue 6: DNS-over-HTTPS Fails With `cloudflare-dns.com` in Docker

**Problem:** `cloudflare-dns.com` resolves to IPv6-only addresses in Docker containers. ntnt's reqwest client may fail to connect via IPv6, causing silent empty responses.

**Workaround:** Use `1.1.1.1` (IPv4 direct) instead of `cloudflare-dns.com`.

**Proposed Fix:** This is more of a deployment note than a language fix. But:
- Ensure reqwest is compiled with IPv6 support
- Consider adding a `dns_resolve` stdlib function that handles both IPv4/IPv6

---

## Priority

| Issue | Severity | Fix Complexity |
|-------|----------|---------------|
| 1. fetch() Result unwrap | 🔴 High | Medium (behavior change) |
| 2. fetch() arg count | 🟡 Medium | Low (overload) |
| 3. JSONB bind params | 🔴 High | Medium (driver change) |
| 4. UUID double cast | 🟡 Medium | Low (driver config) |
| 5. SQL string interpolation | 🟡 Medium | Medium (context-aware interp) |
| 6. IPv6 in Docker | 🟢 Low | N/A (deployment) |

Issues 1 and 3 cause the most pain — silent failures with no useful error messages.
