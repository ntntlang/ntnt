# DD-043: World-Class Auth — Making `std/auth` the Best stdlib Auth Ever

**Status:** In Progress — Final Sprint
**Author:** Larri
**Date:** 2026-03-20 (slashed and sharpened 2026-04-28; WebAuthn/passkeys phase added 2026-04-29)
**Branch:** `main` through v0.4.9; remaining v0.4.9 work is PR 3 + template proof; WebAuthn/passkeys are post-v0.4.9

---

## Vision

`std/auth` should be the small, secure auth foundation that lets real apps stop rebuilding auth from scratch.

It owns authentication, session lifecycle, staged auth, credential verification, reset/TOTP primitives, API-auth enforcement hooks, and safe extension seams. Apps own product policy: roles, groups, permissions, UI, email delivery, onboarding, organizations, and business decisions.

**Design principle:** the secure primitive should be the obvious primitive. OAuth, local credentials, staged setup, password reset, TOTP, API endpoints, and app authorization should all compose through one session/challenge/auth lifecycle model — not through parallel app-owned auth tables.

### Ownership Boundary

| Layer | `std/auth` owns | App owns |
|---|---|---|
| **Authentication primitives** | OAuth/OIDC, signed cookies, sessions, CSRF, route/API protection, staged challenges, local credential storage/verification, password reset token lifecycle, TOTP enrollment/verification helpers, request-aware session completion | Login page design, copy, email/SMS delivery, product-specific onboarding |
| **Auth lifecycle metadata** | Device/IP/UA metadata, remember-me plumbing, local account state, TOTP enrollment state, password-reset state, safe local-user metadata helpers | App profile fields, business state, invite/approval state |
| **Authorization seam** | Session data conventions and small helpers for `group_ids` / claims handoff | RBAC/ABAC rules, group definitions, org membership, permission checks beyond the helpers |
| **Extension model** | Namespaced `metadata_json` on auth-owned models, exposed through safe helpers | App-level authorization policy and app-specific metadata contents |

The rule is not “metadata for everything.” The rule is:

1. **Security-critical lifecycle that must be atomic or non-replayable belongs in `std/auth` helpers/storage.** Password reset tokens are the obvious example.
2. **App-specific authorization context belongs in session data or namespaced metadata.** Group IDs, role hints, and profile-ish state do not need new stdlib tables.
3. **No parallel auth system in templates.** The template may extend `std/auth`; it should not recreate users, credentials, TOTP, sessions, or reset state beside it.

---

## Current State (v0.4.9) — What Is Shipped

Everything below is merged or intended as the current v0.4.9 baseline:

- OAuth 2.0 + OIDC providers with PKCE
- Server-side sessions using memory, SQLite, PostgreSQL, or Redis/Valkey for the main session/challenge/OAuth state families
- HMAC-signed session cookies
- CSRF protection
- Secure cookie defaults: `HttpOnly`, `Secure`, `SameSite=Lax`
- Configurable session TTLs, sliding expiry, absolute max lifetime, and presets (`consumer`, `admin`, `internal`, `strict`)
- Request-aware `sign_in_session(response, req, session, options?)`
- `sign_out_session()`, `current_session()`, `current_user()`
- Session metadata: `device_name`, `user_agent_hash`, `last_ip_hash`, `remember_me`
- Current-user session management: `user_sessions(req)`, `logout_all(req, keep_current)`
- Staged auth challenge primitives: `begin_auth_challenge()`, `current_auth_challenge()`, `complete_auth_challenge()`, `cancel_auth_challenge()`
- TOTP primitives: `totp_secret()`, `verify_totp()`, `totp_uri()`
- Local TOTP helpers: `begin_totp_enrollment(...)`, `confirm_totp_enrollment(...)`, `verify_local_totp(...)`, `totp_status(...)`, `reset_totp(...)`
- Route/API protection: `require_auth()` middleware/path/request helper with HTML redirect vs API 401 behavior
- Bearer-token/resource-server helpers: `oauth_validate(...)`, `oauth_introspect(...)`
- Built-in login page with configurable title/logo/copy/provider buttons
- No WebAuthn/passkey support yet; that is intentionally post-v0.4.9, after PR 3 and the template integration proof
- Configurable route prefixes, startup route logging, collision diagnostics
- Auth health check endpoint, dev-only by default
- Refresh-token rotation/preservation with provider-aware semantics
- Safari ITP workaround through a two-phase exchange-token flow
- Local identity/credential store owned by `std/auth`
- `verify_local_password(identifier, password)` credential verification
- `bootstrap_local_user(identifier, password, options?)` bootstrap provisioning
- `set_local_password(identifier, current_password, new_password, options?)` password rotation/setup completion
- Internal module split: config, cookies, providers, OAuth, guards, routes, request helpers, sessions, storage, primitives, utilities
- Memory/SQLite coverage for local identity/credential paths; Postgres/Redis local credential backends are future unless explicitly implemented in the final sprint

