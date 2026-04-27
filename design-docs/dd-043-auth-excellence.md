# DD-043: World-Class Auth — Making `std/auth` the Best stdlib Auth Ever

**Status:** In Progress
**Author:** Larri
**Date:** 2026-03-20
**Branch:** `main` through v0.4.9; PR #98 landed the first local-auth credential/storage slice, and `feat/local-auth-sign-in-bootstrap` adds the next bootstrap/request-aware sign-in slice before DD-062 continues with setup completion, TOTP, and reset work

---

## Vision

`std/auth` should be the auth layer that makes app developers stop rebuilding security-sensitive auth plumbing. Its job is not to become a hosted identity product, a profile system, or an opinionated app-auth framework. It should provide standards-based primitives, safe defaults, durable storage contracts, request-aware session completion, and thin reference flows that apps can compose without losing the secure path.

**Design principle:** language-level auth should make the secure primitive the obvious primitive. OAuth callbacks, local credentials, staged setup, password reset, and future step-up flows should all converge on the same session/cookie/lifecycle/storage semantics. Optional reference routes/pages may exist, but business policy remains app-owned.

### Ownership Boundary

Draw this line before adding auth surface area:

| Layer | Owned by `std/auth` | Not owned by `std/auth` |
|---|---|---|
| **Core primitives** | OAuth/OIDC, signed cookies, server-side sessions, CSRF, route protection, staged auth challenges, TOTP verification/enrollment primitives, local credential/reset/TOTP storage contracts, request-aware session completion, diagnostics, backend contract tests | App-specific onboarding, invite/approval rules, profile fields, orgs, permissions models, copy/design decisions |
| **Reference flows** | Optional built-in login/reset/setup/bootstrap routes and minimal default pages that compose the primitives | Mandatory UI structure, product-specific account-management screens, hosted-dashboard semantics |
| **App policy** | Explicit hooks/options that feed claims/session data into the shared session completion path | Static universal roles/claims baked into provider config, hidden suspicious-activity policy engines, email/SMS delivery choices |

If a capability is security-sensitive lifecycle state that every app otherwise reimplements badly, it belongs in `std/auth` primitives. If it is product/business behavior, `std/auth` should expose a hook or reference flow and get out of the way.

---

## Current State (v0.4.9)

### What We Have (and it is solid)
- [x] OAuth 2.0 + OIDC (Google, GitHub, Discord, Apple, Microsoft, generic)
- [x] PKCE for all OAuth flows
- [x] Server-side sessions (Redis, PostgreSQL, SQLite, memory)
- [x] HMAC-signed session cookies (tamper-proof)
- [x] CSRF protection (per-session tokens, automatic validation on state-changing requests)
- [x] Automatic token refresh with provider-aware refresh-token rotation/preservation
- [x] `HttpOnly`, `Secure`, `SameSite=Lax` cookies by default
- [x] Configurable session/refresh TTLs, sliding expiry, absolute max lifetime, and session presets
- [x] Safari ITP workaround (two-phase exchange token flow), retained after validation
- [x] Current-user session listing (`user_sessions(req)`) and current-user bulk revocation (`logout_all(req, keep_current)`)
- [x] Session rotation/migration helpers and sign-in/sign-out/current-session helpers
- [x] Staged auth challenge primitives for MFA/setup/step-up flows
- [x] Internal auth module split for config, cookies, providers, OAuth, guards, routes, request helpers, sessions, storage, primitives, and utilities
- [x] Explicit fallback/error policy for existing session, challenge, OAuth-state, and exchange-token storage
- [x] Backend contract coverage for memory/SQLite by default and Postgres/Redis when env-gated services are available
- [x] Configurable route prefixes, built-in auth route logging, auth health diagnostics, and a configurable built-in login page
- [x] Session metadata/security-signal plumbing (`device_name`, `user_agent_hash`, `last_ip_hash`, `remember_me`) across the OAuth path and existing storage backends
- [x] Password hashing and generic crypto helpers in `std/crypto`
- [x] TOTP/MFA primitives (`totp_secret`, `totp_verify`, `totp_uri`) in `std/auth`
- [x] API key validation, Turnstile CAPTCHA verification, OAuth token introspection, client credentials grant, and OIDC discovery
- [x] Local-auth credential foundation from PR #98: focused `auth/storage/local.rs` storage home, explicit local-auth record/fallback policy, local identity/account state model, memory/SQLite identity + credential storage, and public `verify_local_password(identifier, password, options?)`
- [x] Local-auth bootstrap/sign-in branch: public `create_local_user`, `bootstrap_local_user`, and request-aware `local_sign_in(response, req, credentials, session?, options?)` helpers that reuse shared session/challenge cookie semantics

