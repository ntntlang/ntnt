# DD-062: First-Class Local Email/Password/TOTP Auth in `std/auth`

**Status:** Draft / implementation blueprint
**Parent:** DD-043 Phase 9 — First-Class Local Auth
**Target:** v0.4.x auth roadmap after the v0.4.9 DD-043 foundation

---

## Problem

`std/auth` already owns a strong set of auth primitives:

- OAuth/OIDC providers and callback handling
- signed session cookies
- server-side session stores
- staged auth challenges
- route protection
- CSRF posture
- session lifecycle controls
- request/session metadata plumbing
- TOTP primitives
- sign-in/sign-out/current-session helpers

But for a very common real app shape — a local admin area using **email + password + TOTP** — developers still have to build and maintain their own mini auth system around those primitives:

- local user/credential table
- password-hash storage and verification
- password reset tokens
- TOTP enrollment state
- bootstrap admin account creation
- first-login / forced-password-change flows
- local login route handlers
- ad hoc admin/session claim handoff from custom local auth into `std/auth` sessions

That is exactly the wrong level of abstraction.

A template app should not need a custom `lib/admin_db.tnt` auth subsystem just to get a normal local admin login. If the language already owns sessions, cookies, staged challenges, TOTP primitives, and protected-route enforcement, it should also own the common local credential lifecycle that feeds those mechanisms.

---

## Goal

Make **local email/password/TOTP auth** a first-class `std/auth` path so apps configure or call primitives instead of rebuilding security-sensitive lifecycle state.

Target outcome:

- no custom credential/account tables in the template
- no template-owned password login state machine
- no template-owned password-reset token model
- no template-owned staged-setup persistence model
- no request-less local sign-in path that misses session rotation or request metadata
- template becomes mostly auth config + views/copy + app-specific authorization policy

This DD is not about turning `std/auth` into a hosted identity product or a universal app-account framework. It draws a hard boundary:

| Layer | Belongs in `std/auth` | Belongs in the app/plugin |
|---|---|---|
| **Primitives** | local credential records, password verification orchestration, reset-token issue/consume, TOTP enrollment state, staged auth continuations, request-aware session completion, backend contracts | profile fields, org membership, billing/account management, invite/approval policy |
| **Reference flows** | optional bootstrap/login/reset/setup routes and minimal pages built from the primitives | custom UI, copy, email/SMS delivery, onboarding decisions |
| **Policy hooks** | explicit hooks/options for deriving session data/claims and choosing reset/setup consequences | static universal roles/claims embedded in local-auth config, hidden risk-policy engines |

---

## Relationship to DD-043

DD-062 is the detailed child plan for DD-043 Phase 9.

Current 0.4.9 baseline: the shared manual/staged session completion path is already request-aware. `sign_in_session(response, req, session, options?)` and `complete_auth_challenge(response, req, session?, options?)` rotate/migrate existing sessions and capture request-derived metadata. Local auth should consume that path rather than introduce its own session creation semantics.

| DD-043 Phase | DD-062 Section | Purpose |
|---|---|---|
| 9A — Architecture Preflight and Storage Boundary | Phase 0 | Prepare module/storage/test boundaries before adding durable local-auth state |
| 9B — Local Identity and Credential Store | Phase 1 | Add auth-owned local identity/account/credential records |
| 9C — Request-Aware Local Sign-In Flow | Phase 2 | Verify email/password and create sessions with OAuth-equivalent lifecycle semantics |
| 9D — First-Login Activation, Forced Password Change, and TOTP Enrollment | Phase 3 | Model setup and MFA enrollment through staged auth, not app-owned state machines |
| 9E — Password Reset Lifecycle | Phase 4 | Add reset token issue/consume/reset semantics |
| 9F — Template-Grade Integration | Phase 5 | Migrate template off custom local-auth persistence |
| 9G — Local Auth Verification Matrix | Phase 6 | Ratchet backend contracts, end-to-end flows, docs, and CI |

---

## Design Principles

