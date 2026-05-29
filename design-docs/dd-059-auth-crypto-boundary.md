# DD-059: Hard-Remove Generic Crypto Exports from `std/auth`

## Status: implemented in the v0.4.9 baseline

## Problem
`std/auth` should own authentication concerns, not generic crypto utilities.

On current head, the real remaining overlap was narrower than the first draft assumed:
- `hash_password` was still exported from `std/auth`
- `verify_password` was still exported from `std/auth`
- `uuid` was already canonical in `std/crypto`

So the actual cleanup is to remove the remaining password-helper aliases from `std/auth`, not to pretend all three paths still exist there.

## Decision
Hard-remove the remaining generic crypto aliases from `std/auth`.

After this change:
- `hash_password` must be imported from `std/crypto`
- `verify_password` must be imported from `std/crypto`
- `uuid` remains in `std/crypto`
- auth-owned helpers remain in `std/auth`, including local identity/credential lifecycle, password reset, TOTP, sessions, OAuth, CSRF, staged challenges, route/API protection, and current-user session management

## Explicitly In Scope vs Out of Scope
**In scope**
- remove `hash_password` from `std/auth`
- remove `verify_password` from `std/auth`
- update docs/tests to reflect the hard removal

**Out of scope**
- moving TOTP helpers out of `std/auth`
- changing OAuth/session/CSRF/current-user helpers
- inventing a deprecation bridge or compatibility shim

TOTP stays in `std/auth` because it is authentication-specific, not generic crypto.

## Local Audit
I checked the local NTNT app stack before shipping this change.

Substantial local apps do use `std/auth`, but only for auth-owned helpers. I did **not** find substantial local apps importing these soon-removed names from `std/auth`:
- `uuid`
- `hash_password`
- `verify_password`

One nearby repo, `larri-site-template`, still imports `totp_secret` / `verify_totp` from `std/auth`, which is consistent with the intended boundary and stays untouched.

## Implementation Checklist
### Phase 1 — Re-audit current-head reality
- [x] Re-check whether `uuid` is still exported from `std/auth`
- [x] Re-check whether `hash_password` is still exported from `std/auth`
- [x] Re-check whether `verify_password` is still exported from `std/auth`
- [x] Audit local substantial apps for `std/auth` imports of the soon-removed names
- [x] Record that TOTP helpers remain intentionally in `std/auth`
- [x] Update the DD to reflect the narrower real scope on current head

### Phase 2 — Runtime/module boundary cleanup
- [x] Remove `hash_password` export from `std/auth`
- [x] Remove `verify_password` export from `std/auth`
- [x] Confirm `std/crypto` remains the canonical import path for both helpers
- [x] Confirm `uuid` is already canonical in `std/crypto`
- [x] Leave TOTP and other auth-owned helpers untouched in `std/auth`

### Phase 3 — Tests and docs
- [x] Add a regression test proving password helpers still work from `std/crypto`
- [x] Add a regression test proving password helpers are no longer exported from `std/auth`
- [x] Update docs/comments so they reflect hard removal, not deprecation rhetoric
- [x] Regenerate docs with `ntnt docs --generate`

### Phase 4 — Validation and ship readiness
- [x] Run `cargo fmt`
- [x] Run `cargo build --profile dev-release`
- [x] Run `cargo test --lib`
- [x] Run `cargo test --test language_features_tests --test type_checker_tests --test cli_tests`
- [x] Run `cargo build --release --locked`
- [x] Run `./target/release/ntnt docs --generate`
- [x] Run `cargo fmt -- --check`
- [x] Open PR with the DD, implementation, and audit context together

## Validation
Implemented in the v0.4.9 auth baseline; original work happened on `feat/dd-059-auth-crypto-boundary`. Current 0.4.9 polish keeps the boundary intact while adding auth-owned local password/reset/TOTP features in `std/auth`.

Validated behavior:
- `hash_password` works from `std/crypto`
- `verify_password` works from `std/crypto`
- importing password helpers from `std/auth` now fails as a missing export
- TOTP helpers remain in `std/auth`

## Bottom Line
Ship it.

This is a small breaking change, but it is the right one: it removes the remaining generic crypto aliases from `std/auth` without dragging auth-specific helpers into unnecessary churn. The added local-auth primitives do not reopen the generic-crypto boundary; they are auth lifecycle APIs, not generic hash/UUID utilities.