### What's Still Missing (the gap to "best ever")
- [ ] Complete first-class local email/password/TOTP credential lifecycle owned by `std/auth`; local identity/credential storage, bootstrap provisioning, password verification, and request-aware sign-in are started, but setup completion/reset/TOTP flows are not complete
- [ ] Auth-owned password reset and TOTP enrollment stores with fail-closed fallback semantics
- [x] First-class local credential sign-in domain helper that feeds verified local credentials into the existing request-aware session-completion path
- [ ] Admin/arbitrary-user session APIs (`list_sessions(user_id)`, `revoke_session(session_id)`, `revoke_all_sessions(user_id)`) distinct from current-user helpers
- [ ] Security event storage and suspicious-activity policy actions (`warn`, `challenge`, `revoke`)
- [ ] Fully behavioral remember-me support (`remember_ttl`, request capture, cookie/session TTL selection)
- [ ] Stronger test ratchets for session metadata round-trips, route-protection end-to-end flows, local-auth migration paths, and Postgres/Redis CI coverage
- [ ] Architecture guardrails that keep new auth work inside focused auth modules instead of re-growing `auth.rs` or the storage contract boundary

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

**Result:** Merged in PR #78 (`feat: add auth session helpers`) on 2026-04-13, then tightened in PR #97 so manual/staged session completion is request-aware before first-class local auth builds on it.

- [x] Rotate session ID automatically on successful OAuth callback/session upgrade paths
- [x] Invalidate old session ID immediately after rotation
- [x] Preserve intended session data across rotation
- [x] Add request-aware manual/staged session completion rotation and metadata capture before first-class local auth ships
- [x] Session store migration implemented in Rust for Redis/Postgres/SQLite backends
- [x] Keep `migrate_session(old_id, new_id)` internal to Rust/session-store code
- [x] Public ntnt API exposes only high-level session helpers, not low-level store migration primitives (`sign_in_session`, `rotate_session`, `sign_out_session`, `current_session`, `current_user`)
- [x] Request-aware `sign_in_session(response, req, session, options?)` helper that persists session + attaches cookie while rotating/migrating existing sessions and capturing request metadata
- [x] `sign_out_session()` helper that revokes session + clears cookie
- [x] `current_session()` / `current_user()` helper for request-time lookups
- [x] Shared auth cookie defaults with per-app overrides
- [x] Centralize cookie posture: name, path, same-site, secure, httpOnly, expiry
- [x] Defer session rotation event logging until auth security logging exists, tracked in Phase 6

**Possible public API shape:**

```ntnt
let resp = redirect("/admin")
return sign_in_session(resp, req, map {
    "subject_id": user["id"],
    "claims": app_claims_for_user(user)
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
    "claims": app_claims_for_user(user)
})
```

**Ships real value:** the template’s password/TOTP/password-change flow gets much cleaner and more robust.

### Phase 4.5 — Auth Internal Architecture Cleanup
**Goal:** Pay down the structural debt around `std/auth` before more lifecycle, observability, local-auth, and security features pile onto the old shape.

**Status:** Complete enough to proceed, but intentionally not declared "done forever." The major cleanup wave landed: auth is no longer one undifferentiated file, storage semantics are documented, backend contracts exist, and the later Phase 5–8 work proved the structure is usable. The follow-up work is now tracked explicitly in Phase 9A and Phase 10 instead of pretending the first cleanup pass solved every future pressure point.

**What landed:**
- [x] Split `src/stdlib/auth.rs` into focused internal modules for config, cookies, providers, OAuth flow, guards/route protection, built-in routes, request helpers, sessions, storage, primitives, and utilities
- [x] Established a shared internal auth storage boundary for sessions, staged auth challenges, OAuth states, and exchange tokens
- [x] Documented the fallback/error contract for existing auth record families
- [x] Added backend contract coverage for memory/SQLite and env-gated Postgres/Redis paths
- [x] Preserved the public `std/auth` API while moving behavior behind clearer internal boundaries
- [x] Re-ran docs generation and auth validation gates during the implementation slices

**Important reality check:** the cleanup improved the architecture, but it did not make the architecture self-enforcing yet.

Current pressure points:
- `src/stdlib/auth.rs` is still large and remains the gravity well for public types, native-function registration, global memory state, JWT/TOTP wrappers, and most auth tests
- `src/stdlib/auth/storage.rs` is now the second gravity well: it owns the storage contract plus backend-native logic for every existing auth record family
- Backend contract tests are useful but concentrated in `auth.rs`, making them harder to extend as new auth state families arrive
- The existing fallback policy is appropriate for mostly-transient session/challenge/OAuth state, but it must not be blindly reused for durable local credentials

**Architecture standard after this phase:** every new auth feature must answer three questions before implementation:
1. Which internal module owns the domain behavior?
2. Which storage contract owns persistence semantics, and how are fallback/error paths tested?
3. Which public API, docs, generated stdlib reference, and typechecker signatures need to agree?

**Follow-up ownership:**
- Phase 9A owns the preflight cleanup required before first-class local auth lands
- Phase 10 owns ongoing guardrails, complexity budgets, and periodic architecture audits

**Ships real value:** this phase made the auth system understandable enough to keep improving. The next job is making that structure harder to accidentally bypass.

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