---

## Final-Sprint Target

The final sprint should make `std/auth` complete enough that `template.heylarri.com` can build one unified auth system on top of it:

1. Local email/password auth uses `std/auth` local identity + credential records.
2. Password setup/change uses `set_local_password(...)`.
3. TOTP setup/verification uses `std/auth` helpers and auth-owned metadata, not a template-owned TOTP model.
4. Password reset uses `std/auth` reset-token helpers with hashed, one-time, TTL-bound tokens.
5. Authorization context uses app-supplied `group_ids` / claims in session data, not a stdlib RBAC schema.
6. Admin pages and API endpoints use the same `require_auth(...)` / session / group helpers.
7. The template deletes parallel auth tables and keeps only app-specific policy and UI.

### Extension Contract: Metadata Without the Junk Drawer

`metadata_json` is a server-side extension bag, not a public dumping ground.

Rules:

- Metadata must be namespaced. Use reserved `std/auth` namespaces such as `auth.totp` only through stdlib helpers; use app namespaces such as `app.groups` or `template.*` for app data.
- Raw password reset tokens must never be stored in metadata. Reset tokens are selector/hash/TTL/consume records managed by `std/auth`.
- TOTP secret material must never appear in safe local-user payloads, current-user maps, templates, logs, or health diagnostics.
- App authorization data may be stored as group IDs or claims, but app policy decides what those IDs mean.
- Metadata helpers must merge/replace deliberately and validate JSON shape. No blind client-controlled metadata writes.
- The safe local-user payload should include only non-secret identity/account fields and explicitly allowed metadata views.

---

## Remaining Work (Fast Path — 2-3 PRs)

### PR 1 — Metadata + Authorization Context Polish

Goal: make the extension seam explicit and safe without inventing a full RBAC system.

- [x] Expose local identity `metadata_json` through safe read/update helpers: `local_user(identifier, options?)` and `update_local_user_metadata(identifier, metadata, options?)`
- [x] Define reserved metadata namespaces:
  - `auth.totp` for stdlib-managed TOTP enrollment state
  - `auth.reset` only if needed for non-token reset metadata; raw/reset-token hashes live in reset-token storage
  - app-owned namespaces such as `app.*` or `template.*`
- [x] Add a standard session-data convention for authorization context:
  - `group_ids: [String]`
  - optional `claims: Map`
- [x] Add small authorization helper for apps/API endpoints: `has_group(session_or_req, group_id_or_ids)` inspects session data and does not own RBAC policy
- [x] Document canonical composition:
  - `verify_local_password(...)`
  - app derives `group_ids` / claims
  - `sign_in_session(response, req, map { "subject_id": ..., "data": map { "group_ids": [...] } })`
- [x] Add tests proving local login receives the same session metadata (`device_name`, `user_agent_hash`, `last_ip_hash`) as OAuth/manual session completion
- [x] Add examples for protecting HTML routes and JSON/API endpoints with `require_auth(...)` and `has_group(...)`

### PR 2 — Strong TOTP Support

Goal: make TOTP a real local-auth extension path without requiring template-owned TOTP tables.

- [x] Add TOTP enrollment helpers around the existing primitives: `begin_totp_enrollment(...)`, `confirm_totp_enrollment(...)`, `verify_local_totp(...)`, `totp_status(...)`, `reset_totp(...)`
- [x] Store TOTP enrollment state under the reserved auth metadata namespace on the local identity
- [x] Keep TOTP secrets server-side and absent from safe payloads/current-user/template data; only one-time setup URI material is returned by enrollment
- [x] Document staged auth challenge composition for password → TOTP and setup-required continuations
- [x] Ensure examples do not grant protected-route/API access until required setup/TOTP steps complete
- [x] Add tests for enrollment, verification, reset/re-enrollment, disabled/locked account rejection, and no-secret leakage

