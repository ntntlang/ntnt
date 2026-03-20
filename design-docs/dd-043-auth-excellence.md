# DD-043: World-Class Auth — Making `std/auth` the Best stdlib Auth Ever

**Status:** Draft  
**Author:** Larri  
**Date:** 2026-03-20  
**Branch:** TBD (multi-phase)

---

## Vision

`std/auth` should be the auth system that makes developers say "I don't need a third-party auth service." One import, one function call, and you get security that matches or exceeds Auth0, Firebase Auth, Clerk, and Lucia — without the vendor lock-in, pricing tiers, or complexity.

**Design principle:** Secure by default, simple to use, hard to misconfigure. Every feature should either be automatic (the developer never thinks about it) or one line of config (the developer opts in explicitly). No feature should require the developer to understand the security implications — the safe path should be the easy path.

---

## Current State (v0.4.6)

### What We Have (and it's solid)
- [x] OAuth 2.0 + OIDC (Google, GitHub, Discord, Apple, Microsoft, generic)
- [x] PKCE for all OAuth flows
- [x] Server-side sessions (Redis, PostgreSQL, SQLite, memory)
- [x] HMAC-signed session cookies (tamper-proof)
- [x] CSRF protection (per-session tokens, automatic validation on state-changing requests)
- [x] Automatic token refresh (access tokens refreshed via refresh tokens transparently)
- [x] `HttpOnly`, `Secure`, `SameSite=Lax` cookies by default
- [x] Configurable session/refresh TTLs
- [x] Safari ITP workaround (two-phase exchange token flow)
- [x] Session listing (`user_sessions`) and revocation (`revoke_session`)
- [x] Periodic cleanup of expired sessions/tokens
- [x] Password hashing (bcrypt, argon2 via `std/crypto`)
- [x] TOTP/MFA support (`totp_secret`, `totp_verify`, `totp_uri`)
- [x] API key validation
- [x] Turnstile CAPTCHA verification
- [x] OAuth token introspection
- [x] Client credentials grant (M2M)
- [x] OIDC discovery (`.well-known/openid-configuration`)

### What's Missing (the gap to "best ever")

---

## Phase 1: Session Security Hardening

**Goal:** Make sessions bulletproof. These are table-stakes features that every serious auth system implements.

**Estimated effort:** Medium (2-3 PRs)

### 1.1 Session ID Rotation on Authentication
**Priority:** Critical  
**Reference:** OWASP Session Management Cheat Sheet, Section "Renew the Session ID After Any Privilege Level Change"

**Problem:** When a user authenticates, the session ID stays the same. An attacker who obtained the pre-auth session ID (e.g., via session fixation) gets promoted to an authenticated session.

**Solution:** After successful OAuth callback, before setting the cookie:
1. Create the session (as now)
2. Generate a **new** session ID
3. Migrate session data to the new ID
4. Delete the old session
5. Set the cookie with the new ID

This is automatic — no app developer action required.

**Implementation:**
```rust
// In handle_auth_callback, after create_session:
let new_id = generate_session_id();
migrate_session(&session.id, &new_id);  // copy data, delete old
session.id = new_id;
store_session(session);
```

- [ ] Add `migrate_session(old_id, new_id)` for all backends
- [ ] Rotate in `handle_auth_callback` Phase 2 (when cookie is set)
- [ ] Add `rotate_session(req)` as a public stdlib function for custom auth flows
- [ ] Tests: verify old session ID is invalid after rotation

### 1.2 Sliding Session Expiry
**Priority:** High  
**Reference:** OWASP "Session Expiration"

**Problem:** Sessions have a fixed `expires_at` set at creation time. A user who's actively using the app for 8 hours straight gets logged out at the session boundary, even if they've been active the entire time.

**Solution:** Add optional sliding window that extends `expires_at` on each authenticated request. Controlled by config:

```ntnt
enable_auth([google], map {
    "session_ttl": 86400 * 30,     // max lifetime: 30 days
    "idle_timeout": 86400 * 7,     // inactive for 7 days → session dies
    "sliding": true                 // extend idle_timeout on each request
})
```

**Behavior:**
- Each authenticated request updates `last_active_at` on the session
- Session is valid if: `now < expires_at AND now - last_active_at < idle_timeout`
- `expires_at` is an absolute ceiling — even with sliding, session dies after max lifetime
- If `sliding` is false (default for backward compat), behavior is unchanged