- [x] Refresh token rotation when providers issue a new refresh token
- [x] Invalidate old refresh token references where feasible
- [x] Log refresh token rotation events
- [x] Document provider differences clearly
- [x] `/auth/health` endpoint (dev-only by default)
- [x] Health output shows config state without leaking secrets
- [x] Diagnose common issues like redirect mismatch, missing env, wrong store

**Implementation note:** Phase 6 preserves provider-specific refresh-token behavior correctly (including rotation only when a provider actually returns a new refresh token), logs rotation events without exposing token material, and adds a built-in `/auth/health` diagnostics route that is dev-only by default and can be explicitly enabled in production.

**Provider differences:** some providers return a fresh refresh token on every refresh, while others omit `refresh_token` unless rotation occurs. `std/auth` now preserves the stored token when providers omit it, replaces the stored token when they rotate it, and surfaces the distinction through safe auth logs and diagnostics rather than pretending all providers behave the same.

**Ships real value:** better production safety and far easier troubleshooting.

### Phase 7 — Auto-Routes and Convenience UI
**Goal:** Make the easy path extremely easy without forcing apps into one UI.

**Status:** Core Phase 7A landed in PR #94 and is on `main` in v0.4.9.

- [ ] Auto-generate every standard auth route when not overridden
- [x] Configurable path prefixes (`/auth/*` by default)
- [x] Clear startup log showing registered auth routes
- [x] Path collision diagnostics for built-in auth routes and protected-path config
- [x] Built-in login page template
- [x] Configurable title/logo/copy
- [x] Provider buttons generated automatically
- [ ] Support custom HTML override if needed
- [ ] Add end-to-end route-dispatch tests proving route protection works through actual ntnt server/file-route setup, not only helper-level matching

**Ships real value:** simple apps get auth with almost no wiring, while custom apps still own their design.

### Phase 8 — Advanced Sessions and Security Signals
**Goal:** Add higher-end capabilities once the core mechanics are excellent.

**Status:** Phase 8A landed in PR #96 and is on `main` in v0.4.9. This slice delivered metadata/security-signal plumbing, not the full suspicious-activity product layer.

**What landed:**
- [x] Store user agent hash / device name fields on session records
- [x] Pass remember-me flag through OAuth state storage
- [x] Persist OAuth-state security metadata (`remember_me`, `device_name`, `user_agent_hash`, `last_ip_hash`) across SQLite/Postgres/Redis + memory fallback
- [x] Persist session security metadata (`device_name`, `user_agent_hash`, `last_ip_hash`) across SQLite/Postgres/Redis + memory fallback
- [x] Copy captured OAuth-state metadata onto the real session at callback time before store/rotation
- [x] Track IP signal material on sessions via `last_ip_hash`
- [x] Fall back to request `ip` when proxy headers are absent for direct connections
- [x] Use keyed HMAC-SHA256 for stored security-signal hashes instead of plain SHA-256
- [x] Harden SQLite/Postgres migration behavior so schema updates stop swallowing real failures
- [x] Extend auth storage contract tests and auth flow tests for the new metadata paths

**Still open before Phase 8 is product-complete:**
- [ ] Expose appropriate device metadata through session-management APIs (`device_name` yes; raw hashes no)
- [ ] `list_sessions(user_id)` API for admin/arbitrary-user session lookup
- [ ] `revoke_session(session_id)` API
- [ ] `revoke_all_sessions(user_id)` API
- [ ] Password change hook support for revocation
- [ ] Account disable hook support for revocation
- [ ] Optional `revoke_on_password_change: true` config
- [ ] Add `security_events` storage for auth events
- [ ] Configurable suspicious-activity actions: `warn`, `challenge`, `revoke`
- [ ] Expose `auth_events(user_id, limit?)` for admin dashboards
- [ ] Rate limit failed OAuth callbacks and future local-auth login/reset attempts
- [ ] Add `remember_ttl`
- [ ] Capture remember-me intent from request/config instead of hard-coding false on auth start
- [ ] Set distinct remember-me TTL/persistence behavior on session and cookie creation
- [ ] Strengthen contract tests to assert metadata round-trips across store/get, migrate/rotate, session listing, refresh lookup, and OAuth-state consume

**Assessment:** Phase 8A was the right foundation slice. It made the metadata survive real OAuth/session paths and existing backends. The next slices should turn that plumbing into visible session-management, remember-me, eventing, and suspicious-activity behavior.

**Ships real value:** this is where `std/auth` starts becoming genuinely standout, not just competent.

### Phase 9 — First-Class Local Auth
**Goal:** Make `std/auth` world-class not only for OAuth/session plumbing, but also for the extremely common app shape of **local email + password + TOTP auth**.

**Why this must exist:** the current `std/auth` surface is strong at sessions, cookies, staged auth challenges, OAuth/OIDC, TOTP primitives, sign-in/sign-out helpers, and protected-route handling. But a normal local admin/app flow still requires developers to build and own a mini auth system beside it:

- local user/credential table
- password-hash storage and verification
- password reset token storage
- persisted TOTP enrollment state
- first-login / forced-password-change flow
- bootstrap admin account creation
- ad hoc session-claim handoff from custom local auth into `std/auth` sessions