1. **One auth system, not two.** Local auth, OAuth, staged setup, password reset, and future step-up flows must converge on the same session/cookie/lifecycle/security-event model.
2. **Durable credential state fails closed.** Local credentials, reset tokens, TOTP enrollment state, bootstrap state, and account-state changes must not silently degrade to process memory in production when the configured durable backend fails.
3. **Request-aware session creation.** Local login must receive the request so it can rotate/migrate existing sessions, capture `device_name`, `user_agent_hash`, and `last_ip_hash`, and apply the same cookie/session TTL behavior as OAuth.
4. **Staged auth is the continuation model.** Password → TOTP, first-login setup, forced password change, and reset recovery should use existing staged challenge semantics where appropriate.
5. **Local identity is not a profile platform.** Store the minimum durable auth state. App profile data remains app-owned.
6. **Storage behavior is a compatibility surface.** Every local-auth record family needs contract tests across backends at the same time it is introduced.
7. **UI stays optional.** `std/auth` may offer default routes/pages, but custom UI must not require custom credential/session/reset persistence.

---

## Preferred Developer Experience

Primary shape: local auth is a primitive family used by `enable_auth(...)` for shared config/diagnostics and by request-aware helpers for custom UI. Reference routes can be generated from the same primitives, but the primitives must remain usable without accepting built-in UI or app policy.

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

Rationale:

- Developers should still have one obvious auth configuration entrypoint: `enable_auth(...)`.
- Local credentials should share route prefixes, cookies, sessions, lifecycle presets, protected paths, health diagnostics, startup summaries, and backend contract tests.
- `sign_in_session(response, req, session, options?)` is intentionally request-aware so local/manual auth gets OAuth-equivalent session rotation and request metadata capture.
- Claims/session data should come from app hooks or explicit session-completion data, not static authorization policy buried in credential config.
- Email is the first identifier preset, not the universal identity model.

Acceptable later convenience:

```ntnt
enable_local_auth(map {
    "preset": "admin",
    "identifier": "email",
    "totp": true,
    "password_reset": true
})
```

If added, this must delegate to the same underlying primitive/config path. It must not become a parallel auth subsystem or quietly own signup policy, onboarding, email delivery, profile data, roles/orgs, or account-management UI.

---

## Public API Direction

Names are still draft, but the capability shape should be stable.

### Configuration

- `local_credentials(config: Map) -> LocalCredentialConfig` (or equivalent final name)
- local credential config accepted through `enable_auth(..., options)` so it shares auth diagnostics, stores, route prefixes, protected-path behavior, and lifecycle presets
- optional reference route/page generation can wrap the same primitive config, but custom UI must call the same verification/session-completion helpers
- startup summary includes local-auth status without exposing secrets
- `/auth/health` reports local-auth configuration posture without exposing hashes, reset tokens, TOTP secrets, or bootstrap password material

### Local identity/admin helpers

Possible helpers; exact names can change during implementation:

- `local_user(identifier) -> Result<LocalUser?, String>`
- `create_local_user(identifier, options) -> Result<LocalUser, String>`
- `disable_local_user(identifier_or_id) -> Result<Unit, String>`
- `require_password_change(identifier_or_id) -> Result<Unit, String>`
- `set_local_password(identifier_or_id, new_password, options?) -> Result<Unit, String>`

These should be intentionally small. App profile and authorization data should remain app-owned. Session claims should be supplied through explicit hooks/session-completion data, not static universal `claims` config on the credential provider.

### Local sign-in/session helpers

The built-in local sign-in route can be enough for simple apps, but custom login forms need a safe lower-level helper.

The helper must be request-aware. A shape in this family is acceptable:

```ntnt
local_sign_in(req, response, map {
    "email": email,
    "password": password,
    "remember_me": remember_me
})
```

It should return either:

- a completed signed-in response, or
- a staged-auth continuation response for TOTP/setup/password-change, or
- a safe auth error result

It must delegate to request-aware `sign_in_session(response, req, session, options?)` or the same internal session-completion primitive so local login receives OAuth-equivalent rotation, metadata capture, cookie, TTL, and lifecycle behavior. It should not directly create sessions, attach cookies, or bypass the shared completion path.

