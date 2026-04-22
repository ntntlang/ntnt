# DD-043 Phase 6 Implementation Plan

## Scope
Implement the full Phase 6 slice from DD-043:
1. Refresh token rotation handling when providers issue a new refresh token
2. Invalidate old refresh token references where feasible
3. Log refresh token rotation events
4. Add `/auth/health` endpoint (dev-only by default)
5. Health output shows config state without leaking secrets
6. Diagnose common issues like redirect mismatch, missing env, wrong store
7. Document provider differences clearly

## Likely code areas
- `src/stdlib/auth/oauth.rs`
  - tighten refresh response parsing to preserve whether provider returned a new refresh token vs reused prior token
- `src/stdlib/auth/sessions.rs`
  - centralize auto-refresh handling and emit rotation/security logs
- `src/stdlib/auth/storage.rs`
  - add a helper to clear/replace stale session rows by refresh-token rotation semantics where backend support allows it
- `src/stdlib/auth/routes.rs`
  - implement `/auth/health`
  - include auth diagnostics payload + dev/prod gating
- `src/stdlib/auth/guards.rs` or request helpers
  - exempt `/auth/health` consistently from auth enforcement if needed
- `src/stdlib/auth.rs`
  - wire built-in route and any public-facing helpers/docs
  - add tests near existing auth tests for refresh rotation + health route behavior
- `docs/STDLIB_REFERENCE.md` / generated docs
  - regenerated if docstrings or exported behavior text changes
- `design-docs/dd-043-auth-excellence.md`
  - mark Phase 6 items complete once shipped

## Design approach
### Refresh-token rotation semantics
- Distinguish between:
  - provider returned a new refresh token (real rotation)
  - provider omitted refresh token (keep existing one)
- When a new refresh token is returned, treat it as rotation and overwrite the stored token atomically with the new access token + expiry.
- "Invalidate old refresh token references where feasible" likely means:
  - ensure current session storage no longer retains the prior token after update
  - log rotation occurrence for observability
  - document that upstream provider invalidation is provider-defined and not always app-enforceable
- Avoid pretending we can revoke provider-side old tokens unless the provider exposes a revocation endpoint and we already support it.

### Observability / security logging
- Emit structured-ish stderr logs with stable `[auth]` prefixes for:
  - refresh success without rotation
  - refresh success with rotation
  - refresh failure
- Keep logs secret-safe: never print token values.

### `/auth/health`
- Built-in route under `/auth/health`
- Dev-only by default: in production, return 404/disabled unless explicit config opt-in exists; if no opt-in exists today, implement strict dev-only behavior for this phase.
- Return JSON or HTML? Prefer JSON map-like HTTP response since this is diagnostics.
- Include:
  - auth configured or not
  - provider names
  - route URLs
  - cookie posture (`secure`, same-site, cookie name)
  - session store backend kind
  - token storage enabled flag
  - protected paths count/list (safe)
  - warnings/diagnostics array for likely misconfigurations:
    - default/dev session secret in production
    - missing provider env values / blank client id or secret
    - likely redirect-uri mismatch hints based on `SITE_URL` / request host / provider callback shape
    - memory store in production warning
- Exclude secrets, tokens, raw URLs with embedded credentials.

## Tests
- Unit tests for refresh response parsing:
  - new refresh token returned => rotation detected
  - no refresh token returned => keep old token, no rotation event
- Session refresh tests across memory/sqlite existing helpers:
  - expired session auto-refresh updates access token
  - rotated refresh token replaces stored token
- Health route tests:
  - enabled in dev
  - hidden/disabled in prod
  - output contains safe config summary and warnings
  - output does not contain client_secret / session_secret
- If practical, tests for provider-diagnostics text on missing config

## Risks / review focus
- Do not break existing refresh behavior for providers that never resend refresh tokens.
- Keep storage updates aligned across backends; existing `update_session_record_tokens` may already be sufficient if it overwrites refresh token when provided.
- Avoid leaking secrets in health output or logs.
- Match existing built-in route patterns instead of inventing a new response shape.

## Self-review notes
- Existing `refresh_access_token()` currently always returns some refresh token by falling back to the old one. That hides whether rotation happened; fix by preserving provider response distinction.
- Existing `update_session_record_tokens()` already uses `COALESCE` semantics, which is good for providers that omit refresh tokens.
- Need to inspect route registration before implementing `/auth/health` to ensure built-in dispatch exists.
- Need to inspect current tests around auth routes and refresh flows to extend, not duplicate.
