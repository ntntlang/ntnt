# NTNT Security Audit

**Date:** 2026-02-16  
**Version:** v0.3.13 (branch: security-and-performance-audit)  
**Auditor:** Automated security review  

## Summary

Overall, NTNT demonstrates **strong security awareness** — many common vulnerabilities have already been addressed (SSRF protection, path traversal prevention, parameterized queries, CSRF tokens, constant-time comparison, secure cookie defaults). This audit found **no critical exploitable vulnerabilities** but identifies several medium and low-severity hardening opportunities.

| Severity | Count |
|----------|-------|
| 🔴 Critical | 1 |
| 🟡 Medium | 7 |
| 🟢 Low | 8 |

---

## 🔴 Critical Findings

### C-1: Async HTTP Server — No Path Traversal Protection on Static Files

**File:** `src/stdlib/http_server_async.rs` (lines 160-175)  
**Attack Vector:** The async server's `find_static_file()` method does NOT check for `..` path traversal or perform canonicalization, unlike the sync server which has thorough protection.

```rust
// http_server_async.rs - VULNERABLE
pub async fn find_static_file(&self, path: &str) -> Option<(String, String)> {
    for dir in dirs.iter() {
        if path.starts_with(&dir.url_prefix) {
            let relative = path.strip_prefix(&dir.url_prefix).unwrap_or("");
            let relative = relative.trim_start_matches('/');
            let file_path = PathBuf::from(&dir.fs_path).join(relative);
            if file_path.exists() && file_path.is_file() {
                return Some((...));
            }
        }
    }
}
```

An attacker could request `GET /static/../../etc/passwd` and read arbitrary files on the system.

**Fix:** Port the sync server's path traversal checks (`.contains("..")`, URL decode check, canonicalize + starts_with) to the async server. **Implemented in this PR.**

---

## 🟡 Medium Findings

### M-1: No Recursion Depth Limit in Interpreter

**File:** `src/interpreter.rs`  
**Risk:** A malicious or buggy NTNT script can cause stack overflow via deep recursion. The interpreter's `call_function()` has no depth counter.

```ntnt
fn evil(n) { return evil(n + 1) }
evil(0)  // Stack overflow → process crash
```

**Impact:** Denial of service — crashes the entire server process.  
**Recommendation:** Add a configurable `max_call_depth` (default: 1000) to the interpreter. Increment on function entry, decrement on exit, error if exceeded. **Proposed — needs Josh's review for implementation approach.**

### M-2: Request Body Size Limit — Already Implemented ✅

**File:** `src/stdlib/http_server.rs` (line 2611)  
**Status:** No issue. The sync server's `process_request()` already enforces `max_body_size` (default 10MB) using `take()` on the reader and checking both Content-Length header and actual bytes read. The async server also limits to 10MB via `axum::body::to_bytes`.

### M-3: Default Session Secret is Predictable

**File:** `src/stdlib/auth.rs` (line 843)  
**Risk:** `DEFAULT_SESSION_SECRET` is `"ntnt-dev-secret-change-in-production"`. While production mode correctly refuses to start with this value, development mode only warns. If a dev server is accidentally exposed, session cookies can be forged.

**Impact:** Session forgery in development environments exposed to the internet.  
**Current Mitigation:** Production mode exits with fatal error if default secret is used. This is good.  
**Recommendation:** Consider generating a random secret on first startup in dev mode (write to `.ntnt-dev-secret` file). **Proposed — low priority given existing production guard.**

### M-4: No Response Size Limit in HTTP Client

**File:** `src/stdlib/http.rs`  
**Risk:** `http_get()` calls `response.text()` which reads the entire response body into memory with no size limit. A malicious server (or SSRF to an internal endpoint) could return gigabytes of data.

**Impact:** Memory exhaustion of the NTNT process.  
**Recommendation:** Add a configurable response size limit (default: 50MB) using `response.bytes()` with a limit check. **Implemented in this PR.**

### M-5: CORS Default is Wildcard Origin

**File:** `src/stdlib/http_server.rs` (line 205)  
**Risk:** `CorsConfig::default()` uses `origins: vec!["*"]` which allows any origin. While `credentials: false` (correct — prevents `*` with credentials), developers who call `enable_cors()` without arguments get a fully permissive CORS policy.

**Impact:** Cross-origin API access from any website.  
**Recommendation:** Document this clearly and consider warning when `*` origin is used in production mode. The current behavior is technically correct (credentials=false with * is safe per spec), but could surprise developers. **Documented — no code change needed.**

### M-6: No Timeout for Sync HTTP Server Requests

**File:** `src/interpreter.rs`  
**Risk:** The sync `tiny_http` server has no per-request timeout. A slow handler (e.g., infinite loop in NTNT code, slow database query) ties up the single interpreter thread indefinitely, blocking all other requests.

**Impact:** Denial of service via slow requests.  
**Current Mitigation:** The async server has a 30-second timeout via `TimeoutLayer`.  
**Recommendation:** Add a per-request timeout to the sync server using a watchdog thread. **Proposed — architectural change needs review.**

### M-7: PostgreSQL Connection String May Leak in Errors

**File:** `src/stdlib/postgres.rs`  
**Risk:** If `pg_connect()` fails, the error message from the postgres crate may include the connection string (which contains username/password). This error propagates to NTNT code as `Err("Connection failed: ...")`.