That is the current local-auth subsystem gap.

**Architectural standard:** a template app should not need a custom `lib/admin_db.tnt` mini-auth subsystem to get excellent email/password/TOTP auth. The template should mostly configure `std/auth`, provide custom views/copy if it wants them, and move on.

**Preferred public API direction:** local auth should be a first-class primitive family that feeds the existing `enable_auth(...)` and request-aware session-completion path, not a bundled app-auth product. Config may enable reference routes, but core local credential behavior should remain decomposed enough that custom UI does not require custom persistence.

```ntnt
import {
    enable_auth,
    local_credentials,
    verify_local_password,
    sign_in_session,
} from "std/auth"
import { parse_form, redirect } from "std/http/server"

enable_auth([], "admin", map {
    "local_credentials": local_credentials(map {
        "identifier": "email",
        "totp": true,
        "password_reset": true
    }),
    "login_page": false,
    "success_url": "/admin",
    "failure_url": "/admin/login"
})

fn login(req) {
    let form = parse_form(req)
    let verified = verify_local_password(form["email"] ?? "", form["password"] ?? "")?
    return sign_in_session(redirect("/admin"), req, map {
        "subject_id": verified["subject_id"],
        "email": verified["email"],
        "claims": app_claims_for_local_user(verified)
    })
}
```

A later `enable_local_auth(...)` convenience wrapper is acceptable only if it delegates into the same primitive provider/config path and keeps policy hooks explicit. It must not become two auth systems or bake roles, onboarding, email delivery, or account UI into `std/auth`.

#### Phase 9A — Architecture Preflight and Storage Boundary
- [x] Split or carve `auth/storage.rs` enough that new local-auth state does not get buried in the existing storage monolith (`auth/storage/local.rs` now owns the first local-auth records)
- [x] Define explicit local-auth record families before code lands: local identity, credential secret, password reset token, TOTP enrollment/setup state, and bootstrap state
- [x] Define local-auth fallback/error policy before implementation; durable credential/TOTP/reset state should fail closed rather than silently fall back to process memory in production
- [ ] Move the backend contract harness toward reusable storage-module ownership so every new local-auth record can extend it immediately
- [ ] Document which parts of `auth.rs` are allowed to grow for local auth and which must live in focused modules
- [ ] Add review checklist items for local-auth regressions: captured-but-not-persisted state, reset-token replay, backend mismatch, swallowed migration errors, and app/std ownership ambiguity

**Status after PR #98:** the first credential table/helper slice has a focused storage/domain home, but the reusable contract-harness/review-guidance ratchet is still open and should be paired with the next local-auth slice.

#### Phase 9B — Local Identity and Credential Store
- [x] Add first-class local credential storage owned by `std/auth` for memory and SQLite
- [x] Support a generic local subject + identifier model; ship email as the first documented identifier preset with normalization rules, not as the only possible local-auth identity shape
- [x] Store password hashes in auth-owned tables rather than pushing that responsibility to apps
- [x] Define a lean durable local-user/account shape: identity + auth state first, not a full profile platform
- [x] Support account states needed by real flows: bootstrap/pending setup/active/disabled/locked/password-change-required
- [x] Support bootstrap account creation from config/env for the common admin-panel case
- [x] Ensure bootstrap credentials force rotation/setup instead of becoming a permanent production secret path
- [x] Add memory/SQLite contract tests by default
- [ ] Add Postgres/Redis contract tests in required backend CI

#### Phase 9C — Request-Aware Local Sign-In Flow

**Dependency already satisfied:** the shared manual/staged completion primitive is now request-aware via `sign_in_session(response, req, session, options?)` and `complete_auth_challenge(response, req, session?, options?)`. This phase should not invent a second session-completion path; it should build local credential verification and any higher-level local sign-in helper on that existing primitive.

Because this lands before 0.4.9 is released, the old pre-release `sign_in_session(response, session, options?)` shape should not be kept as a compatibility shim. Callers must migrate by passing the route `req` as the second argument; otherwise local/manual auth would silently miss rotation, existing-session migration, and request metadata capture.

- [x] Add built-in email/password verification through `std/auth` (`verify_local_password`)
- [x] Use the existing request-aware session-completion primitive from the local sign-in path so it can rotate/migrate existing sessions and capture request metadata
- [x] Reuse the same session cookie, session TTL, metadata, and rotation semantics as OAuth-backed sign-in
- [x] Reuse staged auth challenge primitives for local login continuation instead of inventing a second pending-auth model
- [x] Support straightforward local sign-in that upgrades into a real auth session with app-supplied claims/session data
- [x] Ensure local sessions receive `device_name`, `user_agent_hash`, and `last_ip_hash` just like OAuth sessions
- [ ] Finish full remember-me behavior for local sign-in once `remember_ttl` policy is ratcheted end-to-end

