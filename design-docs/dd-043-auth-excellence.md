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

## Phase 3: Developer Experience

**Goal:** Make auth configuration foolproof and observable.

**Estimated effort:** Small-Medium (2 PRs)

### 3.1 Auth Health Check Endpoint
**Priority:** High

**Problem:** Developers can't easily verify their auth configuration is correct and secure.

**Solution:** Add `auth_health` handler that returns a JSON assessment:

```ntnt
get("/auth/health", auth_health)
```

Returns (only in dev mode):
```json
{
    "status": "healthy",
    "providers": ["google"],
    "session_store": "redis",
    "cookie_secure": true,
    "csrf_enabled": true,
    "pkce_enabled": true,
    "session_ttl": 2592000,
    "refresh_enabled": true,
    "sliding_sessions": false,
    "active_sessions": 42,
    "warnings": [
        "session_secret is using dev default — set a secure random secret for production"
    ]
}
```

- [ ] Add `auth_health` handler function
- [ ] Only enabled when `NTNT_ENV != production` (security: don't expose config in prod)
- [ ] Check for common misconfigurations and emit warnings
- [ ] Include session store health (can connect to Redis/PG?)

### 3.2 Security Headers Audit
**Priority:** Medium

**Problem:** Apps might misconfigure security headers that affect auth (missing HSTS, bad CSP, etc.).

**Solution:** Add warnings to `auth_health` for missing/misconfigured headers:
- `Strict-Transport-Security` not set → warn (cookies can be intercepted)
- `Content-Security-Policy` too permissive → note
- `X-Frame-Options` missing → warn (clickjacking on login page)

- [ ] Scan response headers in health check
- [ ] Emit actionable warnings with fix suggestions

### 3.3 Typed Config Validation
**Priority:** Medium

**Problem:** Passing wrong types to `enable_auth` config map fails silently (falls back to defaults). A developer who writes `"session_ttl": "3600"` (string instead of int) gets the default 1-week TTL with no warning.

**Solution:** Validate and warn on type mismatches during `enable_auth`:

```
[auth] Warning: session_ttl should be Int, got String "3600" — using default 604800
```

- [ ] Add type validation for all config fields in `enable_auth`
- [ ] Log warnings for type mismatches (don't error — maintain backward compat)
- [ ] Suggest fixes in warning messages
- [ ] Validate value ranges (e.g., `session_ttl < 60` → "Session TTL under 60 seconds is unusually short")

### 3.4 `enable_auth` Presets
**Priority:** Low

**Problem:** Developers have to manually configure session TTLs, refresh TTLs, sliding windows, etc. Easy to get wrong.

**Solution:** Named presets for common patterns:

```ntnt
enable_auth([google], "strict")      // short sessions, no sliding, frequent re-auth
enable_auth([google], "balanced")    // 30-day sessions, 7-day idle, sliding
enable_auth([google], "relaxed")     // 90-day sessions, 30-day idle, sliding
enable_auth([google], map { ... })   // full custom (as today)
```

| Preset | session_ttl | idle_timeout | sliding | max_lifetime | refresh |
|--------|-------------|--------------|---------|--------------|---------|
| `strict` | 1 hour | 15 min | true | 24 hours | off |
| `balanced` | 30 days | 7 days | true | 90 days | on |
| `relaxed` | 90 days | 30 days | true | 365 days | on |

- [ ] Define preset configurations
- [ ] Accept string as second arg to `enable_auth` (detect String vs Map)
- [ ] Allow preset + overrides: `enable_auth([google], map { "preset": "balanced", "session_ttl": 86400 * 60 })`

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
| **1** | 1.1 | Session rotation on auth | Critical security gap |
| **2** | 1.2 + 1.3 | Sliding sessions + absolute cap | Most-requested session feature |
| **3** | 2.1 | Refresh token rotation | RFC 9700 compliance |
| **4** | 3.3 | Typed config validation | Prevents misconfig (like the 3600 incident) |
| **5** | 3.1 | Auth health check | DX, catches issues early |
| **6** | 4.1 | Device-aware sessions | Differentiator |
| **7** | 4.2 | Session revocation cascade | Natural follow-on to 4.1 |
| **8** | 3.4 | Presets | Polish |
| **9** | 4.3 | Suspicious activity | Advanced security |
| **10** | 4.4 | Remember me | Nice-to-have |
| **11** | 5.2 | Module split | Housekeeping when needed |

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
| One-line setup | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |

¹ Lucia is sessions-only — OAuth/tokens/MFA are app-level  
² Requires additional packages (Socialite, django-allauth)  
³ Requires django-otp  
⁴ Pending A/B validation — may be removed if unnecessary  

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