```rust
Err(e) => Ok(Value::EnumValue {
    variant: "Err".to_string(),
    values: vec![Value::String(format!("Connection failed: {}", e))],
})
```

**Impact:** Credential leakage to application error handlers.  
**Recommendation:** Sanitize connection errors to remove credentials. **Implemented in this PR.**

---

## 🟢 Low Findings

### L-1: File System Module Has No Sandboxing

**File:** `src/stdlib/fs.rs`  
**Risk:** `read_file()`, `write_file()`, `remove_dir_all()` etc. operate on any path the NTNT process can access. There's no concept of a project root or allowed directories.

**Impact:** A NTNT script can read/write anywhere the process user has access.  
**Recommendation:** Consider an optional `NTNT_FS_ROOT` environment variable that restricts fs operations to a specific directory tree. **Proposed for future version.**

### L-2: Symlink Following in Static File Serving

**File:** `src/stdlib/http_server.rs`  
**Risk:** The `find_static_file()` method uses `canonicalize()` which follows symlinks. A symlink inside the static directory pointing outside it would pass the `starts_with()` check after canonicalization.

**Impact:** Potential directory escape via symlinks within the static directory.  
**Current Mitigation:** The attacker would need write access to create symlinks in the static dir.  
**Recommendation:** Add an option to reject symlinks in static file serving. **Low priority.**

### L-3: No Cookie Value Encoding

**File:** `src/stdlib/http_server.rs`  
**Risk:** While cookie names are validated (`is_valid_cookie_name`), cookie values are only sanitized via `sanitize_cookie_value()`. The sanitization removes control characters but doesn't URL-encode special characters, which could cause parsing issues.

**Impact:** Cookie parsing edge cases, not exploitable for injection.  
**Status:** Already handled — `sanitize_cookie_value()` removes semicolons, newlines, etc.

### L-4: bcrypt Default Cost Factor Could Be Higher

**File:** `src/stdlib/crypto.rs`  
**Risk:** `hash_password()` defaults to cost 12. OWASP recommends a minimum of 10, so this is compliant. However, modern hardware can brute-force cost-12 hashes at reasonable speeds.

**Impact:** Slightly weaker password hashing than optimal.  
**Recommendation:** Consider defaulting to cost 13 in a future version. Current cost 12 is acceptable.

### L-5: Argon2 Parameters Are OWASP Compliant ✅

**File:** `src/stdlib/crypto.rs`  
**Status:** No issue. Argon2id with m=19456, t=2, p=1 matches OWASP's first recommended configuration.

### L-6: HMAC Accepts Any Key Length

**File:** `src/stdlib/crypto.rs`  
**Risk:** `hmac_sha256()` accepts any key length including empty strings. While HMAC-SHA256 technically works with any key length, very short keys are weak.

**Impact:** Developers could use weak HMAC keys without warning.  
**Recommendation:** Consider warning on keys shorter than 32 bytes. **Low priority.**

### L-7: Redis KV Uses KEYS Command for Listing

**File:** `src/stdlib/kv.rs`  
**Risk:** `RedisKV::list()` uses the `KEYS` command which blocks Redis for large keyspaces. Redis documentation recommends `SCAN` instead.

**Impact:** Performance degradation on large Redis databases.  
**Recommendation:** Replace `KEYS` with `SCAN` for production use. **Proposed for future version.**

### L-8: Error Information Leakage Controlled by Config ✅

**File:** `src/stdlib/http_server.rs`  
**Status:** No issue. The `SecurityConfig` has `detailed_errors` that defaults to false in production mode, preventing internal error details from leaking to clients.

---

## Positive Security Findings (Already Implemented)

These are notable security features already present in the codebase:

1. **✅ SSRF Protection** (`src/stdlib/http.rs`) — Comprehensive IP validation, metadata endpoint blocking, configurable via environment variables
2. **✅ Path Traversal Prevention** (`src/stdlib/http_server.rs` sync) — `.contains("..")`, URL decode check, `canonicalize()` + `starts_with()` 
3. **✅ SQL Injection Prevention** — All database modules use parameterized queries exclusively
4. **✅ Constant-Time Comparison** (`src/stdlib/auth.rs`) — `constant_time_compare()` used for CSRF tokens and session validation
5. **✅ CSRF Protection** — OAuth state parameter with 10-minute expiry, per-session CSRF tokens
6. **✅ Session Security** — HttpOnly, Secure, SameSite=Lax by default; production secret requirement
7. **✅ Open Redirect Prevention** — `redirect_safe()` function with proper URL validation
8. **✅ Cookie Name Validation** — RFC 6265 compliant cookie name checking
9. **✅ Security Headers** — X-Content-Type-Options, X-Frame-Options, Referrer-Policy, X-XSS-Protection added by default
10. **✅ AES-256-GCM** — Proper authenticated encryption with random nonces
11. **✅ Random Nonce Generation** — Uses `OsRng` for cryptographic randomness throughout

---

## Recommendations Summary

### Must Fix (Before Production)
1. **C-1:** Add path traversal protection to async server static file serving

### Should Fix
2. **M-2:** Enforce body size limits in sync server
3. **M-4:** Add response size limits to HTTP client
4. **M-7:** Sanitize PostgreSQL connection errors

### Nice to Have
5. **M-1:** Add recursion depth limit to interpreter
6. **M-6:** Add request timeout to sync server
7. **L-1:** Optional filesystem sandboxing
8. **L-7:** Replace Redis KEYS with SCAN