**Implementation:**
- [ ] Add `last_active_at` field to Session struct
- [ ] Add `idle_timeout` and `sliding` to AuthConfig
- [ ] Update `get_user_from_request` to check idle timeout and touch `last_active_at`
- [ ] Debounce the touch — only write if `last_active_at` is >60s stale (avoid write amplification)
- [ ] Update session in store on touch (all backends)
- [ ] Migration: existing sessions default `last_active_at = created_at`
- [ ] Tests: sliding extends, absolute ceiling holds, idle timeout kills

### 1.3 Absolute Session Lifetime Cap
**Priority:** High  
**Reference:** OWASP "Absolute Timeout"

**Problem:** With token refresh, sessions can theoretically live forever (refresh_ttl extends the session). There's no hard cap.

**Solution:** Add `max_lifetime` config (default: 90 days). Sessions beyond this are force-expired regardless of refresh tokens. Already somewhat implemented via `refresh_ttl`, but should be explicit and independently configurable.

- [ ] Add `max_lifetime` to AuthConfig (default: `refresh_ttl` or 90 days)
- [ ] Check `now - created_at > max_lifetime` in session validation
- [ ] When max lifetime is hit, session is dead — no auto-refresh possible
- [ ] Log `[auth] Session {id} exceeded max lifetime, forcing re-auth`

### 1.4 Session Cookie Name Customization
**Priority:** Low  
**Reference:** OWASP "Session ID Name Fingerprinting"

**Problem:** Cookie is always `ntnt_session`, which reveals the framework.

**Solution:** Already have `cookie_name` in AuthConfig, but it's hardcoded to `"ntnt_session"`. Allow overriding:

```ntnt
enable_auth([google], map {
    "cookie_name": "sid"  // generic, reveals nothing
})
```

- [ ] Parse `cookie_name` from config map in `enable_auth`
- [ ] Default remains `"ntnt_session"` for backward compat
- [ ] Document the recommendation to use a generic name

---

## Phase 2: OAuth Hardening (RFC 9700 Compliance)

**Goal:** Full compliance with RFC 9700 (OAuth 2.0 Security Best Current Practice, published January 2025).

**Estimated effort:** Medium (2-3 PRs)

### 2.1 Refresh Token Rotation
**Priority:** Critical  
**Reference:** RFC 9700 §4.14, OWASP

**Problem:** Refresh tokens are long-lived and static. If stolen, an attacker can silently generate new access tokens indefinitely.

**Solution:** On every token refresh, the authorization server should issue a **new** refresh token and invalidate the old one. Our auto-refresh already calls the provider's token endpoint, and most providers (Google, GitHub) return a new refresh token in the response. We should:

1. Always store the new refresh token when returned
2. Detect refresh token reuse (same old refresh token used twice → possible theft)
3. On reuse detection: revoke ALL sessions for that user (nuclear option)

**Implementation:**
- [ ] Add `refresh_token_hash` column to sessions (track which refresh token is current)
- [ ] On successful refresh: update stored refresh token, update hash
- [ ] On refresh with stale token (hash mismatch): flag as potential theft
- [ ] Config option: `"refresh_token_reuse": "revoke_all" | "warn" | "ignore"`
- [ ] Default: `"warn"` (log + continue) — `"revoke_all"` for strict mode
- [ ] Tests: rotation works, reuse detected, revocation cascade

### 2.2 State Parameter Entropy and Binding
**Priority:** Medium  
**Reference:** RFC 9700 §4.4

**Problem:** Our OAuth state parameter is a UUID (128 bits of entropy — good), but we should also bind it to the user's browser session to prevent state injection attacks.

**Solution:** Include a hash of the user's existing session cookie (if any) or a fingerprint of the request in the OAuth state. Validate on callback.

- [ ] Bind state to pre-auth session or request fingerprint
- [ ] Validate binding on callback before consuming state
- [ ] Reject states that don't match the originating browser

### 2.3 Authorization Code Replay Protection
**Priority:** Medium  
**Reference:** RFC 9700 §4.5

**Problem:** If an authorization code is intercepted and replayed, the attacker gets a valid session. PKCE mitigates this (which we already use), but we should also enforce single-use codes at our end.

**Current state:** We already consume OAuth state atomically (GETDEL/DELETE...RETURNING), which effectively prevents code replay since the state is consumed. This is already handled. Document and test.

- [ ] Verify and document that our state consumption prevents code replay
- [ ] Add explicit test for double-callback with same code

### 2.4 Redirect URI Exact Match Validation
**Priority:** Medium  
**Reference:** RFC 9700 §4.1

**Problem:** We trust the provider to validate redirect URIs, but we should also validate on our end that the callback came from an expected redirect URI.

- [ ] Store the redirect URI in OAuth state
- [ ] Validate on callback that the request URI matches
- [ ] Reject mismatches