### Password reset helpers

- `issue_password_reset(email, options?) -> Result<PasswordReset, String>`
- `consume_password_reset(token, new_password, options?) -> Result<PasswordResetResult, String>`
- default routes can wrap these helpers, but apps/plugins own email delivery

`std/auth` owns token generation, token hashing/storage, one-time consume, TTL, replay rejection, and reset consequences. Apps own the email/SMS/provider used to deliver reset links.

---

## Internal Architecture Target

The current auth code already has modules, but local auth should not expand the remaining monoliths.

Target ownership:

```text
src/stdlib/auth.rs                     public exports, registration glue, shared surface only
src/stdlib/auth/types.rs               shared public/internal structs and value conversions
src/stdlib/auth/local.rs               local-auth domain operations
src/stdlib/auth/local_passwords.rs     password verification/hash policy wrappers if needed
src/stdlib/auth/local_totp.rs          TOTP enrollment/reset domain logic if it outgrows primitives
src/stdlib/auth/password_reset.rs      reset token domain operations
src/stdlib/auth/sessions.rs            shared session lifecycle and request-aware sign-in completion
src/stdlib/auth/storage/mod.rs         storage contract boundary
src/stdlib/auth/storage/session.rs     session records
src/stdlib/auth/storage/challenge.rs   staged challenge records
src/stdlib/auth/storage/oauth.rs       OAuth state records
src/stdlib/auth/storage/exchange.rs    Safari/session exchange tokens
src/stdlib/auth/storage/local.rs       local identity/credential/TOTP/reset/bootstrap records
```

Exact file names can change. The important rule: durable local-auth state must have a clear storage/domain home and must not be hidden inside generic challenge `data_json` or appended casually to `auth.rs`.

---

## Local Auth Data Model

Keep this lean. The goal is auth state, not profiles.

### Local identity/account

Minimum durable shape:

- `id` — stable internal local subject id
- `identifier_kind` — e.g. `email`; extensible beyond the first preset
- `identifier` — canonical display/source identifier for the chosen kind
- `identifier_normalized` — normalized lookup key for the chosen kind
- `created_at`
- `updated_at`
- `state` — `bootstrap`, `pending_setup`, `active`, `disabled`, `locked`, `password_change_required`
- `metadata_json` — small auth-owned metadata needed for auth lifecycle only; not app profile data, roles, permissions, organizations, or session claims

Do **not** store app authorization claims/roles on the local identity as part of the primitive data model. Claims for sessions should come from an app-owned hook/helper at session completion, e.g. `app_claims_for_local_user(verified)`, so `std/auth` owns credential lifecycle without becoming the app's authorization database.

### Credential secret

- `local_user_id`
- `password_hash`
- `password_hash_algorithm`
- `password_hash_params_json` or encoded hash metadata
- `password_changed_at`
- `must_change_password`
- optional future `password_rehash_required`

Password hashing may continue to use `std/crypto` internally, but local auth owns storage, verification orchestration, and hash-upgrade policy.

### TOTP enrollment

- `local_user_id`
- `state` — `not_enrolled`, `pending_enrollment`, `enrolled`, `reset_required`
- encrypted/secret TOTP material or storage-compatible secret representation
- `created_at`
- `verified_at`
- `last_reset_at`

Do not store durable TOTP enrollment state only in staged challenge `data_json`. Challenges are pending flow state; enrollment is account state.

### Password reset token

- token id / selector
- token hash, never raw token
- `local_user_id`
- `created_at`
- `expires_at`
- `consumed_at`
- reset consequences: force password change, clear TOTP, require TOTP re-enrollment, revoke sessions

Reset consume must be one-time and atomic per backend.

### Bootstrap state

- bootstrap email/config fingerprint
- created local user id
- `created_at`
- `consumed` / `rotated` / `setup_completed` marker

Bootstrap passwords should be treated as temporary. Successful bootstrap login should force password rotation and/or setup completion based on config.

---

## Fallback and Error Policy

Existing DD-043 fallback semantics allow memory fallback for some transient state. Local auth needs a stricter policy.

