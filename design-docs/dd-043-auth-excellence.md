# DD-043: World-Class Auth — Making `std/auth` the Best stdlib Auth Ever

**Status:** In Progress  
**Author:** Larri  
**Date:** 2026-03-20  
**Branch:** `feat/auth-challenges-v0.4.9` (Phase 2 merged via PR #77, Phase 3 merged via PR #78)

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

## Unified Phased Implementation Plan

This is the single source of truth for how we should build `std/auth` forward. The phases below are the ordered implementation plan, and each phase contains the exact capabilities we want to ship and track with checkboxes.

### Phase 0 — Validate Safari ITP Complexity Before Building More
**Goal:** Prove or kill the speculative Safari ITP workaround before we build more behavior on top of it.

- [x] Josh: Test A login on Safari with ITP fix + correct 30-day TTL
- [x] Check session persistence after 24h
- [x] If needed, deploy Test B build with direct cookie-on-redirect
- [x] Decide whether the ITP workaround stays or gets removed

**Result:** Verified. Phase 0 is complete, and this no longer blocks forward auth work.

**Why first:** if the workaround is unnecessary, we should remove that complexity before layering more session behavior on top.

### Phase 1 — Foundation Mini-Phase
**Goal:** Establish safe defaults and clear diagnostics without spending too long on lower-leverage convenience work.

**Result:** Complete. This foundation pass was already shipped and should have been marked done earlier.

- [x] Production-safe default auth config in `enable_auth`
- [x] Startup summary showing providers, routes, session settings, and cookie posture
- [x] Typed config validation with actionable errors and typo suggestions
- [x] Fatal errors for required missing config, warnings for optional config

**Ships real value:** apps get a stable auth floor and much better debugging immediately.

### Phase 2 — Route Protection for Real Apps
**Goal:** Make route protection trivial, especially for file-routed apps like the template.

**Result:** Merged in PR #77 (`feat: add auth route protection helpers`) on 2026-04-12.

- [x] `require_auth()` middleware helper
- [x] Configurable protected path patterns
- [x] Accept shorthand like `require_auth("/admin/*")`
- [x] Support exact path + subtree matching (`/admin` and `/admin/*`)
- [x] Work cleanly with file-routed apps, not just hand-written route tables
- [x] Smart redirect vs `401`/`403` based on HTML vs API requests
- [x] Always exempt `/auth/*` and auth health/debug routes
- [x] Expose authenticated session state through helpers instead of ad hoc cookie parsing in handlers

**Ships real value:** apps stop repeating auth checks in every protected page.

### Phase 3 — Session Core and Cookie Helpers
**Goal:** Remove the most repetitive and error-prone login/logout/session code.

**Result:** Merged in PR #78 (`feat: add auth session helpers`) on 2026-04-13.

- [x] Rotate session ID automatically on successful auth callback/login
- [x] Invalidate old session ID immediately after rotation
- [x] Preserve intended session data across rotation
- [x] Session store migration implemented in Rust for Redis/Postgres/SQLite backends
- [x] Keep `migrate_session(old_id, new_id)` internal to Rust/session-store code
- [x] Public ntnt API exposes only high-level session helpers, not low-level store migration primitives (`sign_in_session`, `rotate_session`, `sign_out_session`, `current_session`, `current_user`)
- [x] `sign_in_session()` helper that persists session + attaches cookie
- [x] `sign_out_session()` helper that revokes session + clears cookie
- [x] `current_session()` / `current_user()` helper for request-time lookups
- [x] Shared auth cookie defaults with per-app overrides
- [x] Centralize cookie posture: name, path, same-site, secure, httpOnly, expiry
- [x] Defer session rotation event logging until auth security logging exists, tracked in Phase 6

**Possible public API shape:**

```ntnt
let resp = redirect("/admin")
return sign_in_session(resp, map {
    "subject_id": user["id"],
    "claims": map { "role": "admin" }
})

let session = current_session(req)
let user = current_user(req)
return sign_out_session(redirect("/login"), req)
```

**Ships real value:** the template’s verbose login/logout/cookie handling mostly disappears here.

### Phase 4 — Staged Auth Primitives
**Goal:** Make multi-step auth flows first-class instead of hand-rolled.

**Status:** In progress on `feat/auth-challenges-v0.4.9`.

**Initial implementation slice:** ship a distinct pending-auth challenge store and the four core helpers, with one-time completion semantics and session upgrade into the existing Phase 3 helpers. Keep challenge state minimal and isolated from protected-route access.

- [x] Distinct pending-auth primitive separate from full authenticated sessions
- [x] `begin_auth_challenge()` helper
- [x] `current_auth_challenge()` helper
- [x] `complete_auth_challenge()` helper
- [x] `cancel_auth_challenge()` helper
- [x] Explicit TTL and one-time-use semantics for challenges
- [x] Safe place for minimal staged-auth metadata
- [x] No protected-route access until challenge upgrades into a real session
- [ ] Works for password → TOTP verification
- [ ] Works for first login → TOTP setup → forced password change
- [ ] Works for future MFA and step-up auth flows

**Possible public API shape:**

```ntnt
let resp = redirect("/admin/verify")
return begin_auth_challenge(resp, map {
    "subject_id": user["id"],
    "kind": "mfa_pending",
    "ttl": 1800,
    "data": map { "next": "/admin" }
})

let challenge = current_auth_challenge(req)
let resp = redirect("/admin")
return complete_auth_challenge(resp, req, map {
    "claims": map { "role": "admin" }
})
```

**Ships real value:** the template’s password/TOTP/password-change flow gets much cleaner and more robust.

### Phase 4.5 — Auth Storage + Architecture Cleanup
**Goal:** Pay down the technical debt introduced while shipping the Phase 4 primitives, before more auth features stack on top of the current structure.

**Why now:** Phase 4 delivered the right primitives, but it also increased backend-specific branching, fallback-path nuance, and pressure on the already-large `src/stdlib/auth.rs`. This cleanup phase should happen before additional auth lifecycle and observability work, so we improve the structure while the surface area is still relatively contained.

- [ ] Extract auth storage operations behind a clearer internal abstraction so session, auth-challenge, OAuth-state, and exchange-token backend behavior is easier to keep consistent across SQLite/Postgres/Redis
- [ ] Split `src/stdlib/auth.rs` into smaller focused modules without changing the public `std/auth` API surface
- [ ] Add env-gated Redis/Postgres integration coverage for auth storage flows so backend-specific regressions are caught earlier
- [ ] Centralize and document fallback/error semantics so auth reads, consumes, and cleanups follow one intentional policy instead of drifting per code path
- [ ] Re-review Phase 3 and Phase 4 helpers after the refactor to ensure docs, tests, and public behavior still match exactly

**Non-goal:** this phase is not for adding new end-user auth features. It is specifically for making the current auth foundation cleaner, more testable, and less likely to accumulate technical debt as we continue.

**Exit criteria:** we should be able to explain auth persistence behavior in one coherent model, add a backend without copy-pasting half the file, and review future auth PRs without re-litigating fallback semantics in every thread.

### Phase 5 — Session Lifecycle and Presets
**Goal:** Harden session lifetime behavior and make strong defaults ergonomic.

- [ ] Add sliding session expiry support
- [ ] Throttle refreshes so stores are not updated on every request
- [ ] Refresh cookie Max-Age/Expires along with sliding sessions
- [ ] Add absolute maximum session lifetime support
- [ ] Store session creation time separately from idle expiry
- [ ] Clear log/error path when max lifetime forces re-auth
- [ ] Define preset configurations (`consumer`, `admin`, `internal`, `strict`)
- [ ] Accept String or Map as second arg to `enable_auth`
- [ ] Allow preset + override merge
- [ ] Document preset guidance clearly

**Ships real value:** apps get secure, reusable lifecycle behavior without inventing timeout policy from scratch.

### Phase 6 — OAuth Hardening and Observability
**Goal:** Improve security posture and make auth easier to inspect in production.

- [ ] Refresh token rotation when providers issue a new refresh token
- [ ] Invalidate old refresh token references where feasible
- [ ] Log refresh token rotation events
- [ ] Document provider differences clearly
- [ ] `/auth/health` endpoint (dev-only by default)
- [ ] Health output shows config state without leaking secrets
- [ ] Diagnose common issues like redirect mismatch, missing env, wrong store

**Ships real value:** better production safety and far easier troubleshooting.

### Phase 7 — Auto-Routes and Convenience UI
**Goal:** Make the easy path extremely easy without forcing apps into one UI.

- [ ] Auto-generate standard auth routes if not overridden
- [ ] Configurable path prefixes (`/auth/*` by default)
- [ ] Clear startup log showing registered routes
- [ ] Path collision detection with app-defined routes
- [ ] Built-in login page template
- [ ] Configurable title/logo/copy
- [ ] Provider buttons generated automatically
- [ ] Support custom HTML override if needed

**Ships real value:** simple apps get auth with almost no wiring, while custom apps still own their design.

### Phase 8 — Advanced Sessions and Security Signals
**Goal:** Add higher-end capabilities once the core mechanics are excellent.

- [ ] Store user agent hash / device name on session creation
- [ ] `list_sessions(user_id)` API
- [ ] `revoke_session(session_id)` API
- [ ] `revoke_all_sessions(user_id)` API
- [ ] Password change hook support for revocation
- [ ] Account disable hook support for revocation
- [ ] Optional `revoke_on_password_change: true` config
- [ ] Add `security_events` storage for auth events
- [ ] Track IP changes per session
- [ ] Configurable suspicious-activity actions: `warn`, `challenge`, `revoke`
- [ ] Expose `auth_events(user_id, limit?)` for admin dashboards
- [ ] Rate limit failed OAuth callbacks
- [ ] Add `remember_me` and `remember_ttl`
- [ ] Pass remember-me flag through OAuth state
- [ ] Set appropriate TTL/persistence on session creation

**Ships real value:** this is where `std/auth` becomes genuinely standout, not just competent.

### Phase 9 — Internal Cleanup and Complexity Budget
**Goal:** Keep the implementation maintainable as the feature set grows.

- [ ] Track auth code size / complexity budget as features land
- [ ] Split internals into modules when the public API settles:
  - [ ] `auth/config.rs`
  - [ ] `auth/session.rs`
  - [ ] `auth/oauth.rs`
  - [ ] `auth/providers.rs`
  - [ ] `auth/security.rs`
  - [ ] `auth/handlers.rs`

**Ships real value:** keeps `std/auth` maintainable without delaying earlier wins.

## What We Explicitly Won't Do

These are features that other auth systems offer but don't belong in a language stdlib:

- **User management UI** — That's the app's job, not the language's
- **Email/password signup flows** — We provide `hash_password`/`verify_password` + session management; the signup form and email verification is app logic
- **Social login buttons/UI** — We handle the OAuth protocol; the "Sign in with Google" button is app HTML
- **Multi-tenancy** — Session isolation between tenants is app-level routing
- **Billing/subscription gating** — Not auth, not our problem
- **SMS/email OTP delivery** — We provide TOTP verification; delivery channels are app-specific

---

## Delivery Order

The best delivery order is not just "security first" or "DX first". It should deliver a complete slice each time, where every wave leaves `std/auth` materially more useful in real apps.

For template cleanup specifically, the biggest wins are:
1. section-wide route protection
2. session/cookie helpers
3. staged-auth challenge primitives

So the plan should put those cleanup-heavy features first, but still start with a very thin foundation pass so the rest of the work lands on stable defaults and validation.

| Order | Delivery Wave | Feature Bundle | Why This Order |
|-------|---------------|----------------|----------------|
| **0** | Validation | 5.1 Safari ITP A/B test | Validate the constraint before optimizing around it |
| **0.5** | Foundation Mini-Wave | 3.1 + 3.3 + 3.4 zero-config defaults, startup summary, typed config validation | Thin safety pass before the cleanup-heavy auth primitives |
| **1** | Route Protection | 3.7 + 3.7.1 `require_auth` plus section/file-route protection | Biggest immediate cleanup win for file-routed apps and templates |
| **2** | Session Core | 1.1 + 3.7.2 session rotation, sign-in/sign-out/current-session helpers | Removes the most repetitive login/logout/cookie boilerplate |
| **3** | Staged Auth | 3.7.3 challenge primitives for pending auth states | Unlocks password → TOTP, first-login setup, and step-up auth cleanly |
| **4** | Session Lifecycle | 1.2 + 1.3 + 3.6 sliding sessions, max lifetime, presets | Hardens lifecycle rules after the core mechanics are in place |
| **5** | OAuth Hardening + Observability | 2.1 + 3.5 refresh token rotation and auth health check | Tightens security posture and makes behavior inspectable |
| **6** | Auto-Routes + UI Convenience | 3.2 + 3.8 auto-routes and login page generator | Helpful DX layer after the mechanics are already excellent |
| **7** | Advanced Sessions | 4.1 + 4.2 + 4.3 + 4.4 device sessions, cascade revoke, suspicious activity, remember me | Differentiators layered onto a solid core |
| **8** | Internal Cleanup | 5.2 module split | Do once the public API and architecture are proven |

### Delivery Wave Deliverables

#### Wave 0.5 — Foundation Mini-Wave
**Ships:**
- production-safe default auth config
- startup summary showing active auth config
- typed config validation with actionable errors

**Strong value:** creates a stable floor for the cleanup-heavy phases without spending a whole early phase on lower-leverage convenience work.

#### Wave 1 — Route Protection
**Ships:**
- `require_auth()` middleware
- path/subtree protection for file-routed apps
- HTML redirect vs API `401` behavior
- section-wide protection for things like `routes/admin/*`

**Strong value:** apps stop re-implementing auth checks in every page handler.

#### Wave 2 — Session Core
**Ships:**
- session ID rotation on successful auth
- `sign_in_session()`
- `sign_out_session()`
- `current_session()` / `current_user()`
- shared cookie defaults and overrides

**Strong value:** the most repetitive and mistake-prone login/logout/cookie code disappears.

#### Wave 3 — Staged Auth
**Ships:**
- `begin_auth_challenge()`
- `current_auth_challenge()`
- `complete_auth_challenge()`
- `cancel_auth_challenge()`
- one-time, TTL-bound pending auth state distinct from real sessions

**Strong value:** multi-step auth becomes a normal pattern instead of ad hoc glue code.

#### Wave 4 — Session Lifecycle
**Ships:**
- sliding expiration
- absolute max lifetime
- ergonomic presets (`strict`, `balanced`, `relaxed`, etc.)

**Strong value:** apps get secure lifecycle behavior without each team inventing different timeout logic.

#### Wave 5 — OAuth Hardening + Observability
**Ships:**
- refresh token rotation
- auth health check endpoint / diagnostics

**Strong value:** security and operability improve together, which is the right trade for production auth.

#### Wave 6 — Auto-Routes + UI Convenience
**Ships:**
- auto-mounted auth routes
- optional login page generator and convenience UI helpers

**Strong value:** fast starts for simple apps, while custom apps can still own their UX.

#### Wave 7 — Advanced Sessions
**Ships:**
- device-aware session tracking
- revocation cascades
- suspicious activity signals
- remember-me semantics built on the hardened lifecycle model

**Strong value:** this is where `std/auth` moves from solid to category-leading.

#### Wave 8 — Internal Cleanup
**Ships:**
- module split / internal architecture cleanup

**Strong value:** keeps the implementation maintainable without delaying user-facing wins.

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
