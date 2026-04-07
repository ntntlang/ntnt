# DD-059: Hard-Remove Generic Crypto Exports from `std/auth`

## Status: implemented on branch / awaiting PR review

## Problem
`std/auth` should own authentication concerns, not serve as a second home for generic crypto helpers.

That boundary had drifted. Historically, some generic helpers were reachable from `std/auth`, specifically password helpers that already belonged conceptually in `std/crypto`.

On current head, the real remaining overlap was narrower than the original draft assumed:
- `hash_password` was still exported from `std/auth`
- `verify_password` was still exported from `std/auth`
- `uuid` was **already canonical in `std/crypto`** on current head, so there was no active `std/auth` export left to remove there

This DD tightens the boundary all the way instead of carrying a deprecation alias forever.

## Decision
Hard-remove the remaining generic crypto exports from `std/auth`.

After this change:
- `hash_password` must be imported from `std/crypto`
- `verify_password` must be imported from `std/crypto`
- `uuid` remains in `std/crypto`
- auth-owned helpers stay in `std/auth`

## What stays in `std/auth`
This DD does **not** flatten `std/auth` into nothing. The module still owns:
- OAuth helpers
- session helpers
- CSRF helpers
- auth middleware / current-user helpers
- TOTP / MFA helpers such as `totp_secret` and `verify_totp`

TOTP remains intentionally in scope for `std/auth` because it is authentication-specific, not a generic crypto utility.

## Why hard removal is the right call
- The boundary becomes obvious instead of “mostly true except for a few old helpers”
- AI agents and humans get one canonical import path for crypto primitives
- We avoid indefinite warning-only compatibility clutter
- The local audit did not find substantial app usage of the soon-removed `std/auth` imports

## Local Audit
I audited the local NTNT app stack and nearby repos before implementing this change.

Substantial local apps using `std/auth` were using auth-owned helpers only, for example:
- `larri-dashboard`
- `larri-design-ntnt`

I did **not** find substantial local apps importing the removed names from `std/auth`:
- `uuid`
- `hash_password`
- `verify_password`

One nearby repo did show auth-specific usage that should remain untouched:
- `~/repos/larri-site-template/lib/admin_db.tnt` imports `totp_secret` and `verify_totp` from `std/auth`

That reinforces the intended boundary instead of arguing against it.

## Implementation Notes
### Runtime/module surface
- remove `hash_password` from `std/auth`
- remove `verify_password` from `std/auth`
- keep the `std/crypto` implementations as the canonical path
- leave TOTP helpers in `std/auth`

### Docs
- update the DD to reflect the narrower real scope discovered during implementation
- update generated docs/comments so they no longer talk about the `std/auth` aliases as merely deprecated
- make the boundary explicit: generic crypto in `std/crypto`, auth-specific flows in `std/auth`

### Tests
- add regression coverage that `hash_password` / `verify_password` still work from `std/crypto`
- add regression coverage that importing `hash_password` from `std/auth` now fails with a missing-export error

## Risks
| Risk | Why it matters | Mitigation |
|------|----------------|------------|
| External apps may still import removed names from `std/auth` | This is a breaking change | Intentional. Break loudly and keep the boundary clean. |
| Scope creep into TOTP | Could accidentally move auth-specific helpers out of `std/auth` | Keep TOTP explicitly in `std/auth` and document that it is out of scope for this DD. |
| Draft drift from reality | Original DD assumed `uuid` was still exported from `std/auth` | Update the DD to reflect the actual audited current-head state. |

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
- [x] Confirm `uuid` is already canonical in `std/crypto` and does not need a new removal patch in `std/auth`
- [x] Leave auth-owned helpers, including TOTP, untouched in `std/auth`

### Phase 3 — Tests and docs
- [x] Add a regression test proving password helpers still work from `std/crypto`
- [x] Add a regression test proving `hash_password` is no longer exported from `std/auth`
- [x] Update crypto docs/comments so they describe the auth alias as removed, not merely deprecated
- [x] Regenerate docs with `ntnt docs --generate`
- [x] Re-run `ntnt learn`-driven generated docs via the standard docs generation path

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
Implemented and validated on branch `feat/dd-059-auth-crypto-boundary`.

Validation loop completed:
- `cargo fmt`
- `cargo build --profile dev-release`
- `cargo test --lib`
- `cargo test --test language_features_tests --test type_checker_tests --test cli_tests`
- `cargo build --release --locked`
- `./target/release/ntnt docs --generate`
- `cargo fmt -- --check`

Behavior validated:
- `hash_password` works from `std/crypto`
- `verify_password` works from `std/crypto`
- importing `hash_password` from `std/auth` now fails as a missing export
- TOTP helpers remain in `std/auth`

## Bottom-Line Recommendation
Ship it.

This is the right kind of breaking change:
- small
- conceptually clean
- locally low-risk
- boundary-improving

The only real caveat is to keep the scope disciplined: this DD removes the remaining generic crypto aliases from `std/auth`, but it does **not** argue that auth-specific helpers such as TOTP should move.