| Record family | Store failure | Lookup failure | Consume/update failure | Memory fallback? |
|---|---|---|---|---|
| Local identity/account | fail closed | fail closed | fail closed | dev/test only if explicitly configured |
| Credential secret/hash | fail closed | fail closed | fail closed | no production fallback |
| TOTP enrollment state | fail closed | fail closed | fail closed | no production fallback |
| Password reset token | fail closed | fail closed | fail closed; atomic consume required | no production fallback |
| Bootstrap state | fail closed | fail closed | fail closed | dev/test only if explicitly configured |
| Staged local challenge | existing challenge policy, but TTL/consume must be tested | existing challenge policy | existing challenge policy | acceptable for transient challenge state only |
| Session after local login | existing session policy | existing session policy | migrate/update/delete errors propagate | existing session policy |

This table should become implementation comments and contract tests, not just documentation confetti.

---

## Phased Implementation Plan

### Phase 0 — Architecture Preflight

**Purpose:** make the local-auth implementation path obvious before adding durable credential state.

- [ ] Split or carve `auth/storage.rs` enough to give local-auth storage a focused home
- [ ] Move or isolate the auth storage contract harness so local-auth record families can reuse it cleanly
- [ ] Add local-auth fallback/error policy comments near the storage contract boundary
- [ ] Add auth review checklist entries for local-auth-specific regressions
- [ ] Decide exact module names and public API names before implementation starts

**Exit criteria:** no one has to guess where local identity, password reset, TOTP enrollment, or bootstrap state belongs.

### Phase 1 — Local Identity and Credential Store

**Purpose:** add durable local users/credentials owned by `std/auth`.

- [ ] Create the local identity/account model
- [ ] Create credential-secret storage and verification helpers
- [ ] Normalize email lookup rules and document them
- [ ] Add account states: bootstrap, pending setup, active, disabled, locked, password-change-required
- [ ] Support bootstrap account creation from config/env
- [ ] Force bootstrap credential rotation/setup completion according to config
- [ ] Add memory/SQLite contract tests by default
- [ ] Add Postgres/Redis contract coverage in backend CI
- [ ] Add migration tests for existing SQLite/Postgres stores

### Phase 2 — Request-Aware Local Sign-In

**Purpose:** let email/password login produce sessions with the same safety posture as OAuth login.

**Baseline:** request-aware manual session completion already exists in 0.4.9 branch work. This phase should wire local credential verification into that primitive, not design a new cookie/session attachment path.

- [ ] Add local password verification through `std/auth`
- [ ] Add local sign-in domain operation that delegates to `sign_in_session(response, req, session, options?)` or the same internal primitive
- [ ] Rotate/migrate existing sessions on successful local login
- [ ] Capture `device_name`, `user_agent_hash`, and `last_ip_hash` from request metadata
- [ ] Apply configured session TTL, max lifetime, sliding expiry, cookie policy, and remember-me behavior
- [ ] Return staged continuation when TOTP/setup/password-change is required
- [ ] Add tests proving local login does not lose session metadata compared with OAuth login

### Phase 3 — First Login, Forced Password Change, and TOTP Enrollment

**Purpose:** make setup and MFA enrollment native rather than app-owned state machines.

- [ ] Add staged first-login setup flow
- [ ] Add forced-password-change flow before final session completion
- [ ] Add TOTP enrollment challenge flow
- [ ] Persist TOTP enrollment state explicitly after verification
- [ ] Define TOTP reset/re-enrollment semantics
- [ ] Ensure no protected-route access is granted until setup requirements complete
- [ ] Add end-to-end tests for bootstrap → password change → TOTP enrollment → session completion

### Phase 4 — Password Reset Lifecycle

**Purpose:** make reset tokens secure, backend-consistent, and app-ergonomic.

- [ ] Add auth-owned reset token issue helper
- [ ] Store only reset token hashes/selectors, never raw reset tokens
- [ ] Add atomic one-time reset token consume helper
- [ ] Add reset replay rejection tests
- [ ] Add reset expiry tests
- [ ] Define reset consequences: session revocation, forced password change, optional TOTP reset/re-enrollment
- [ ] Define email-delivery boundary and examples
- [ ] Add failed-login and reset-attempt rate limiting hooks or built-in policy