#### Phase 9D — First-Login Activation, Forced Password Change, and TOTP Enrollment
- [ ] Support first-login setup as a first-class local-auth flow
- [ ] Support staged TOTP enrollment using the existing auth challenge model
- [ ] Support forced password change before final session completion
- [ ] Support completion into a normal signed-in session only after required setup steps are satisfied
- [ ] Persist TOTP enrollment/setup state explicitly; do not hide durable enrollment state inside generic challenge `data_json`
- [ ] Define TOTP reset/re-enrollment behavior after password reset or admin intervention

#### Phase 9E — Password Reset Lifecycle
- [ ] Add auth-owned password-reset token storage and lifecycle helpers
- [ ] Support reset token issue + consume flows through `std/auth`
- [ ] Make reset tokens one-time-use, TTL-bound, and atomically consumed across every backend
- [ ] Support optional TOTP reset / re-enrollment semantics after password reset
- [ ] Support reset flows that intentionally force fresh staged setup when security posture demands it
- [ ] Define the email-delivery boundary: `std/auth` issues/validates tokens and builds safe URLs; apps/plugins send email
- [ ] Rate-limit failed local login and password-reset attempts

#### Phase 9F — Template-Grade Integration and Deletion of App-Owned Mini-Auth
- [ ] Make the Larri site template use the built-in local auth path instead of custom auth persistence code
- [ ] Delete template-owned local-auth state machines and tables once parity exists
- [ ] Leave the template with mostly config + views + app-specific copy, not a bespoke auth backend
- [ ] Use the template migration as the proof that the local-auth architecture is actually simpler, not just theoretically cleaner
- [ ] Keep custom UI possible without requiring custom credential/session/reset persistence

#### Phase 9G — Local Auth Verification Matrix
- [ ] Add contract tests for every local-auth record family across memory and SQLite by default
- [ ] Add required CI coverage for Postgres and Redis/Valkey local-auth contracts
- [ ] Add migration tests for existing DBs missing new local-auth columns/tables
- [ ] Add end-to-end ntnt tests for bootstrap login, password change, TOTP setup, normal login, reset token issue/consume, reset replay rejection, disabled account rejection, and session revocation after credential changes
- [ ] Add docs/reference examples that are runnable, lintable, and intentionally clear about app-owned UI/email boundaries

**Non-goals for this phase:**
- [ ] Do not turn `std/auth` into a giant full-profile identity platform in one shot
- [ ] Do not block local auth on solving every RBAC / organizations / account-management use case
- [ ] Do not make templates keep custom persistence “just for flexibility” if `std/auth` can own the common case cleanly
- [ ] Do not ship disconnected helper functions without a coherent local-auth architecture behind them

**Design standard for this phase:**
- [ ] Local auth should feel like one intentional subsystem, not “some templates plus some helpers plus some tables”
- [ ] Email/password/TOTP should be a primary path in `std/auth`, not a second-class example
- [ ] If a normal local admin/auth flow still requires custom app tables after this phase, the phase is incomplete

**Ships real value:** closes the biggest remaining gap between “great auth primitives” and “great auth architecture,” so apps can rely on `std/auth` for the full local email/password/TOTP lifecycle instead of rebuilding it.

### Phase 10 — Architecture Discipline and Complexity Budget
**Goal:** Prevent `std/auth` from collapsing back into a monolith as local auth, session management, security events, and future auth features land.

**Status:** Active. This phase is numbered after local auth because it is ongoing discipline; PR #98 landed the first local-auth storage guardrail, but contributor guidance, reusable contract harnesses, and backend CI visibility still need to harden.

**Assessment after the Phase 8 branch review:** the 4.5 cleanup largely achieved its intended effect, but only partially solved the long-term maintainability problem. The module split is real and materially improves clarity. The storage contract is also much more coherent than before. Recent review feedback mostly hit end-to-end plumbing gaps and edge-case semantics rather than “where does this logic even belong?” confusion. That is a meaningful architectural win.

**However:** `src/stdlib/auth.rs` is still very large and still acts as a gravity well for public surface, native-function registration, shared types, global memory state, JWT/TOTP wrappers, and the main auth test module. `src/stdlib/auth/storage.rs` is also large enough that adding local credentials/reset/TOTP records there without further structure would create a second monolith.

**Bottom line:** the DD intent was mostly achieved for the cleanup wave — enough to justify continuing from this shape — but future auth work must be forced through explicit module/storage/test boundaries. Cleanup alone is not a force field. Annoying, but here we are.

#### Phase 10A — Contributor and Review Guardrails
- [ ] Write explicit contributor guidance for what belongs in `auth.rs` vs internal auth modules
- [ ] Document the allowed responsibilities of `auth.rs`: public surface, shared types, registration glue, and only the minimum unavoidable coordination logic
- [ ] Document when a new auth change must extend an existing module versus when it merits a new focused module
- [ ] Add an auth-specific review checklist covering: captured-but-not-persisted state, schema/query name drift, swallowed migration errors, backend mismatch paths, fallback ambiguity, reset lifecycle drift, session metadata loss, remember-me behavior drift, and app/std ownership confusion

