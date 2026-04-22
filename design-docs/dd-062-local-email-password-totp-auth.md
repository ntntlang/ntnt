# DD-062: First-Class Local Email/Password/TOTP Auth in `std/auth`

## Problem

`std/auth` already owns sessions, staged auth challenges, cookies, OAuth/OIDC, CSRF, and TOTP primitives.

But for a very common real app shape — a local admin area using **email + password + TOTP** — developers still have to build and maintain their own mini auth system around those primitives:

- local user table
- password-hash storage
- password reset tokens
- TOTP enrollment state
- local login route handlers
- first-login / forced-password-change flows
- admin-role / claims glue

That is exactly the wrong level of abstraction.

The template app should not need a custom `lib/admin_db.tnt` auth subsystem just to get a normal local admin login.

## Goal

Make **local email/password/TOTP auth** a first-class `std/auth` path so apps can configure it instead of rebuilding it.

Target outcome:

- no custom account tables in the template
- no template-owned password login state machine
- no template-owned password-reset token model
- no template-owned staged-setup persistence model
- template becomes mostly configuration + views

## Desired Developer Experience

Something in this family should be possible:

```ntnt
import { enable_auth, local_auth } from "std/auth"

enable_auth([
  local_auth(map {
    "email_password": true,
    "totp": true,
    "password_reset": true,
    "bootstrap_email": get_env("ADMIN_BOOTSTRAP_EMAIL"),
    "bootstrap_password": get_env("ADMIN_BOOTSTRAP_PASSWORD"),
    "claims": map { "role": "admin" }
  })
], "admin", map {
  "login_page": false,
  "success_url": "/admin",
  "failure_url": "/admin/login"
})
```

Or, if that is too provider-shaped, an equivalent `enable_local_auth(...)` surface is acceptable.

The specific API name matters less than the capability.

## Scope

### Phase 1 — Local identity store
- [ ] Add built-in local user storage owned by `std/auth`
- [ ] Canonical identifier is **email**
- [ ] Store password hashes in auth-owned tables
- [ ] Support bootstrap account creation from config/env
- [ ] Support password verification through `std/auth`

### Phase 2 — Local login + staged auth flow
- [ ] Add built-in email/password sign-in entrypoint
- [ ] Reuse current staged-auth challenge primitives for TOTP / setup / forced-password-change
- [ ] Support first-login setup flow
- [ ] Support forced password change after bootstrap or reset
- [ ] Support completion into real auth sessions with configured claims/data

### Phase 3 — Password reset
- [ ] Add auth-owned password-reset token storage
- [ ] Add helpers/routes for reset-token issue + consume
- [ ] Support reset flows that clear TOTP enrollment when configured
- [ ] Support forcing re-enrollment after reset

### Phase 4 — Turnkey template fit
- [ ] Make the Larri site template use the built-in local auth system instead of custom tables
- [ ] Delete template-owned local auth persistence/state machine code
- [ ] Keep template customization at the UI/copy layer only

## Why this belongs in `std/auth`

We already have most of the hard parts:

- signed session cookies
- session stores
- staged auth challenges
- TOTP helpers
- logout/sign-in helpers
- OAuth/OIDC flows
- auth route configuration

What is missing is not raw capability. What is missing is **cohesion around local credentials**.

This is why local apps still feel like they have to build a second auth system beside `std/auth`.

## Non-Goals

- replacing OAuth/OIDC work
- building a full end-user identity platform in one shot
- modeling arbitrary profile fields up front
- solving every RBAC problem before shipping local auth

This should start with the smallest excellent local-auth core:

**email + password + TOTP + reset + staged setup**

## Recommendation

Treat this as the next practical `std/auth` simplification step.

If `std/auth` can own OAuth well but still cannot cleanly own a normal local admin login, the library remains incomplete in a way users feel immediately.