---

## Phase 3: Developer Experience — 2-3x Easier Than Anything Else

**Goal:** Make ntnt auth the fastest path from zero to production-secure authenticated app in any language. Not by hiding things, but by making the right thing obvious at every layer.

**Design principle: Progressive Disclosure, Not Progressive Complexity.** Every layer should feel complete on its own. You don't learn Layer 2 exists until you need it. When you do need it, it's one clear step — not "now go read 40 pages of docs."

**Estimated effort:** Large (4-5 PRs — this is the DX differentiator)

### The Problem With Auth Today (Every Language)

**What developers actually hate about auth** (sourced from Reddit r/nextjs, r/webdev, Auth0 complaints, Clerk reviews):

1. **"Too many files / too many concepts before hello world."** Auth.js needs auth.ts + middleware.ts + route handler + provider config + adapter setup. Lucia needs db schema + adapter + auth module + middleware + session validation. Laravel Sanctum needs migration + config + middleware group + CORS setup. It's 5-8 files before you see a login page.

2. **"Silent misconfiguration."** Wrong session store URL? Sessions go to memory and vanish on restart — no error. Wrong TTL type? Falls back to default — no warning. Wrong redirect URI? Opaque OAuth error from Google. Developers lose hours to config bugs that could've been caught at startup.