### Phase 5 — Template-Grade Integration

**Purpose:** prove the abstraction by deleting the custom local-auth subsystem from a real template.

- [ ] Migrate the Larri site template to `std/auth` local auth
- [ ] Delete template-owned local credential tables/state machines once parity exists
- [ ] Keep custom login/setup/reset views possible without custom auth persistence
- [ ] Document the before/after diff as proof the system got simpler
- [ ] Add examples for custom UI with built-in local-auth persistence

### Phase 6 — Verification and Documentation Ratchet

**Purpose:** make local auth hard to regress.

- [ ] Contract tests for every local-auth record family across memory and SQLite by default
- [ ] Required Postgres/Redis backend contract CI job or explicit non-silent skip reporting
- [ ] End-to-end ntnt tests for bootstrap, normal login, TOTP setup, forced password change, reset issue/consume, reset replay, disabled account rejection, and session revocation
- [ ] Generated stdlib docs for every public helper
- [ ] `docs/AI_AGENT_GUIDE.md` examples for local auth
- [ ] DD-043 status/checklist updates after each implementation slice
- [ ] Review checklist updates for any bug class discovered during implementation

---

## Testing Matrix

| Behavior | Unit | Storage contract | End-to-end ntnt/server | Backend CI |
|---|:---:|:---:|:---:|:---:|
| Email normalization/lookup | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Bootstrap account creation | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Password verify success/failure | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Disabled/locked account rejection | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Password-change-required continuation | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Session rotation on local login | ✅ | — | ✅ | all session backends |
| Device/IP/UA metadata capture | ✅ | ✅ | ✅ | all session backends |
| TOTP enrollment/setup completion | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Password reset issue/consume | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Reset replay rejection | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Reset expiry | ✅ | ✅ | ✅ | SQLite/Postgres/Redis |
| Reset-triggered session revocation | ✅ | ✅ | ✅ | all session backends |
| Backend failure fail-closed behavior | ✅ | ✅ | — | SQLite/Postgres/Redis |

---

## Non-Goals

- Building a full end-user identity/profile platform
- Owning arbitrary app profile fields
- Solving every RBAC/ABAC/organizations use case before shipping local auth
- Owning public signup UX, invite policy, onboarding copy, or email templates
- Sending email/SMS directly from `std/auth`
- Making templates keep custom auth persistence “for flexibility” once `std/auth` can own the common case

Clarification: local credential storage, password verification, reset tokens, bootstrap setup, and TOTP enrollment are **in scope**. The app/product workflows around those mechanics remain app-owned.

---

## Acceptance Criteria

DD-062 is complete when:

1. A template can configure local email/password/TOTP auth without creating custom credential/reset/TOTP tables.
2. Local login creates sessions with the same cookie, lifecycle, rotation, metadata, and revocation semantics as OAuth login.
3. Password reset tokens are one-time, TTL-bound, hash-stored, backend-consistent, and replay-tested.
4. TOTP enrollment state is durable and explicit, not hidden only in challenge payloads.
5. Durable credential/reset/TOTP state fails closed on backend errors in production.
6. Memory/SQLite contract tests run by default and Postgres/Redis coverage is visible in CI.
7. The Larri site template deletes its app-owned local-auth mini-system and gets simpler.
8. Public docs make the easy path obvious and the app-owned boundaries explicit.

---

## Recommendation

Treat local auth as the next practical `std/auth` simplification step, but do not implement it as a pile of helpers.

The right implementation order is:

1. architecture/storage/test preflight
2. local identity and credential store
3. request-aware local sign-in
4. staged setup/TOTP
5. password reset
6. template migration
7. contract/docs/CI ratchet

If `std/auth` can own OAuth well but still cannot cleanly own a normal local admin login, the library remains incomplete in a way users feel immediately. If it owns local auth with the same discipline as OAuth/session/challenge state, it becomes genuinely hard to beat.
