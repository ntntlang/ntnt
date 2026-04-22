# DD-043: World-Class Auth — Making `std/auth` the Best stdlib Auth Ever

**Status:** In Progress  
**Author:** Larri  
**Date:** 2026-03-20  
**Branch:** `main` (Phase 2 merged via PR #77, Phase 3 merged via PR #78, Phase 4 core + Phase 4.5A/4.5B refactor merged via PR #81)

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

**Status:** Core staged-auth primitives merged in PR #81 (`feat: add staged auth challenges and split std/auth internals`) on 2026-04-14. The primitive layer is shipped; the concrete app-flow follow-through items below are still open.

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

### Phase 4.5 — Auth Internal Architecture Cleanup
**Goal:** Pay down the structural debt around `std/auth` before more lifecycle, observability, and security features pile onto the current shape.

**Status:** In progress. PR #81 landed the planning/docs pass plus the major module split pass, and PR 4.5C-1 now carries the internal storage contract across sessions, staged auth challenges, OAuth states, and exchange tokens. The next unfinished architecture step is shared fallback/error semantics work, followed by the backend contract-test matrix.

**Why now:** Phase 4 shipped the right staged-auth primitives, but it also made a pre-existing architecture issue impossible to ignore: `src/stdlib/auth.rs` is carrying too many responsibilities at once. The risk is not just storage duplication. It is that config parsing, cookie policy, OAuth flow logic, session/challenge persistence, route protection, built-in handlers, JWT/TOTP helpers, and tests are all evolving in one giant file with partially repeated semantics. If we want auth to become world-class instead of merely feature-rich, we should clean up the internal architecture now, while the surface area is still understandable.

**Design principles for this phase:**
- [ ] Keep the public `std/auth` API stable unless a change is overwhelmingly justified
- [ ] Separate pure auth state transitions from HTTP request/response glue where practical
- [ ] Prefer one intentional internal model over per-backend drift
- [ ] Make backend-specific code explicit, localized, and contract-tested
- [ ] Optimize for future auth work being easier to review, reason about, and extend

**Workstream A — Internal module boundaries**
- [x] Split `src/stdlib/auth.rs` into smaller focused modules without changing the public `std/auth` API surface
- [x] Establish a clear internal layout for config, cookies, providers, OAuth flow, storage, sessions, staged auth challenges, middleware/route protection, built-in routes, JWT helpers, and TOTP helpers
- [x] Move shared validation and conversion helpers to stable internal homes so they stop drifting across feature work
- [x] Keep module boundaries understandable enough that a new contributor can answer “where does this logic live?” quickly and confidently

**Workstream B — Auth domain model and flow boundaries**
- [ ] Identify the core auth state transitions that should be treated as domain logic rather than HTTP glue (for example: begin staged auth, consume challenge, rotate session, sign out, exchange OAuth result into session)
- [ ] Refactor those transitions so request parsing, cookie mutation, and redirect/response shaping are thin adapters around clearer internal operations
- [ ] Normalize the lifecycle vocabulary across sessions, auth challenges, OAuth states, and exchange tokens so store/get/consume/delete/cleanup semantics are easier to compare and reason about

**Workstream C — Internal persistence contract**
- [x] Extract a clearer internal auth storage abstraction so session, auth-challenge, OAuth-state, and exchange-token behavior is easier to keep consistent across SQLite/Postgres/Redis and memory fallback paths
- [ ] Define one intentional place for fallback/error semantics instead of letting each code path decide ad hoc whether backend failures should surface, degrade to memory, or return `None`
- [ ] Reduce copy-pasted backend logic where the operation shape is truly shared, while preserving backend-native implementations where atomicity or query shape genuinely differs
- [ ] Revisit cleanup responsibilities so memory, SQLite, Postgres, and Redis all follow the same mental model even if the implementation details differ

**Workstream D — Test architecture and verification matrix**
- [x] Add backend contract tests that exercise the same auth storage behaviors across memory and SQLite for the current contract helpers
- [x] Add env-gated Redis/Postgres integration coverage for auth storage flows so backend-specific regressions are caught before review comments or production incidents
- [x] Add higher-level auth flow tests that verify staged auth, session rotation, protected-route lookup, and logout behavior against the cleaned-up internal boundaries
- [x] Re-review Phase 3 and Phase 4 helpers after the refactor to ensure docs, tests, and public behavior still match exactly

**Workstream E — Safety rails for the refactor itself**
- [x] Preserve behavior first, then simplify structure, rather than mixing feature changes into the cleanup branch
- [x] Write down the intended auth persistence/error/fallback model in the design doc and/or code comments before refactoring the trickiest paths
- [x] Use small, reviewable commits or PR slices when possible so architecture cleanup does not become an unreadable mega-diff
- [x] Regenerate docs and re-run the full auth validation gate after each meaningful slice, not just at the very end

**Recommended execution order:**
- [x] 4.5A: document the intended internal architecture and fallback/error model before moving code
- [x] 4.5B: split modules and relocate helpers with behavior held constant
- [x] 4.5C: introduce the internal storage contract and normalize shared semantics (4.5C-1 and 4.5C-2 complete)
- [x] 4.5D: add/finish backend contract tests and env-gated integration coverage
- [x] 4.5E: run a final API/docs behavior audit for all Phase 3 and Phase 4 helpers

**Current recommendation:** Phase 4.5 is complete. The storage contract is in place, the fallback/error policy is now explicit and test-covered across sessions, auth challenges, OAuth states, and exchange tokens, and the final Phase 3/4 API/docs behavior pass is done. Move next into Phase 5 session lifecycle work.

**Non-goals:**
- [ ] Do not add major new end-user auth features in this phase
- [ ] Do not redesign the public `std/auth` API unless the cleanup reveals a serious correctness issue
- [ ] Do not force auth onto `std/kv` just for abstraction symmetry if a purpose-built internal auth store boundary is cleaner
- [ ] Do not ship a prettier file layout while leaving core fallback and lifecycle semantics ambiguous

**Exit criteria:**
- [ ] We can explain auth internals as one coherent model instead of a set of backend-specific exceptions
- [ ] Adding or modifying an auth backend no longer requires copy-pasting large sections of session/challenge/state logic
- [ ] Future auth PRs can land in focused modules instead of reopening a 10k-line file every time
- [ ] Reviewers can evaluate fallback/error behavior from one documented policy instead of rediscovering it thread by thread
- [ ] The cleaned-up structure makes Phases 5–8 safer to implement, not merely nicer to look at

**Current landed module map (after PR #81):**
- [x] `auth/config.rs` — `AuthConfig`, defaults, option parsing, config validation, startup summary, and session-store initialization helpers
- [x] `auth/cookies.rs` — cookie-name validation, shared cookie settings, signed session/challenge cookies, clear-cookie helpers, and `SITE_URL`-aware cookie posture helpers
- [x] `auth/providers.rs` — built-in provider definitions, provider normalization, and provider/value conversion helpers
- [x] `auth/oauth.rs` — OAuth start/exchange/refresh/discovery/introspection flow coordination and provider userinfo handling
- [x] `auth/guards.rs` — protected-path registration/matching, auth enforcement behavior, and challenge-kind validation
- [x] `auth/routes.rs` — built-in `/auth/*` handlers and request/response adapters
- [x] `auth/request_helpers.rs` — request extraction plus `Session`/`User`/`AuthChallenge` to `Value` conversion helpers
- [x] `auth/sessions.rs` — session lifecycle, mutation, and session-listing/revocation store coordination
- [x] `auth/storage.rs` — auth-challenge, OAuth-state, exchange-token, and cleanup persistence logic across backends
- [x] `auth/primitives.rs` — low-level auth primitives like IDs, nonces, HMAC signing helpers, and TOTP primitives
- [x] `auth/utils.rs` — response builders and canonical JSON/value conversion helpers used across auth internals

**Remaining target splits / normalization work:**
- [ ] Split `auth/storage.rs` into a clearer internal storage contract plus backend-focused modules (`mod`, `memory`, `sqlite`, `postgres`, `redis`) once 4.5C starts
- [ ] Decide whether session domain logic should stay centralized in `auth/sessions.rs` or split further into a narrower `session_core.rs` plus admin/query helpers
- [ ] Pull staged-auth challenge lifecycle into a clearer domain boundary (`challenge_core.rs` or equivalent) if that materially improves the session/challenge split during 4.5C
- [ ] Split JWT helpers out of `auth.rs` if they continue growing enough to deserve `auth/jwt.rs`
- [ ] Keep TOTP primitives in `auth/primitives.rs` unless a dedicated `auth/totp.rs` becomes justified by size or clarity
- [ ] Revisit whether `auth/request_helpers.rs` and `auth/utils.rs` should converge on a more explicit `value_maps.rs` style home for `Session`/`User`/`AuthChallenge` conversion semantics

**Recommended PR slicing plan:**
- [x] PR 4.5A-1: add architecture notes, auth persistence/error/fallback invariants, and TODO anchors without moving behavior yet
- [x] PR 4.5A-2: add backend contract-test harness that can run against memory/SQLite immediately and Postgres/Redis when env-gated services are available
- [x] PR 4.5B-1: move config/cookie/provider/JWT/TOTP helpers into modules with behavior held constant
- [x] PR 4.5B-2: move middleware and built-in route glue into modules with no intentional semantic changes
- [x] PR 4.5C-1: introduce the internal storage contract for sessions/auth challenges/OAuth states/exchange tokens while preserving existing backend-native implementations
- [x] PR 4.5C-2: centralize fallback/error semantics and make each auth state type follow the same documented rules
  - [x] Define the canonical fallback/error policy table for sessions, auth challenges, OAuth states, and exchange tokens
  - [x] Make store/get/consume/delete/cleanup semantics follow that policy consistently across memory, SQLite, Postgres, and Redis
  - [x] Normalize TTL/expiry behavior for memory fallback paths so they do not silently diverge from backend-native semantics
  - [x] Add focused tests that prove the chosen fallback/error behavior instead of relying on comments and TODOs
- [x] PR 4.5D-1: add/finish Redis/Postgres integration coverage and cross-backend contract assertions
- [x] PR 4.5E-1: final audit PR for docs generation, public API parity, and cleanup of temporary compatibility shims

**Highest-risk regression hotspots to guard during the refactor:**
- [ ] Cookie naming/signing/clearing behavior for both session and auth-challenge cookies stays byte-for-byte compatible unless intentionally changed
- [ ] HTML redirect vs API `401`/`403` behavior in `require_auth()` does not drift during middleware extraction
- [ ] One-time consume semantics for OAuth states, exchange tokens, and auth challenges remain atomic per backend
- [ ] Session rotation preserves intended claims/data while invalidating the old session immediately
- [ ] Refresh-token and expired-session lookup behavior does not regress while storage semantics are being normalized
- [x] Memory fallback continues to behave intentionally, especially under backend errors and mixed cleanup paths
- [x] Generated stdlib docs and `@ntnt` signatures remain accurate after functions move files/modules

**Validation matrix for phase completion:**
- [x] Memory backend: session sign-in/current-user/sign-out/rotation/challenge begin-complete-cancel all pass
- [x] SQLite backend: same core session + challenge flows pass with cleanup and consume behavior verified
- [x] Postgres backend: same core session + challenge flows pass in env-gated integration coverage
- [x] Redis/Valkey backend: same core session + challenge flows pass in env-gated integration coverage, including atomic consume paths
- [x] Protected-route behavior verified for HTML pages and API routes after module split
- [x] OAuth state and exchange-token paths still pass, including Safari/redirect-chain-related flows already covered by current behavior
- [x] `cargo test stdlib::auth::tests`, any relevant integration suites, `cargo build --release --locked`, and `./target/release/ntnt docs --generate` all pass at the end of each major slice

**Ships real value:** this phase does not add flashy new auth features, but it is what makes the next layers of auth quality, safety, and speed possible without quietly accumulating structural debt.

### Phase 5 — Session Lifecycle and Presets
**Goal:** Harden session lifetime behavior and make strong defaults ergonomic.

- [x] Add sliding session expiry support
- [x] Throttle refreshes so stores are not updated on every request
- [x] Refresh cookie Max-Age/Expires along with sliding sessions (for built-in/auth-enforced response paths)
- [x] Add absolute maximum session lifetime support
- [x] Store session creation time separately from idle expiry (use immutable `created_at` as the max-lifetime anchor)
- [x] Clear log/error path when max lifetime forces re-auth
- [x] Define preset configurations (`consumer`, `admin`, `internal`, `strict`)
- [x] Accept String or Map as second arg to `enable_auth`
- [x] Allow preset + override merge
- [x] Document preset guidance clearly

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

### Phase 9 — Post-Cleanup Complexity Discipline
**Goal:** Prevent `std/auth` from collapsing back into a monolith after the 4.5 cleanup lands.

- [ ] Track auth code size / complexity budget as features land
- [ ] Require new auth work to fit the post-4.5 module boundaries instead of reopening broad grab-bag files
- [ ] Extend backend contract tests when adding new auth state types, flows, or stores
- [ ] Treat fallback/error semantics as design-level behavior, not incidental implementation detail
- [ ] Periodically re-audit whether new features are preserving the intended internal architecture

**Ships real value:** preserves the benefits of the cleanup so future auth work stays understandable instead of slowly regressing.

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
| **4** | Internal Architecture Cleanup | 4.5 internal module split, storage contract, fallback semantics, and backend verification matrix | Pay down auth structural debt before more lifecycle and security features stack onto the current shape |
| **5** | Session Lifecycle | 1.2 + 1.3 + 3.6 sliding sessions, max lifetime, presets | Hardens lifecycle rules after the core mechanics and architecture are stable |
| **6** | OAuth Hardening + Observability | 2.1 + 3.5 refresh token rotation and auth health check | Tightens security posture and makes behavior inspectable |
| **7** | Auto-Routes + UI Convenience | 3.2 + 3.8 auto-routes and login page generator | Helpful DX layer after the mechanics are already excellent |
| **8** | Advanced Sessions | 4.1 + 4.2 + 4.3 + 4.4 device sessions, cascade revoke, suspicious activity, remember me | Differentiators layered onto a solid core |
| **9** | Post-Cleanup Discipline | Phase 9 module-boundary enforcement and complexity budget | Prevent the architecture cleanup from eroding over time |

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

#### Wave 4 — Internal Architecture Cleanup
**Ships:**
- module split into coherent auth internals
- documented fallback/error semantics
- internal storage contract across sessions/challenges/states/tokens
- backend contract-test harness + env-gated Postgres/Redis verification

**Strong value:** this is the quality wave that makes the next auth features safer to build instead of more expensive every time.

#### Wave 5 — Session Lifecycle
**Ships:**
- sliding expiration
- absolute max lifetime
- ergonomic presets (`strict`, `balanced`, `relaxed`, etc.)

**Current status:** Wave 5B added lifecycle presets (`consumer`, `admin`, `internal`, `strict`), preset+override merge, preset-string `enable_auth(...)` input support, and cookie refresh alignment for the built-in/auth-enforced response paths. Phase 5 is complete.

**Strong value:** apps get secure lifecycle behavior without each team inventing different timeout logic.

#### Wave 6 — OAuth Hardening + Observability
**Ships:**
- refresh token rotation
- auth health check endpoint / diagnostics

**Strong value:** security and operability improve together, which is the right trade for production auth.

#### Wave 7 — Auto-Routes + UI Convenience
**Ships:**
- auto-mounted auth routes
- optional login page generator and convenience UI helpers

**Strong value:** fast starts for simple apps, while custom apps can still own their UX.

#### Wave 8 — Advanced Sessions
**Ships:**
- device-aware session tracking
- revocation cascades
- suspicious activity signals
- remember-me semantics built on the hardened lifecycle model

**Strong value:** this is where `std/auth` moves from solid to category-leading.

#### Wave 9 — Post-Cleanup Discipline
**Ships:**
- ongoing enforcement of module boundaries, complexity budget, and backend contract coverage for new auth work

**Strong value:** protects the cleanup investment so auth does not slowly collapse back into a grab-bag implementation.

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