#### Phase 10B — Contract-Test Ratchet
- [ ] Require new auth persistence/state shapes to extend the backend contract harness in the same PR that introduces them
- [ ] Add missing contract coverage for metadata round-trips across session store/get, migrate/rotate, list sessions, refresh lookup, and OAuth-state consume
- [ ] Require new fallback/error semantics to be tested in primary, fallback, and failure modes
- [ ] Make Postgres/Redis auth contract coverage visible in CI instead of silently green when env vars are absent
- [ ] Treat auth storage behavior as a compatibility surface, not just an implementation detail

#### Phase 10C — `auth.rs` and `storage.rs` Pressure Relief
- [ ] Move obvious non-surface types/helpers out of `auth.rs` when doing so improves ownership without destabilizing the public API
- [ ] Extract JWT and TOTP wrappers if they continue to grow or distract from the public auth surface
- [ ] Move memory-store/global-state code toward focused module ownership
- [x] Start splitting storage by record family before adding durable local-auth records (`auth/storage/local.rs`)
- [ ] Continue splitting storage/test ownership as reset, TOTP, bootstrap, and backend implementations land
- [ ] Reassess whether the main auth test concentration should move toward module-local test ownership over time
- [ ] Track `auth.rs` and `auth/storage.rs` growth relative to internal modules so broad edits become visible early

#### Phase 10D — Periodic Architecture Audit
- [ ] Re-review new auth work periodically against the intended module boundaries
- [ ] Check whether recent auth changes increased clarity or merely moved complexity around
- [ ] Reconfirm that fallback semantics remain deliberate and documented as auth work expands
- [ ] Update DD-043, DD-062, and REVIEW guidance when review incidents reveal new recurring patterns
- [ ] Revisit whether large tests inside `auth.rs` should migrate toward module-local test ownership as the structure stabilizes

**Ships real value:** preserves the benefits of the cleanup and local-auth work so future auth development stays understandable instead of slowly regressing into a suspiciously feature-rich junk drawer.

## What We Explicitly Won't Do

These boundaries keep `std/auth` focused as a language stdlib auth system rather than drifting into a hosted identity product.

- **User management UI** — account-management pages, admin dashboards, visual design, and product-specific workflows are app-owned
- **Public self-service signup product flows** — `std/auth` may own local credential storage and verification, but the decision to allow public signup, invite-only signup, approval workflows, email copy, and onboarding UX belongs to the app
- **Email/SMS delivery** — `std/auth` should issue and validate reset/setup tokens and produce safe URLs; apps/plugins own the delivery channel
- **Arbitrary profile systems** — names, avatars, preferences, organizations, billing fields, and app-specific profile data belong in app tables
- **Full RBAC/ABAC/organization modeling** — `std/auth` can carry claims and expose hooks, but it should not solve every authorization model before shipping excellent auth
- **Billing/subscription gating** — not auth, not our problem
- **Vendor-hosted identity service semantics** — no pricing tiers, tenant dashboards, or hosted control planes inside the language runtime

Clarification: **local email/password/TOTP credential lifecycle is in scope** for Phase 9. The non-goal is owning every product/business workflow around accounts, not the core credential/session/reset/setup mechanics that every app currently has to rebuild.

## Delivery Order

The best delivery order is not just "security first" or "DX first". Each wave should leave `std/auth` materially more useful in real apps while reducing, not increasing, long-term architectural drag.

For template cleanup specifically, the biggest early wins were:
1. section-wide route protection
2. session/cookie helpers
3. staged-auth challenge primitives

Those are now shipped. The next big win is local email/password/TOTP auth, but only if it lands as a coherent subsystem with storage/test discipline rather than another layer of helpers.

| Order | Delivery Wave | Feature Bundle | Why This Order |
|-------|---------------|----------------|----------------|
| **0** | Validation | Safari ITP A/B validation | Validate the constraint before optimizing around it |
| **0.5** | Foundation Mini-Wave | zero-config defaults, startup summary, typed config validation | Thin safety pass before cleanup-heavy auth primitives |
| **1** | Route Protection | `require_auth` plus section/file-route protection | Biggest immediate cleanup win for file-routed apps and templates |
| **2** | Session Core | session rotation, sign-in/sign-out/current-session helpers | Removes repetitive login/logout/cookie boilerplate |
| **3** | Staged Auth | pending auth challenge primitives | Unlocks password → TOTP, first-login setup, and step-up auth cleanly |
| **4** | Internal Architecture Cleanup | module split, storage contract, fallback semantics, backend contract matrix | Makes later auth features safer to build and review |
| **5** | Session Lifecycle | sliding expiration, max lifetime, presets | Hardens lifecycle rules after core mechanics are stable |
| **6** | OAuth Hardening + Observability | refresh token rotation and auth health diagnostics | Tightens security posture and production debuggability |
| **7** | Auto-Routes + UI Convenience | configurable auth routes and login page generator | Improves DX without compromising app-owned UX |
| **8** | Advanced Sessions Foundation | metadata/security-signal plumbing, remember-me plumbing | Establishes the data path for device sessions and security events |
| **8.5** | Architecture Guardrails Preflight | Focused local-auth storage home and fail-closed policy baseline | Prevented the first Phase 9 slice from re-growing the monolith |
| **9** | First-Class Local Auth | local identity store, request-aware login, TOTP setup, password reset, template migration | In progress; credential storage/verification landed first, remaining lifecycle work closes the real-app auth gap |
| **10** | Ongoing Architecture Discipline | complexity budget, contract-test ratchet, periodic audits | Keeps `std/auth` excellent as features continue landing |

