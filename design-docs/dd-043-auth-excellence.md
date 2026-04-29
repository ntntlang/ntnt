# DD-043: World-Class Auth — Making `std/auth` the Best stdlib Auth Ever

**Status:** In Progress — Final Sprint
**Author:** Larri
**Date:** 2026-03-20 (slashed and sharpened 2026-04-28)
**Branch:** `main` through v0.4.9; remaining work is 2-3 focused PRs

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

## Done Criteria

DD-043 is complete when:

1. `std/auth` primitives are solid and composable for OAuth, local email/password, TOTP, password reset, pages, and API endpoints.
2. Password reset tokens are hashed, TTL-bound, one-time, non-enumerating, and not app-reimplemented.
3. TOTP has first-class helper support and no template-owned TOTP model.
4. Authorization context has a clean `group_ids` / claims handoff that apps can use for RBAC without `std/auth` owning RBAC.
5. Local and OAuth sessions share lifecycle, cookie posture, rotation, metadata, and security behavior.
6. Metadata is a deliberate extension seam with namespacing, safe payload rules, and no raw secrets.
7. The template runs on `std/auth` models plus metadata/session-data extensions, with no parallel auth subsystem.

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

*Sharpened 2026-04-28. One DD, one roadmap. DD-062 retired. Final sprint keeps the secure essentials first-class — reset, TOTP, API/page protection, and authorization handoff — while pushing product/RBAC/platform bloat to Future Refinements.*