3. **"The docs don't match reality."** Auth.js v5 is an "infinite beta" — docs show v4 patterns, v5 changed everything. Lucia deprecated itself. Firebase auth docs assume the Firebase ecosystem. When the stdlib IS the docs (because it's one function call), there's nothing to get out of sync.

4. **"I can't see what it's doing."** Sessions expire and nobody knows why. Tokens refresh and nobody knows when. The auth system is a black box until something breaks, then you're reading source code to understand what happened.

5. **"Middleware is confusing."** Which routes need auth? What about API routes vs page routes? Public routes? The mental model of "this middleware runs before everything and decides who gets in" is simple in theory, complex in every implementation.

### The ntnt Answer: Four Layers, Each Complete

```
Layer 0:  enable_auth([google])                           ← works, production-safe
Layer 1:  enable_auth([google], "balanced")               ← named preset, still one line
Layer 2:  enable_auth([google], map { "session_ttl": … }) ← custom config
Layer 3:  oauth_exchange, create_session_from_oauth, …    ← full manual control
```

**Layer 0 is the key insight.** With zero config, `enable_auth` should:
- Auto-detect the session store from the environment (REDIS_URL → Redis, DATABASE_URL → Postgres, else → SQLite file in DATA_DIR)
- Generate a session secret from a hash of the machine ID + app path (stable across restarts in dev, warns in prod to set a real one)
- Use sane defaults (30-day sessions, 90-day refresh, PKCE, CSRF, HttpOnly+Secure+SameSite)
- Auto-register `/auth/{provider}`, `/auth/{provider}/callback`, `/auth/logout` routes
- Print a clear startup summary of what it configured

**The developer writes 4 lines, not 15:**

```ntnt
import { oauth, enable_auth } from "std/auth"
load_env(".env")
enable_auth([oauth("google", get_env("GOOGLE_CLIENT_ID"), get_env("GOOGLE_CLIENT_SECRET"))])
listen(8080)
```

That's it. Protected routes, session management, CSRF, PKCE, token refresh, cookie security — all automatic. The `.env` file has two values: `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`. Nothing else required.

### 3.1 Zero-Config Defaults (Layer 0)
**Priority:** Critical — this is the headline feature

**What happens when you call `enable_auth` with no config map:**

| Config | Auto-detected from | Default |
|--------|-------------------|---------|
| `session_store` | `REDIS_URL` env → Redis; `DATABASE_URL` env → Postgres; else → `./data/sessions.db` (SQLite) | SQLite |
| `session_secret` | `AUTH_SESSION_SECRET` env; else → derive from machine ID + app path | Derived (warns in prod) |
| `session_ttl` | — | 30 days |
| `refresh_ttl` | — | 90 days |
| `idle_timeout` | — | 7 days |
| `sliding` | — | true |
| `cookie_secure` | `NTNT_ENV` = production → true; else → false | Auto |
| `cookie_name` | — | `"_session"` (generic, no framework fingerprinting) |
| `after_login` | — | `"/"` |
| `after_logout` | — | `"/login"` |
| `after_failure` | — | `"/login?error=auth_failed"` |
| `store_tokens` | — | true |

- [ ] Implement env-based session store auto-detection in `enable_auth`
- [ ] Implement deterministic dev secret derivation (hash of hostname + cwd + "ntnt-dev-session")
- [ ] Print startup warning when using derived secret: `[auth] ⚠ Using dev session secret. Set AUTH_SESSION_SECRET for production.`
- [ ] Change default cookie name from `"ntnt_session"` to `"_session"`
- [ ] Set `idle_timeout: 7 days` and `sliding: true` as defaults (Phase 1.2 prerequisite)

### 3.2 Auto-Route Registration
**Priority:** Critical — eliminates boilerplate files

**Problem:** Every ntnt app with auth currently needs these 3 lines:
```ntnt
get("/auth/{provider}", auth_start)
get("/auth/{provider}/callback", auth_callback)
post("/auth/logout", auth_logout)
```

These are the same in every app. They're never customized. They're boilerplate.

**Solution:** `enable_auth` registers them automatically. If the developer wants custom paths, they override:

```ntnt
// Auto-registers /auth/google, /auth/google/callback, /auth/logout
enable_auth([google])

// Custom paths (rare):
enable_auth([google], map {
    "auth_prefix": "/login/oauth",    // → /login/oauth/google, /login/oauth/google/callback
    "logout_path": "/api/logout",     // → POST /api/logout
    "auto_routes": false              // disable auto-registration entirely, wire your own
})
```

- [ ] Register OAuth routes inside `enable_auth` by default
- [ ] Configurable `auth_prefix` (default: `"/auth"`)
- [ ] Configurable `logout_path` (default: `"/auth/logout"`)
- [ ] `auto_routes: false` to opt out completely
- [ ] Detect conflict: if the app manually registers the same routes, warn instead of crash
- [ ] Print registered routes in startup log

### 3.3 Startup Config Summary
**Priority:** High — makes the invisible visible

**Problem:** After `enable_auth`, the developer has no idea what was actually configured. When sessions expire unexpectedly, there's no breadcrumb to trace back to config.

**Solution:** Always print a startup summary. Not behind a flag — always:

```
┌─ Auth ─────────────────────────────────────┐
│ Provider:  google (PKCE ✓)                 │
│ Sessions:  Redis (redis://localhost:6379)   │
│ TTL:       30d session, 7d idle (sliding)   │
│ Refresh:   90d (auto-refresh ✓)            │
│ Cookie:    _session (Secure, HttpOnly, Lax) │
│ CSRF:      enabled                          │
│ Routes:    /auth/google → login             │
│            /auth/google/callback            │
│            POST /auth/logout                │
├─ Warnings ─────────────────────────────────┤
│ ⚠ Using dev session secret                 │
│   → Set AUTH_SESSION_SECRET in production   │
└────────────────────────────────────────────┘
```

In production (no warnings):
```
[auth] ✓ google (PKCE) | Redis | 30d/7d sliding | Secure
```

This is the first thing a developer sees after `enable_auth`. No mystery. No black box.

- [ ] Build the summary printer in `init_auth`
- [ ] Dev mode: boxed format with full details + warnings
- [ ] Production mode: single-line compact format
- [ ] Include every config value that was set (explicit or default)
- [ ] Flag any value that's using a default with a subtle marker

### 3.4 Typed Config Validation with Actionable Errors
**Priority:** High — would have caught the `session_ttl: 3600` staging incident

**Problem:** Passing wrong types to `enable_auth` config map fails silently (falls back to defaults). Passing valid types with suspicious values also fails silently.

**Solution:** Two layers of validation:

**Type validation:**
```
[auth] ✗ session_ttl: expected Int, got String "3600" — using default 2592000
        Fix: "session_ttl": 3600  (without quotes)
```

**Range validation (warnings, not errors):**
```
[auth] ⚠ session_ttl: 3600 (1 hour) is unusually short.
        Common values: 86400 (1 day), 2592000 (30 days), 7776000 (90 days)

[auth] ⚠ session_ttl: 31536000 (365 days) exceeds recommended maximum.
        OWASP recommends absolute session lifetime ≤ 90 days for sensitive apps.
```

**Consistency validation:**
```
[auth] ⚠ refresh_ttl (7 days) is shorter than session_ttl (30 days).
        Refresh tokens can't extend sessions past the session TTL.
        Fix: Set refresh_ttl > session_ttl, or disable refresh (store_tokens: false)
```

- [ ] Add type validation for every config field in `enable_auth` (Int, String, Bool, URL)
- [ ] Add range validation: suspiciously short (<5 min), suspiciously long (>365 days)
- [ ] Add consistency validation: refresh_ttl vs session_ttl, idle_timeout vs session_ttl
- [ ] Log clear fix suggestions with example values
- [ ] Never error on valid types with unusual values — warn only (the developer may know what they're doing)
- [ ] In production: suppress range/consistency warnings (they've been seen in dev)

### 3.5 Auth Health Check Endpoint
**Priority:** Medium

**Problem:** Developers can't verify auth config is correct without logging in and waiting for something to break.

**Solution:** Add `auth_health` handler — automatically registered in dev mode:

```
GET /auth/health  (dev mode only, auto-registered)
```

Returns:
```json
{
    "status": "healthy",
    "providers": [{"name": "google", "pkce": true, "oidc": true}],
    "session_store": {"type": "redis", "connected": true, "active_sessions": 42},
    "config": {
        "session_ttl": 2592000,
        "refresh_ttl": 7776000,
        "idle_timeout": 604800,
        "sliding": true,
        "cookie_secure": false,
        "csrf_enabled": true
    },
    "warnings": ["Using dev session secret"],
    "test_login_url": "/auth/google"
}
```

- [ ] Auto-register `/auth/health` GET route in dev mode
- [ ] Include session store connectivity check
- [ ] Include active session count
- [ ] Include full resolved config (so you can see what defaults were applied)
- [ ] Include `test_login_url` for convenience
- [ ] Blocked in production (`NTNT_ENV=production` returns 404)

### 3.6 `enable_auth` Presets
**Priority:** Medium — sugar, not substance, but excellent DX

**Problem:** Even with good defaults, some apps want "just make it more strict" or "just make it more relaxed" without researching what TTL values mean.

**Solution:** Named presets as the second argument:

```ntnt
enable_auth([google])                         // defaults = "balanced"
enable_auth([google], "strict")               // banking app
enable_auth([google], "relaxed")              // personal dashboard
enable_auth([google], map { ... })            // full custom
enable_auth([google], map {                   // preset + override
    "preset": "strict",
    "after_login": "/dashboard"
})
```

| Preset | session_ttl | idle_timeout | sliding | max_lifetime | refresh | description |
|--------|-------------|--------------|---------|--------------|---------|-------------|
| `"strict"` | 1 hour | 15 min | true | 24 hours | off | Banking, healthcare, anything PII-heavy |
| `"balanced"` | 30 days | 7 days | true | 90 days | on | Most web apps (the default) |
| `"relaxed"` | 90 days | 30 days | true | 365 days | on | Personal tools, dashboards |

**Presets are transparent:** choosing a preset prints every value it sets in the startup summary, so the developer always knows what they got. No magic.

- [ ] Define preset configurations as const maps
- [ ] Accept String or Map as second arg to `enable_auth`
- [ ] `"preset"` key in map applies preset first, then overrides
- [ ] Startup summary shows `[preset: balanced]` or `[custom]`
- [ ] Document all presets with use case guidance

### 3.7 Protected Route Pattern — `require_auth` Middleware
**Priority:** High — simplifies the most common auth question: "which routes need auth?"

**Problem:** The current middleware (`01_auth.tnt`) is custom per-app. Every app re-implements the same logic: check for session, redirect to login, allow public paths. The middleware file is typically 50+ lines of boilerplate that does the exact same thing.

**Solution:** Built-in `require_auth` middleware with route patterns:

```ntnt
import { require_auth } from "std/auth"

// Protect everything except explicitly public routes
use_middleware(require_auth(map {
    "public": ["/", "/login", "/about", "/api/health"],
    "login_redirect": "/login"
}))
```

Or invert — protect specific routes:
```ntnt
use_middleware(require_auth(map {
    "protected": ["/admin/*", "/api/*", "/settings"],
    "public_api": ["/api/health", "/api/status"],  // API routes return 401, not redirect
    "login_redirect": "/login"
}))
```

**Behavior:**
- Browser requests (Accept: text/html) → redirect to `login_redirect`
- API requests (Accept: application/json, or /api/* routes) → return `401 {"error": "Authentication required"}`
- Glob patterns: `"/admin/*"` matches `/admin/anything`
- Auth routes (`/auth/*`) are always public (never require auth to log in)

**This replaces the custom middleware file in most apps.** For apps that need custom auth logic, `require_auth` is just a function they don't call — they write their own middleware as before.

- [ ] Implement `require_auth(config)` as a stdlib middleware generator
- [ ] Support `public` (allowlist) and `protected` (denylist) patterns
- [ ] Glob matching for route patterns
- [ ] Smart redirect vs 401 based on Accept header
- [ ] Always exempt `/auth/*` and health check routes
- [ ] Return the middleware function (compatible with `use_middleware`)

### 3.8 Login Page Generator
**Priority:** Low-Medium — saves time but not critical

**Problem:** Every app needs a login page. Most login pages are identical: centered card, "Sign in with Google" button, maybe an error message. Developers copy-paste this from examples or build it from scratch.

**Solution:** Built-in login page that works out of the box:

```ntnt
import { login_page } from "std/auth"

get("/login", login_page)
```

Renders a clean, minimal login page with buttons for each configured provider. Handles error display (`?error=auth_failed`). Responsive. No external dependencies.

**Customizable via config:**
```ntnt
get("/login", login_page(map {
    "title": "Welcome to MyApp",
    "logo": "/static/logo.png",
    "background": "#f5f5f5",
    "theme": "dark"
}))
```

For apps that want full control: don't use `login_page`. Build your own HTML, link to `/auth/google`. Zero lock-in.

- [ ] Implement `login_page` handler with built-in HTML template
- [ ] Auto-detect configured providers and render buttons for each
- [ ] Handle `?error=` query param for error display
- [ ] Accept optional config map for branding (title, logo, colors, theme)
- [ ] Responsive, accessible, no external CSS/JS dependencies
- [ ] Include CSRF token in any form elements

### Putting It Together: The Full Competitive Comparison

**Auth.js v5 (Next.js) — minimum viable auth:**
```
1. npm install next-auth @auth/core
2. Create auth.ts (config + providers + adapter)
3. Create app/api/auth/[...nextauth]/route.ts (route handler)
4. Create middleware.ts (protected routes)
5. Add NEXTAUTH_SECRET to .env
6. Add provider credentials to .env
7. Set up database adapter (Prisma/Drizzle) if you want sessions
Files: 4 new files + .env changes. ~60-80 lines of config code.
```

**Lucia v3 — minimum viable auth:**
```
1. npm install lucia @lucia-auth/adapter-*
2. Create database schema (users + sessions tables)
3. Run migration
4. Create lib/auth.ts (Lucia instance + adapter)
5. Create login route handler
6. Create callback route handler  
7. Create middleware for session validation
8. Create logout handler
Files: 5-7 new files + migration. ~100-150 lines of auth code.
```

**Laravel Sanctum — minimum viable auth:**
```
1. composer require laravel/sanctum
2. php artisan vendor:publish --provider="Laravel\Sanctum\SanctumServiceProvider"
3. php artisan migrate
4. Add Sanctum middleware to api middleware group
5. Configure CORS for SPA
6. Add Socialite for OAuth (separate package)
Files: config changes across 3-4 files + migration. ~40-60 lines.
```

**ntnt today (v0.4.6):**
```
1. Add 2 env vars to .env (GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)
2. Add ~15 lines to server.tnt (imports, oauth, enable_auth, route registration)
3. Create middleware/01_auth.tnt (~50 lines of route protection)
4. Create login page view
Files: 2 files modified, 1 new middleware, 1 new view. ~70 lines.
```

**ntnt after Phase 3:**
```
1. Add 2 env vars to .env (GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)
2. Add 4 lines to server.tnt:
   import { oauth, enable_auth, require_auth, login_page } from "std/auth"
   enable_auth([oauth("google", get_env("GOOGLE_CLIENT_ID"), get_env("GOOGLE_CLIENT_SECRET"))])
   use_middleware(require_auth(map { "public": ["/", "/login"] }))
   get("/login", login_page)
Files: 1 file modified, 0 new files. 4 lines of auth code.
```

**That's the 3x improvement.** From ~70 lines across 4 files → 4 lines in 1 file. Zero boilerplate files. Zero middleware files. Zero custom login page HTML. And every security feature is still there — PKCE, CSRF, signed cookies, auto-refresh — just automatic.

**The escape hatch is always there:** any of these automatic behaviors can be overridden by passing config, or disabled entirely with `auto_routes: false`. You can always go back to "I'll wire everything manually." But you shouldn't have to.

---

## Phase 4: Advanced Session Management

**Goal:** Features that put ntnt auth ahead of every other stdlib.

**Estimated effort:** Large (3-4 PRs)

### 4.1 Device-Aware Sessions
**Priority:** High

**Problem:** Users can't see or manage where they're logged in. No "your active sessions" page like Google/GitHub.

**Current state:** We already have `user_sessions(req)` and `revoke_session(req, session_id)`. But sessions don't store device metadata.

**Solution:** Capture and store device context at session creation:

```ntnt
let sessions = user_sessions(req)
// Returns: [
//   { session_id: "abc...", device: "Chrome on macOS", ip: "73.x.x.x",
//     last_active: 1773967904, created: 1773967000, current: true },
//   { session_id: "def...", device: "Safari on iPhone", ip: "73.x.x.x",
//     last_active: 1773960000, created: 1773900000, current: false }
// ]
```

**Implementation:**
- [ ] Parse `User-Agent` at session creation → store simplified device string
- [ ] Store IP at creation (already have `req.ip`)
- [ ] Add `device`, `ip`, `last_ip` fields to Session
- [ ] Update `user_sessions` response to include device metadata
- [ ] Add `last_active_at` tracking (Phase 1.2 prerequisite)
- [ ] Update `last_ip` on each request (with debounce)

### 4.2 Session Revocation Cascade
**Priority:** Medium

**Problem:** When a user changes their password or suspects compromise, they need to revoke ALL other sessions instantly.

**Solution:**
```ntnt
let count = revoke_all_sessions(req)            // kill all except current
let count = revoke_all_sessions(req, true)      // kill ALL including current (force re-auth)
```

- [ ] Add `revoke_all_sessions(req, include_current?)` function
- [ ] Efficiently delete by user_id across all backends
- [ ] Return count of revoked sessions
- [ ] Log security event

### 4.3 Suspicious Activity Detection
**Priority:** Medium

**Problem:** No visibility into potentially compromised sessions.

**Solution:** Track and flag suspicious patterns:
- Same session used from two very different IPs simultaneously
- Session used from a different country than creation
- Rapid session creation (brute force attempt)

```ntnt
enable_auth([google], map {
    "security_events": true,  // enable event logging
    "ip_change_action": "warn"  // or "revoke" — what to do on IP change
})
```

- [ ] Add `security_events` table/key for logging auth events
- [ ] Track IP changes per session
- [ ] Configurable actions: `"warn"` (log), `"challenge"` (force re-auth), `"revoke"`
- [ ] Expose via `auth_events(user_id, limit?)` for admin dashboards
- [ ] Rate limit failed OAuth callbacks (prevent brute force)

### 4.4 Remember Me / Persistent vs. Session Cookies
**Priority:** Low

**Problem:** Some apps want short-lived sessions by default but offer a "remember me" option for longer persistence.

**Solution:**
```ntnt
enable_auth([google], map {
    "remember_me": true,           // enable the feature
    "remember_ttl": 86400 * 90,    // "remember me" sessions last 90 days
    "session_ttl": 86400           // default sessions last 1 day
})
```

The login page includes a checkbox. `auth_start` reads a query param (`?remember=1`) and stores it in OAuth state. On callback, session TTL is set accordingly.

- [ ] Add `remember_me`, `remember_ttl` to AuthConfig
- [ ] Pass `remember` flag through OAuth state
- [ ] Set appropriate TTL on session creation based on flag
- [ ] Cookie: session cookie (no Max-Age) for short, persistent cookie for remember

---

## Phase 5: Validate and Remove Speculative Code

**Goal:** Prove or disprove the Safari ITP fix with real data.

**Estimated effort:** Small (1 PR if removing)

### 5.1 Safari ITP A/B Test
**Priority:** High (do this NOW — before investing in more features)

**Problem:** We shipped the two-phase exchange token flow for Safari ITP, but then discovered the actual issue was a config mismatch (`session_ttl: 3600` on staging). We haven't proven ITP is a real problem for our use case.

**Test plan:**
1. ✅ Test A: Log in on Safari with ITP fix + correct 30-day TTL → check persistence after 24h
2. Test B: Deploy staging with direct cookie-on-redirect (revert ITP fix), same TTL → check persistence after 24h
3. If both persist → remove ITP fix (saves ~400 lines of complexity)
4. If only A persists → ITP fix is validated, it stays

- [ ] Josh: Test A login on Safari (done 2026-03-20)
- [ ] Check session persistence after 24h
- [ ] If needed: deploy Test B build
- [ ] Decision: keep or remove ITP code based on data

### 5.2 Complexity Budget
Every feature in this doc adds code to `auth.rs` (already ~7000 lines). We should track lines of code and consider splitting into sub-modules when it gets unwieldy:

| Module | Scope |
|--------|-------|
| `auth/config.rs` | AuthConfig, presets, validation |
| `auth/session.rs` | Session CRUD, rotation, sliding |
| `auth/oauth.rs` | OAuth flows, PKCE, state management |
| `auth/providers.rs` | Built-in provider configs, OIDC discovery |
| `auth/security.rs` | Rate limiting, suspicious activity, events |
| `auth/handlers.rs` | auth_start, auth_callback, auth_logout, etc. |

This isn't necessary now, but should happen before Phase 4.

---

## What We Explicitly Won't Do

These are features that other auth systems offer but don't belong in a language stdlib:

- **User management UI** — That's the app's job, not the language's
- **Email/password signup flows** — We provide `hash_password`/`verify_password` + session management; the signup form and email verification is app logic
- **Social login buttons/UI** — We handle the OAuth protocol; the "Sign in with Google" button is app HTML
- **Multi-tenancy** — Session isolation between tenants is app-level routing
- **Billing/subscription gating** — Not auth, not our problem
- **SMS/email OTP delivery** — We provide TOTP verification; delivery channels are app-specific

---

## Implementation Order

| Order | Phase | Feature | Why This Order |
|-------|-------|---------|----------------|
| **0** | 5.1 | Safari ITP A/B test | Must validate before building more |
| **1** | 3.1 + 3.2 + 3.3 | Zero-config defaults + auto-routes + startup summary | The headline DX improvement — changes the first impression |
| **2** | 3.4 | Typed config validation | Prevents misconfig (like the 3600 incident) |
| **3** | 3.7 | `require_auth` middleware | Eliminates the biggest boilerplate file |
| **4** | 1.1 | Session rotation on auth | Critical security gap |
| **5** | 1.2 + 1.3 | Sliding sessions + absolute cap | Prerequisite for presets |
| **6** | 3.6 | Presets | Builds on sliding sessions, gives the "strict/balanced/relaxed" UX |
| **7** | 2.1 | Refresh token rotation | RFC 9700 compliance |
| **8** | 3.5 | Auth health check endpoint | DX, catches issues early |
| **9** | 3.8 | Login page generator | Nice DX, low effort |
| **10** | 4.1 | Device-aware sessions | Differentiator |
| **11** | 4.2 | Session revocation cascade | Natural follow-on to 4.1 |
| **12** | 4.3 | Suspicious activity | Advanced security |
| **13** | 4.4 | Remember me | Nice-to-have |
| **14** | 5.2 | Module split | Housekeeping when needed |

---

## Competitive Landscape

| Feature | ntnt (today) | ntnt (this DD) | Auth.js v5 | Lucia v3 | Laravel Sanctum | Django Auth |
|---------|:---:|:---:|:---:|:---:|:---:|:---:|
| OAuth/OIDC | ✅ | ✅ | ✅ | ❌¹ | ❌² | ❌² |
| PKCE | ✅ | ✅ | ✅ | ❌¹ | ❌ | ❌ |
| Server sessions | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Signed cookies | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ |
| CSRF auto-protection | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| Token auto-refresh | ✅ | ✅ | ✅ | ❌¹ | ❌ | ❌ |
| Session rotation | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ |
| Sliding sessions | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Refresh token rotation | ❌ | ✅ | ✅ | ❌¹ | ❌ | ❌ |
| Device-aware sessions | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Config presets | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Health check endpoint | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Suspicious activity | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| TOTP/MFA | ✅ | ✅ | ❌ | ❌¹ | ❌ | ❌³ |
| ITP workaround | ✅ | ✅⁴ | ❌ | ❌ | ❌ | ❌ |
| **Zero-config setup** | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| **Auto route registration** | ❌ | ✅ | ✅⁵ | ❌ | ❌ | ✅ |
| **Startup config summary** | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| **Config validation** | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| **Built-in route protection** | ❌ | ✅ | ✅⁶ | ❌ | ✅ | ✅ |
| **Built-in login page** | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ |
| **0 boilerplate files** | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| One-line setup | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |

¹ Lucia is sessions-only — OAuth/tokens/MFA are app-level  
² Requires additional packages (Socialite, django-allauth)  
³ Requires django-otp  
⁴ Pending A/B validation — may be removed if unnecessary  
⁵ Auth.js uses file-system convention (`[...nextauth]/route.ts`) — auto but requires specific file structure  
⁶ Auth.js middleware.ts works but is a separate file with its own config — not integrated into the auth call  

---

## Success Criteria

We'll know this is "the best auth system ever made for any language" when:

1. **Zero-config security:** `enable_auth([google])` with no options map is already production-safe
2. **No CVE surface:** Every OWASP session management recommendation is implemented by default
3. **RFC 9700 compliant:** Full OAuth security BCP compliance
4. **Observable:** Developers can see exactly what auth is doing and whether it's configured correctly
5. **Competitive table is all green:** Every feature other systems offer, plus device sessions, presets, health checks, and suspicious activity that nobody else has in a stdlib
6. **< 30 seconds to auth:** From `ntnt new` to authenticated app in under 30 seconds of developer time

---

*This is a living document. Update checkboxes as implementation progresses.*