### Delivery Wave Deliverables

#### Wave 0.5 — Foundation Mini-Wave
**Shipped:**
- production-safe default auth config
- startup summary showing active auth config
- typed config validation with actionable errors

**Value:** creates a stable floor for the cleanup-heavy phases without spending a whole early phase on lower-leverage convenience work.

#### Wave 1 — Route Protection
**Shipped:**
- `require_auth()` middleware
- path/subtree protection for file-routed apps
- HTML redirect vs API `401` behavior
- section-wide protection for things like `routes/admin/*`

**Value:** apps stop re-implementing auth checks in every page handler.

#### Wave 2 — Session Core
**Shipped:**
- session ID rotation on successful OAuth callback/session upgrade
- request-aware `sign_in_session(response, req, session, options?)`
- `sign_out_session()`
- `current_session()` / `current_user()`
- shared cookie defaults and overrides

**Follow-up:** local auth should use the request-aware `sign_in_session(response, req, session, options?)` path or a higher-level primitive that delegates to it, not create a second session-completion model.

**Value:** the most repetitive and mistake-prone login/logout/cookie code disappears.

#### Wave 3 — Staged Auth
**Shipped:**
- `begin_auth_challenge()`
- `current_auth_challenge()`
- `complete_auth_challenge()`
- `cancel_auth_challenge()`
- one-time, TTL-bound pending auth state distinct from real sessions

**Value:** multi-step auth becomes a normal pattern instead of ad hoc glue code.

#### Wave 4 — Internal Architecture Cleanup
**Shipped:**
- module split into coherent auth internals
- documented fallback/error semantics for existing state families
- internal storage contract across sessions/challenges/states/tokens
- backend contract-test harness + env-gated Postgres/Redis verification

**Follow-up:** split storage further before adding local-auth durable record families.

**Value:** makes the next auth features safer to build instead of more expensive every time.

#### Wave 5 — Session Lifecycle
**Shipped:**
- sliding expiration
- absolute max lifetime
- lifecycle presets (`consumer`, `admin`, `internal`, `strict`)
- preset + override merge
- cookie refresh alignment for built-in/auth-enforced response paths

**Value:** apps get secure lifecycle behavior without each team inventing different timeout logic.

#### Wave 6 — OAuth Hardening + Observability
**Shipped:**
- refresh token rotation/preservation semantics
- safe refresh-token rotation logging
- auth health check endpoint / diagnostics

**Value:** security and operability improve together, which is the right trade for production auth.

#### Wave 7 — Auto-Routes + UI Convenience
**Shipped:**
- configurable auth route prefixes
- startup route logging
- collision diagnostics
- built-in login page generator

**Remaining:** full auto-mount/override behavior and custom HTML override support.

**Value:** fast starts for simple apps, while custom apps can still own their UX.

#### Wave 8 — Advanced Sessions Foundation
**Shipped:**
- device/session metadata plumbing
- keyed hashes for user-agent/IP security signals
- remember-me flag storage plumbing
- backend migration hardening for metadata fields

**Remaining:** visible session-management APIs, behavioral remember-me TTLs, security event storage, suspicious-activity policy, and admin revocation APIs.

**Value:** moves `std/auth` from solid to category-leading, provided the plumbing is turned into product behavior in later slices.

#### Wave 8.5 — Architecture Guardrails Preflight
**Shipped in PR #98:**
- focused `auth/storage/local.rs` home for the first local-auth record families
- explicit local-auth record-family enum and fail-closed fallback policy baseline
- memory/SQLite local identity + credential contract coverage in module-local tests

**Still needed:**
- auth contributor/review guidance
- reusable contract-test ratchet for every new auth record family
- visible Postgres/Redis backend coverage instead of env-gated silence

**Value:** prevented the first local-auth slice from undoing the architecture cleanup. The ratchet still needs teeth, because vibes are not a test harness.

#### Wave 9 — First-Class Local Auth
**Shipped so far in PR #98:**
- local identity/account-state model
- memory and SQLite local identity/credential storage
- public `verify_local_password(identifier, password, options?)` with safe non-enumerating failure behavior
- generated stdlib docs and AI agent guide examples for verify-then-`sign_in_session` usage

**Landing in `feat/local-auth-sign-in-bootstrap`:**
- first-class `local_sign_in(response, req, credentials, session?, options?)` domain operation that delegates to request-aware session completion or staged auth challenges
- `create_local_user(...)` and exactly-once `bootstrap_local_user(...)` provisioning with forced setup/credential-rotation semantics

**Still ships next:**
- completion handlers for first-login setup / forced password change and staged TOTP enrollment
- password reset token lifecycle
- local-auth backend CI/contracts beyond the memory/SQLite local credential baseline, plus template migration away from custom auth persistence