### PR 3 — Password Reset Essentials

Goal: ship reset password securely enough to use, without owning email delivery or account UI.

- [ ] Add `issue_password_reset(identifier, options?) -> Result<PasswordReset, String>`
- [ ] Add `consume_password_reset(token, new_password, options?) -> Result<PasswordResetResult, String>`
- [ ] Store only token selectors/hashes, never raw tokens
- [ ] Enforce TTL, one-time consume, replay rejection, and generic responses that do not enumerate accounts
- [ ] Define reset consequences as explicit options/defaults: revoke sessions, force password change, clear/reset TOTP enrollment
- [ ] Apps/plugins own email/SMS delivery; `std/auth` returns the token/safe URL material to deliver
- [ ] Add memory/SQLite tests by default; clearly document/skip Postgres/Redis local reset support unless implemented

### Template Integration Proof

This may be a separate template-repo PR, but it is the real acceptance test.

- [ ] `template.heylarri.com` uses built-in local identity, credential, session, challenge, TOTP, and reset primitives
- [ ] Template stores app-specific auth extension data in namespaced metadata or session data
- [ ] Template uses `group_ids` / claims to build app RBAC without stdlib owning app policy
- [ ] Template protects both pages and API endpoints through `require_auth(...)` / group helpers
- [ ] Template deletes parallel auth tables/state machines
- [ ] The before/after diff is materially simpler

---

## Post-v0.4.9 Phase — WebAuthn + Passkeys

This phase is intentionally **after** PR 3, the template integration proof, and the v0.4.9 auth release. It should not delay 0.4.9. The goal is to add phishing-resistant WebAuthn/passkey primitives that compose with the same `std/auth` local identity, staged challenge, request-aware session, and route/API protection model.

Design posture:

- Treat passkeys as another credential family under `std/auth`, not a separate user system.
- Use standards-shaped ceremonies: server starts a registration/authentication ceremony, stores challenge state server-side, browser calls `navigator.credentials.create/get`, server finishes and consumes state.
- Prefer established Rust implementation work such as `webauthn-rs` rather than home-grown WebAuthn verification. The library docs explicitly warn that ceremony state must be stored server-side and that credential IDs must be globally unique across accounts.
- Store relying-party configuration explicitly: `rp_id`, allowed origins, display name, timeout, user-verification policy, attestation policy, and discoverable-credential policy.
- Use opaque, non-PII user handles for WebAuthn `user.id`; WebAuthn user handles are capped at 64 bytes and are returned by discoverable/usernameless flows.
- Track credential metadata needed for security and UX: credential ID, public key/passkey object, sign counter, transports, user verification, backup eligibility/state when available, created/last-used timestamps, nickname, and disabled/revoked status.
- Enforce credential-ID uniqueness across all users before accepting registration.
- On authentication, validate counters where the authenticator supplies meaningful counters; equal/lower non-zero counters must reject the current authentication attempt. After rejection, disable or quarantine the credential unless explicit policy says to only alert.
- Keep attestation and enterprise hardware-bound policy optional. Consumer passkeys should work without attestation; regulated/high-assurance apps can opt into attestation policy later.
- Do not serialize WebAuthn ceremony state to client cookies. If a dependency exposes such a feature, keep it off by default; state belongs in auth-owned server storage with TTL and one-time consume.

### WA-PR 1 — Foundation, Config, and Storage Contract

Goal: add the auth-owned record families and dependency/config shell without exposing public login helpers yet.

- [ ] Add a focused `src/stdlib/auth/webauthn.rs` module rather than growing `auth.rs`
- [ ] Add WebAuthn dependency/config glue, likely behind a Cargo feature if dependency weight warrants it
- [ ] Add `enable_auth(...)` WebAuthn options:
  - `rp_id` (domain only, no scheme/port)
  - `origin` / `origins`
  - `rp_name`
  - `user_verification` (`required`, `preferred`, `discouraged`) with safe defaults
  - `resident_key` / discoverable credential policy
  - `attestation` (`none` by default; stricter policy deferred)
  - challenge TTL
- [ ] Add storage records for:
  - WebAuthn credentials / passkeys linked to local identity subject IDs
  - pending registration ceremony state
  - pending authentication ceremony state
