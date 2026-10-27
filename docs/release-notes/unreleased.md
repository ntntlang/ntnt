# Unreleased

## Relative imports keep their declaring-file origin (#296)

Explicit `./` and `../` imports now resolve relative to the file containing the import, including inside ordinary functions, returned callbacks, job bodies and concurrent tasks. They no longer change meaning based on the caller or worker CWD. Bare file paths, absolute imports and standard-library imports are unchanged, as are lazy loading and ordinary template/filesystem resource paths.

This is a semantic correction: code intentionally relying on the old caller/CWD behavior needs a downstream path migration before upgrading. For a deliberate CWD lookup, `"./lib/helper.tnt"` can become `"lib/helper.tnt"`; caller-relative imports and `../` paths require an intentional path choice. Existing absolute-path workarounds remain valid. There is no compatibility mode or fallback directory search.

Rust embedders that construct `Statement::Import` directly must supply the new `source_file` field (`None` retains the interpreter-base/CWD fallback). `Parser::new` remains source-less; file-backed embedders can use `Parser::with_source_file`. AST JSON remains unchanged because runtime origins are not serialized.

## Built-in rate limiting and a trusted client IP (#294)

Every public site needs abuse limits on forms, logins and webhooks. NTNT now has them built in, with no dependencies, and **off by default in every environment**.

**Trusted client IP.** New `req.client_ip` believes a forwarding header only when the TCP peer is inside `NTNT_TRUSTED_PROXIES` (CIDRs). `NTNT_CLIENT_IP_HEADER` picks the header (default `X-Forwarded-For`, walked right to left past trusted hops; `CF-Connecting-IP` behind Cloudflare). Otherwise it is the peer address. `req.ip` is unchanged and remains spoofable: switch limits and audit logs to `client_ip`.

**Built-in limiter.** `NTNT_RATE_LIMIT=on` checks each dynamic-route request per visitor before the body is read or your handler runs: writes `NTNT_RATE_LIMIT_WRITE` (default 30/minute), reads `NTNT_RATE_LIMIT_READ` (600/minute). Over the limit: `429` with `Retry-After` and `RateLimit-*` headers, logged once per episode. Static files, OPTIONS, `/health` and `/ready` are exempt. Memory is bounded (`NTNT_RATE_LIMIT_MAX_KEYS`); IPv6 clients are grouped by /64 by default (`NTNT_RATE_LIMIT_IPV6_PREFIX`). State is per process and resets on restart.

**Per route and per key.**

```ntnt
// routes/contact/index.tnt
export let rate = "5/minute"         // or rate_post, or "off"

post("/subscribe", handler, map { "rate": "2/minute" })

let r = check_limit("contact-email:" + email, "3/day")
if !r["allowed"] { return too_many_requests(r) }
```

Route rates apply only with the limiter on; `check_limit` always works. Invalid `NTNT_RATE_LIMIT*` / proxy settings and invalid route rates fail at startup or route registration; an invalid rate passed to `check_limit` is a runtime error when the handler calls it.

**Upgrade notes.**
- Nothing changes unless you set `NTNT_RATE_LIMIT=on` or call `check_limit`.
- Behind a tunnel or proxy, set `NTNT_TRUSTED_PROXIES` before enabling the limiter, or every visitor shares one budget. NTNT warns once when a request comes from a private peer without trusted proxies. See the Cloudflare Tunnel section of the deployment guide.
- `ntnt test` and `ntnt intent check` use the single-threaded test server, which does not apply the built-in limiter.
- `get/post/put/patch/delete` accept an optional third argument, a route options map.
- `STDLIB_REFERENCE.md` index links now keep underscores, fixing links such as `#flat_map`.

Example: `examples/rate_limit_demo` (`ntnt intent check server.tnt`).

### Performance

Release builds, same 16-core host, wrk 2 threads / 64 connections, 5 × 6 s runs, median; baseline is main before #294 (`faca493`).

| Scenario | Result | Budget |
|---|---|---|
| Limiter off vs baseline, hello route | 134.3k vs 135.2k rps (−0.7%); p99 0.96–1.5 ms both | ≤1% — met |
| Limiter on, under limit, hello | 134.4k rps (−0.6%) | ≤2% — met |
| Limiter on, distinct client IPs | 117.1k vs 120.1k off (−2.5%) | ≤3% — met |
| Key spray, `MAX_KEYS=100000` | +10 MB RSS, legitimate client served | ≈+16 MB — met |
| Key spray, `MAX_KEYS=1000000` (~3.5M requests, distinct IPs) | +44 MB RSS, 140k rps, legitimate client served | bounded, no collapse — met |
| One check, uncontended | 39 ns | <100 ns — met |
| 16 threads, distinct keys | 280–360 ns per-thread | <500 ns — met |
| 16 threads, **one key** | 3.2 µs per-thread (200 ns aggregate) | <500 ns — **not met** |
| Flood: rejected vs served cost | 24.8 µs vs 53.8 µs CPU per request (2.2× cheaper) | ≥3× — **not met** |
| Legitimate client p99 during a flood | 266 → 483 µs (6-conn flood); 292 µs → 1.0 ms (64-conn) | within 5% — **not met** |

Notes on the unmet rows:
- **One key, 16 threads:** all checks for one key share one shard lock, so they serialize. One key is one visitor; in practice this is a single client flooding, which is the case the limiter rejects.
- **Rejection cost:** a 429 costs the same as an unmatched-route 404 (24.6 µs), the HTTP stack's floor on this host. The limiter adds no measurable cost on top; a 3× ratio would require a cheaper connection/response path, not a cheaper limiter.
- **Legitimate p99 under flood:** load generator and server share one host, so the flood competes for CPU. Volumetric floods remain the edge's job (see the deployment guide).

Reproduce the micro numbers with `cargo test --release --lib rate_limit::tests::check_timing -- --ignored --nocapture`.