**Value:** closes the gap between great auth primitives and great auth architecture. This branch lands the provisioning and request-aware sign-in entry points; the remaining work is the full setup/reset/TOTP lifecycle after those staged continuations.

#### Wave 10 — Ongoing Architecture Discipline
**Ships continuously:**
- module-boundary enforcement
- complexity budgets for `auth.rs` and `auth/storage.rs`
- backend contract coverage for every new auth state type
- periodic architecture audits and review-guidance updates

**Value:** protects the cleanup/local-auth investment so `std/auth` stays understandable instead of becoming a heroic pile of security-sensitive features.

## Competitive Landscape

This table tracks the intended competitive posture after the v0.4.9 auth work and the DD-043/062 roadmap. "ntnt today" means current `main` behavior; "roadmap" means the target state after the remaining DD-043 waves.

| Feature | ntnt today (v0.4.9) | ntnt roadmap | Auth.js v5 | Lucia v3 | Laravel Sanctum | Django Auth |
|---------|:---:|:---:|:---:|:---:|:---:|:---:|
| OAuth/OIDC | ✅ | ✅ | ✅ | ❌¹ | ❌² | ❌² |
| PKCE | ✅ | ✅ | ✅ | ❌¹ | ❌ | ❌ |
| Server sessions | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Signed cookies | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ |
| CSRF auto-protection | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| Token auto-refresh | ✅ | ✅ | ✅ | ❌¹ | ❌ | ❌ |
| Session rotation | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ |
| Sliding sessions | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Absolute max session lifetime | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |
| Refresh token rotation/preservation | ✅ | ✅ | ✅ | ❌¹ | ❌ | ❌ |
| Device/session metadata plumbing | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Device-session management APIs | ◐ | ✅ | ❌ | ❌ | ❌ | ◐ |
| Config presets | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Health check endpoint | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Suspicious activity policy | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Behavioral remember-me TTL | ❌ | ✅ | ✅ | ◐ | ✅ | ✅ |
| TOTP/MFA primitives | ✅ | ✅ | ❌ | ❌¹ | ❌ | ❌³ |
| First-class local email/password/TOTP lifecycle | ◐ | ✅ | ❌ | ❌ | ◐ | ✅ |
| Auth-owned password reset tokens | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ |
| Bootstrap admin local auth | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ |
| Safari ITP workaround | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Zero-config setup | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Auto route registration / auth route prefixing | ◐ | ✅ | ✅⁴ | ❌ | ❌ | ✅ |
| Startup config summary | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Config validation | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Built-in route protection | ✅ | ✅ | ✅⁵ | ❌ | ✅ | ✅ |
| Built-in login page | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |
| 0 boilerplate files for common local admin auth | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |
| One-line setup | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |

¹ Lucia is sessions-only — OAuth/tokens/MFA are app-level
² Requires additional packages (Socialite, django-allauth)
³ Requires django-otp
⁴ Auth.js uses file-system convention (`[...nextauth]/route.ts`) — auto but requires specific file structure
⁵ Auth.js middleware.ts works but is a separate file with its own config — not integrated into the auth call

---

## Success Criteria

We'll know this is "the best auth system ever made for any language" when:

1. **Zero-config security:** `enable_auth([google])` with no options map is already production-safe.
2. **Common local auth is first-class:** a local admin/app flow with email/password, TOTP setup, password reset, bootstrap account creation, forced password change, and session creation does not require custom app-owned credential tables.
3. **One auth system, not two:** OAuth, local credentials, staged setup, password reset, and future step-up flows all converge on the same session/cookie/lifecycle/security-event model.
4. **Fail-closed durable credentials:** local credential, reset, and TOTP enrollment state never silently degrades to process memory in production when the configured durable backend fails.
5. **OWASP session-management posture by default:** session rotation, idle timeout, absolute lifetime, secure cookies, CSRF posture, revocation, and fixation defenses are built in and hard to misconfigure.
6. **OAuth security BCP alignment:** OAuth/OIDC flows use PKCE, state, nonce where appropriate, refresh-token rotation/preservation, safe diagnostics, and no token leakage in logs or health endpoints.
7. **Observable without leaking secrets:** developers can see exactly what auth is doing and whether it is configured correctly, while diagnostics never expose token/secret material.
8. **Backend behavior is contractual:** every auth state family has contract tests for memory/SQLite plus required Postgres/Redis coverage when relevant, including fallback/error behavior.
9. **Architecture resists entropy:** new auth features have obvious module homes, storage contracts, generated docs, typechecker signatures, and review checklist coverage.
10. **< 30 seconds to auth:** from `ntnt new` to authenticated app in under 30 seconds of developer time for OAuth or common local admin auth.
11. **Competitive table is mostly green:** ntnt matches the mainstream auth systems on core capability and beats them on stdlib cohesion, observability, local-auth ergonomics, and boilerplate.

---

*This is a living document. Update checkboxes, roadmap status, and review guidance as implementation progresses.*