- [ ] Ensure pending ceremony state is TTL-bound and one-time consumed; replay must fail
- [ ] Enforce credential-ID global uniqueness in storage, not only in public helper code
- [ ] Add memory + SQLite contract tests by default; decide whether Postgres/Redis passkey storage is in-scope for this phase or explicitly deferred
- [ ] Add health/config diagnostics that reveal misconfiguration without leaking challenge or credential material

### WA-PR 2 — Passkey Registration Helpers

Goal: let an authenticated local user enroll and manage passkeys without changing sign-in behavior yet.

- [ ] Add `begin_passkey_registration(req, identifier_or_user, options?) -> Result<Map, String>` or equivalent shape that starts registration and stores server-side ceremony state
- [ ] Add `finish_passkey_registration(req, credential_response, options?) -> Result<Map, String>` that validates the browser response, consumes registration state, checks credential-ID uniqueness, and stores the credential
- [ ] Return browser-consumable JSON/map creation options suitable for `navigator.credentials.create({ publicKey })`
- [ ] Add `passkeys(identifier_or_user, options?)` / `local_passkeys(...)` safe listing helper with nickname, created/last-used, transports, backup state, and disabled/revoked fields only
- [ ] Add `rename_passkey(...)` and `revoke_passkey(...)` or defer management helpers explicitly if template UX does not need them yet
- [ ] Prevent duplicate registration for the same credential and for credentials already registered to another account
- [ ] Document client-side JSON/base64url handling in `docs/AI_AGENT_GUIDE.md`; do not assume raw browser `ArrayBuffer` values can be passed through ntnt maps unchanged
- [ ] Add tests for happy path, replayed finish, expired state, cross-user credential collision, malformed client response, disabled local account, and secret/challenge non-exposure

### WA-PR 3 — Passkey Authentication + Session Completion

Goal: use passkeys to authenticate through the existing session lifecycle rather than creating a parallel session path.

- [ ] Add `begin_passkey_authentication(req, identifier?, options?) -> Result<Map, String>` for username-known and optionally discoverable/usernameless flows
- [ ] Add `finish_passkey_authentication(response, req, options?) -> Result<Map, String>` that verifies the assertion, consumes authentication state, updates counters/last-used metadata, and returns safe subject/session data
- [ ] Compose successful authentication with `sign_in_session(response, req, session, options?)` or the same internal request-aware completion primitive
- [ ] Support passwordless sign-in for enrolled passkeys without requiring TOTP; passkeys already include authenticator-local user verification when policy requires it
- [ ] Preserve shared session behavior: rotation/migration, `device_name`, `user_agent_hash`, `last_ip_hash`, TTL/max lifetime/cookie policy, and group/claims handoff
- [ ] Return generic failures for unknown user / missing passkey / disabled credential / wrong assertion to avoid enumeration
- [ ] Add tests for username-known login, usernameless/discoverable login if enabled, counter regression/cloned-credential behavior, disabled credential, disabled local account, session rotation, and metadata capture
- [ ] Add template-style examples for page login and JSON/API login endpoints

### WA-PR 4 — Template UX + Policy Hardening

Goal: prove passkeys are usable in a real ntnt app and tighten policy seams before declaring the phase complete.

- [ ] Add template passkey enrollment, login, list, rename, and revoke flows using the stdlib helpers
- [ ] Add progressive enhancement checks for browser support (`PublicKeyCredential`, conditional mediation when used) and graceful fallback to password/TOTP flows
- [ ] Add recovery/account-lockout guidance: passkeys are strong, but apps still need account recovery policy if passwords are disabled
- [ ] Add optional policy hooks for high-assurance apps:
  - require user verification
  - prefer/require platform or roaming authenticators only if WebAuthn APIs expose enough signal
  - attestation allowlists for enterprise/security-key deployments
  - reject or flag backed-up/synced credentials when an app explicitly requires hardware-bound credentials
- [ ] Keep the default consumer posture permissive: no attestation requirement, allow synced passkeys, require safe origin/RP configuration
- [ ] Add end-to-end browser/manual test notes because WebAuthn cannot be fully covered by pure unit tests without ceremony mocks

Non-goals for the first WebAuthn phase:

- Becoming a hosted identity/passkey provider
- Owning account recovery UI or business policy
- Enterprise attestation policy as the default path
- Browser polyfills or non-WebAuthn credential protocols
- Shipping WebAuthn before v0.4.9 final auth/template work is complete

