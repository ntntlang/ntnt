# DD-043 Phase 7 Plan — Auto-Routes and Convenience UI

**Date:** 2026-04-21  
**Branch:** `feat/auth-phase-7`

## Chosen Phase 7 slice

Implement the coherent first Phase 7 slice that makes the easy path extremely easy without overcommitting to a rigid UI system:

- [ ] Add configurable auth route prefix support (default `/auth`)
- [ ] Make built-in auth handlers and diagnostics honor the configured prefix
- [ ] Add startup logging that shows the registered built-in auth routes for the active prefix
- [ ] Add collision detection helpers so apps get a clear warning/error path when built-in auth routes would overlap app-defined routes
- [ ] Upgrade the built-in login page into a configurable convenience UI with title/logo/copy and auto-generated provider buttons
- [ ] Add opt-out / override config so apps can keep using only the protocol helpers without the built-in page chrome
- [ ] Add tests covering default prefix, custom prefix, login page rendering, route metadata/health output, and collision detection
- [ ] Regenerate docs and verify generated auth docs match the new configuration surface

## Why this slice

This groups the Phase 7 items that share one internal seam: built-in route registration and built-in auth UI metadata. It delivers the whole “zero/low wiring auth” story in one PR while deferring more speculative features.

## Planned implementation

### 1. Config surface
- Extend `AuthConfig` with Phase 7 options:
  - `route_prefix: String` (default `/auth`)
  - `route_collision_mode` or equivalent simple policy for built-in route conflicts
  - `login_page_enabled: bool`
  - `login_page_title: String`
  - `login_page_logo_url: Option<String>`
  - `login_page_heading: String`
  - `login_page_copy: String`
- Parse these via `enable_auth(...)` with strict validation and typo suggestions.
- Normalize route prefixes (`auth`, `/auth`, `/auth/` → `/auth`) and reject empty/invalid values.

### 2. Built-in route metadata + path helpers
- Introduce small helpers to build auth paths from config rather than hardcoding `/auth/...` in route handlers.
- Update start/callback/logout/health/index redirects and diagnostics to use the configured prefix.
- Update health output route metadata so it reflects the active prefix.

### 3. Convenience UI
- Replace the minimal built-in index page with a configurable default login page.
- Preserve safety:
  - HTML-escape all copy values
  - URL-encode provider path segments
  - only emit logo markup when a non-empty logo URL is configured
- Keep provider button generation automatic from configured providers.
- If `login_page_enabled` is false and multiple providers exist, redirect behavior should remain predictable and documented.

### 4. Collision detection
- Add a helper that can evaluate whether known built-in auth routes under the chosen prefix overlap explicit app routes.
- Since `std/auth` does not own the whole router, start with deterministic built-in route manifest + exported collision checker and startup warnings for obvious self-conflicts / reserved-prefix usage.
- Prefer behavior that is useful now without pretending we can introspect all app routes magically.

### 5. Docs/tests
- Unit tests for route prefix normalization and rendered paths.
- Unit tests for login page config rendering and escaping.
- Unit tests for route metadata in `/auth/health` equivalent diagnostics with custom prefix.
- Unit tests for collision detection semantics.
- Generated docs update for new `enable_auth` options and any new helper(s).

## Risks / open questions

1. **Collision detection scope**: full app-route introspection may not exist yet. The plan should avoid a fake promise and instead expose a truthful built-in-route manifest/checker.
2. **Backward compatibility**: existing hardcoded `/auth` references must keep working by default.
3. **UI scope creep**: the built-in page should stay a convenience default, not become a full theming system.

## Self-review

- Matches the Phase 7 checkboxes directly, except that collision detection is implemented honestly through built-in manifest/checker semantics rather than impossible router omniscience.
- Reuses the existing built-in auth index page seam in `routes.rs`, which keeps the diff localized.
- Fits existing `enable_auth` option parsing/validation patterns.
- Keeps custom apps free to ignore the convenience UI.