---

## v0.4.9 Final Sprint Done Criteria

The v0.4.9 final sprint is complete when:

1. `std/auth` primitives are solid and composable for OAuth, local email/password, TOTP, password reset, pages, and API endpoints.
2. Password reset tokens are hashed, TTL-bound, one-time, non-enumerating, and not app-reimplemented.
3. TOTP has first-class helper support and no template-owned TOTP model.
4. Authorization context has a clean `group_ids` / claims handoff that apps can use for RBAC without `std/auth` owning RBAC.
5. Local and OAuth sessions share lifecycle, cookie posture, rotation, metadata, and security behavior.
6. Metadata is a deliberate extension seam with namespacing, safe payload rules, and no raw secrets.
7. The template runs on `std/auth` models plus metadata/session-data extensions, with no parallel auth subsystem.

The WebAuthn/passkey phase is complete later when passkey registration, safe listing/revocation, passkey authentication, request-aware session completion, credential replay/counter protection, template UX, and browser-facing examples all work without creating a second user/session system.

---

## Future Refinements

Everything below is explicitly deferred. It may be valuable later, but it is not on the critical path to a coherent 0.4.9 auth finish.

### Full Auth-Owned Local Auth Lifecycle

Only if real apps prove the final-sprint primitives are insufficient:

- Dedicated TOTP enrollment storage instead of reserved local-user metadata
- TOTP failed-attempt throttling and last-used-step replay tracking if apps need std/auth-owned lockout policy
- Dedicated local-account profile/admin APIs
- Higher-level `local_sign_in(...)` wrapper if explicit composition proves too verbose
- `enable_local_auth(...)` convenience preset
- Built-in reset/setup/reference routes and pages
- Full Postgres/Redis local-auth credential/reset/TOTP storage parity

### Advanced WebAuthn / Passkey Policy

After the first WebAuthn/passkey phase proves the primitive shape:

- Attestation trust-store management and enterprise hardware-bound enforcement
- Account-level policy such as “passkey required for admins” or “multiple passkeys required before password disable”
- Conditional UI / usernameless-first login UX polish across browsers
- Browser/device compatibility matrix and automated WebAuthn ceremony fixtures
- Native/mobile app credential handoff patterns if ntnt grows beyond web apps

### Advanced Authorization / RBAC

- First-class group/role tables
- Permission inheritance
- Organization/team membership models
- Policy DSL or ABAC engine
- Admin UI for groups/users/permissions

`std/auth` should not own these until multiple apps prove the shape. For now it provides session data conventions and small helpers.

### Advanced Session Management

- `list_sessions(user_id)` API for admin/arbitrary-user session lookup
- `revoke_session(session_id)` and `revoke_all_sessions(user_id)` APIs
- Password-change/account-disable hooks for session revocation
- Behavioral `remember_me` TTL selection beyond current plumbing
- Device metadata management UI helpers

### Security Events and Suspicious Activity

- `security_events` storage for auth events
- Configurable suspicious-activity actions: `warn`, `challenge`, `revoke`
- `auth_events(user_id, limit?)` for admin dashboards

### Auto-Routes and UI Completion

- Auto-generate every standard auth route when not overridden
- Custom HTML override for built-in login page
- End-to-end route-dispatch tests for route protection through actual ntnt server

### Architecture Discipline

- Contributor/review guardrails for auth module boundaries
- Contract-test ratchet for new auth record families
- `auth.rs` / `auth/storage.rs` pressure relief and complexity budgets
- Periodic architecture audits

---

## What We Won't Do

- **User management UI** — app-owned
- **Public self-service signup flows** — app-owned policy
- **Email/SMS delivery** — app-owned; `std/auth` issues tokens/safe reset material, apps deliver it
- **Profile systems** — app tables or app metadata
- **Full RBAC/ABAC/org modeling in the final sprint** — app-level, with `group_ids` / claims handoff from `std/auth`
- **Billing/subscription gating** — not auth
- **Vendor-hosted identity semantics** — not a SaaS product

---

*Sharpened 2026-04-28. One DD, one roadmap. DD-062 retired. Final sprint keeps the secure essentials first-class — reset, TOTP, API/page protection, and authorization handoff — while pushing product/RBAC/platform bloat to Future Refinements. WebAuthn/passkeys added 2026-04-29 as a post-v0.4.9 phase after PR 3 and template proof.*
