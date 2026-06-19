# DD-064: `std/net` Enhancements for v0.4.12

**Status:** Draft / future release plan
**Author:** Larri + Josh
**Created:** 2026-06-18
**Related:** [DD-046: `std/net`](dd-046-std-net.md), [DD-047: `std/netmon`](dd-047-std-netmon.md), closed PR [#124](https://github.com/ntntlang/ntnt/pull/124)
**Target baseline:** after v0.4.11 `std/net` release-readiness work lands
**Target release:** v0.4.12 candidate slice

---

## Summary

DD-046 made `std/net` release-ready as a safe primitive layer: IP/CIDR helpers, ICMP ping, TCP connect/reachability, DNS, bounded port scan, TLS certificate inspection, capabilities, and one-shot traceroute.

This DD captures the next small enhancement wave for v0.4.12. The goal is to make `std/net` feel less thin without turning it into a monitoring framework or packet toolkit.

The recommended v0.4.12 direction:

1. Add local network context helpers that require no outbound traffic.
2. Improve DNS and address-selection ergonomics.
3. Add small, bounded probe conveniences that compose with existing primitives.
4. Tighten result consistency and testability where v0.4.11 exposed rough edges.
5. Keep heavy monitoring and repeated/parallel tracing out of `std/net`.

This is a future-facing DD, not a mandate to ship all candidates in one PR. The release plan below intentionally slices the work so we can stop after the high-confidence pieces.

---

## Product Boundary

`std/net` should remain a **primitive library**:

- one-shot checks
- bounded timeouts
- explicit targets
- safe defaults
- structured results
- no surprise scans
- no persistent monitors
- no device/vendor model

`std/netmon` owns higher-level monitoring: SNMP, interface counters, inventory, alert-state transitions, topology hints, sweeps, repeated checks, and storage-oriented telemetry.

The dividing line:

- `std/net`: “Can I safely ask this specific network question once?”
- `std/netmon`: “Can I operate a monitoring system over time?”

This DD should not backdoor `std/netmon` into the default stdlib under a different hat and a tiny mustache.

---

## Explicit Non-Goals

### Do not add parallel traceroute before v0.4.12

Closed PR [#124](https://github.com/ntntlang/ntnt/pull/124) should stay closed unless a future design changes materially.

Reasons:

- It required a large internal abstraction change.
- Raw-socket behavior is hard to CI-test deterministically.
- The PR already showed repeated correctness churn around send failure classification.
- Speed is nice, but not release-essential.
- It makes `std/net` feel like a traceroute engine instead of a primitive library.

If users need repeated or faster traces, use `std/jobs` / `std/concurrent` around the existing one-shot `traceroute()` primitive, or move the higher-level abstraction into `std/netmon`.

### Do not add these to default `std/net` in v0.4.12

- SNMP polling/walking — belongs in DD-047 `std/netmon`.
- ARP/NDP scans, subnet discovery, passive packet capture.
- Bandwidth tests / iperf-style traffic generation.
- SSH remote command execution.
- BGP, LLDP/CDP, device inventory, topology modeling.
- MTR-style continuous traceroute statistics.
- Parallel hop sweeps or per-hop enrichment.
- Runtime package/plugin systems for network protocols.
- Any default path that requires root, `CAP_NET_RAW`, Docker `cap_add`, or sysctl tuning.

---

## Design Principles

1. **Small primitives beat clever frameworks.** Add functions a developer can understand from the signature.
2. **Safe by default remains non-negotiable.** Private/internal targets still require `NTNT_NET_ALLOW_PRIVATE=1` plus per-call `allow_private: true`.
3. **No unbounded fan-out.** Any helper that can touch more than one target, port, or record type needs strict caps and deterministic ordering.
4. **Expected network failures are data.** Timeouts, refused connections, NXDOMAIN, no-answer DNS, TLS validation failure, and unreachable hosts should remain structured probe results where practical.
5. **Capability honesty.** If a feature needs OS support, expose it through `net_capabilities()` and return clear errors when unavailable.
6. **Testable slices only.** Prefer helpers that can be unit-tested with mocks or loopback fixtures. Raw-socket-heavy work needs opt-in tests and should be avoided unless the value is obvious.
7. **Add options before duplicating functions.** If the existing primitive is good and only lacks address-family or resolver control, extend options rather than creating a near-clone.

---

## Current Baseline From DD-046

Assume v0.4.11 ships these `std/net` functions:

- `ip_parse(ip_or_cidr)`
- `subnet_contains(cidr, ip)`
- `subnet_overlaps(a, b)`
- `subnet_split(cidr, new_prefix, opts?)`
- `subnet_supernet(cidr, new_prefix?)`
- `subnet_summarize(cidrs)`
- `ip_range_to_cidrs(start_ip, end_ip)`
- `net_capabilities()`
- `ping(host, opts?)`
- `traceroute(host, opts?)`
- `tcp_connect(host, port, opts?)`
- `reachable(host, opts?)`
- `port_scan(host, ports, opts?)`
- `dns_lookup(name, record_type?, opts?)`
- `dns_reverse(ip, opts?)`
- `tls_info(host, opts?)`

The v0.4.12 candidates below should build on these rather than replacing them.

---

## Candidate Enhancements

### 1. Local network context helpers

These are high-value, low-risk, and require no outbound packets. They help diagnostics apps explain the environment they are running in.

#### `local_interfaces(opts?) -> Result<Array<Map>, String>`

Return local network interfaces and addresses.

Example:

```ntnt
import { local_interfaces } from "std/net"

let interfaces = local_interfaces()
// Ok([
//   map {
//     "name": "eth0",
//     "up": true,
//     "loopback": false,
//     "addresses": [
//       map { "ip": "10.0.50.25", "family": "ipv4", "prefix_len": 24 }
//     ]
//   }
// ])
```

Recommended fields:

- `name`
- `index` when available
- `up`
- `loopback`
- `multicast`
- `mac` when available
- `mtu` when available
- `addresses`: array of `{ ip, family, prefix_len? }`

Options:

- `include_loopback`: default `false`
- `family`: `"all" | "ipv4" | "ipv6"`; default `"all"`, because this helper lists local addresses rather than choosing one address to probe

Implementation notes:

- Prefer a cross-platform crate such as `if-addrs` or `network-interface` after checking maintenance and Windows/macOS behavior.
- Do not expose private addresses as a security bypass issue: this is local process introspection, not outbound probing. Still document that public web apps should not dump this result to untrusted users.

#### `default_route(opts?) -> Result<Map, String>`

Return the default route interface/gateway when available.

Example:

```ntnt
let route = default_route()
// Ok(map { "interface": "eth0", "gateway": "10.0.50.1", "family": "ipv4" })
```

This may be harder cross-platform than `local_interfaces()`. If the implementation gets ugly, defer it and ship `local_interfaces()` first.

#### `local_addr_for(target, opts?) -> Result<Map, String>`

Return the local source address the OS would use to reach a target, without sending application data.

Example:

```ntnt
let src = local_addr_for("8.8.8.8")
// Ok(map { "local_ip": "10.0.50.25", "family": "ipv4" })
```

Implementation approach:

- Use UDP socket connect trick to determine the selected local address.
- Apply `std/net` target policy to the target because it resolves/targets a remote address.
- No packet payload should be sent.

Priority: medium. Useful, but less essential than `local_interfaces()`.

---

### 2. Address-family and resolver-order controls

The v0.4.11 primitives mostly use resolver order. v0.4.12 should let developers choose family behavior without manually pre-resolving and looping.

Add common option support to functions that resolve/connect:

```ntnt
map {
    "family": "auto",       // auto | ipv4 | ipv6
    "address_order": "resolver", // resolver | ipv4_first | ipv6_first
    "max_addresses": 8
}
```

Applicable functions:

- `ping()`
- `tcp_connect()`
- `reachable()` TCP fallback path
- `port_scan()`
- `tls_info()`
- `traceroute()` where method supports address selection

Rules:

- `family: "ipv4"` filters to A/IPv4 addresses.
- `family: "ipv6"` filters to AAAA/IPv6 addresses.
- `family: "auto"` preserves current behavior unless `address_order` is set.
- Empty address set after filtering returns `Err("No resolved addresses for requested family")` or equivalent clear message.
- Policy is still enforced after filtering and before probing.

Do **not** add full Happy Eyeballs parallel racing in v0.4.12 unless there is a small, testable implementation. A simple family/order option is enough.

---

### 3. DNS ergonomics and resolver controls

`dns_lookup()` and `dns_reverse()` are useful but can grow a little without becoming a DNS toolkit.

#### Extend DNS options

Support optional resolver controls:

```ntnt
dns_lookup("example.com", "A", map {
    "timeout_ms": 1000,
    "nameservers": ["1.1.1.1", "8.8.8.8"],
    "strategy": "system" // system | explicit
})
```

Rules:

- Default remains system resolver.
- Explicit nameservers are bounded: max 3.
- `strategy: "system"` ignores `nameservers` unless a future implementation deliberately supports fallback/override semantics.
- `strategy: "explicit"` requires a non-empty `nameservers` array; absent or empty `nameservers` must return a clear `Err(String)`, never silently fall back to the system resolver.
- Nameserver targets must pass the same target policy as outbound probes.
- Private nameservers require `NTNT_NET_ALLOW_PRIVATE=1` plus `allow_private: true`.
- DNS transport errors remain `Err(String)`; clean no-answer stays `Ok([])`.

#### `dns_lookup_many(name, record_types, opts?) -> Result<Map, String>`

Batch common records with shared resolver/timeout options.

Example:

```ntnt
let records = dns_lookup_many("example.com", ["A", "AAAA", "MX", "TXT"], map {
    "timeout_ms": 1000,
    "strategy": "explicit",
    "nameservers": ["1.1.1.1"]
})
// Ok(map { "A": [...], "AAAA": [...], "MX": [...], "TXT": [...] })
```

Bounds:

- `opts` inherits the same resolver controls as `dns_lookup()`, including `timeout_ms`, `strategy`, `nameservers`, and `allow_private` for private nameserver targets.
- Max record types: 8.
- Reject duplicate and unsupported record types.
- Preserve per-type no-answer as an empty array.
- If one record type has an operational resolver failure, the whole call returns `Err(String)`. Mixed partial failure maps are deliberately out of scope for this primitive; apps needing partial behavior can call `dns_lookup()` individually.

Priority: high if current DNS usage feels repetitive; otherwise defer.

---

### 4. Probe result consistency cleanup

Before adding shiny things, make result shapes boringly consistent. Boring is good. Boring means nobody opens a 2/5 confidence review at midnight.

Potential v0.4.12 cleanup:

- Normalize timeout reason strings across `tcp_connect()`, `reachable()` TCP attempts, and `port_scan()`.
- Consider adding stable machine-readable reason codes while preserving human-readable `reason`:

```ntnt
map {
    "connected": false,
    "reason": "timeout",
    "reason_code": "timeout"
}
```

Candidate `reason_code` values:

- `timeout`
- `connection_refused`
- `dns_error`
- `policy_denied`
- `permission_denied`
- `unreachable`
- `invalid_input`
- `backend_unavailable`

Compatibility rule:

- Do not remove or rename existing `reason` fields in v0.4.12.
- If `reason_code` is added, add it everywhere the corresponding result map is returned.
- Tests must assert both human string and code for representative outcomes.

Priority: high if v0.4.11 review leaves any result-string inconsistency unresolved.

---

### 5. TLS audit conveniences

`tls_info()` already returns certificate metadata. v0.4.12 can add convenience checks without inventing a TLS scanner.

#### Option A: extend `tls_info()` with policy options

```ntnt
let cert = tls_info("example.com", map {
    "timeout_ms": 1000,
    "min_days_left": 14,
    "require_valid": true
})
```

Additional result fields:

- `expires_soon`: bool, present whenever TLS policy options are requested; `true` when `min_days_left` is provided and `days_left < min_days_left`, otherwise `false`
- `policy_ok`: bool
- `policy_errors`: array of strings

#### Option B: add `tls_check(host, opts?) -> Result<Map, String>`

```ntnt
let check = tls_check("example.com", map {
    "min_days_left": 14,
    "require_valid": true
})
// Ok(map { "ok": true, "valid": true, "days_left": 90, "errors": [] })
```

Recommendation: prefer extending `tls_info()` unless the result shape becomes noisy. `tls_check()` is friendlier for monitoring apps but starts to overlap with `std/netmon` check orchestration.

Priority: medium.

---

### 6. TCP banner read, maybe

A bounded banner read is useful for diagnostics, but it is also where “simple TCP primitive” starts flirting with protocol clients. Treat as optional, not automatic v0.4.12 scope.

#### `tcp_banner(host, port, opts?) -> Result<Map, String>`

Connect to a TCP service and read a bounded initial banner.

Example:

```ntnt
let banner = tcp_banner("smtp.example.com", 25, map {
    "timeout_ms": 1000,
    "max_bytes": 512
})
// Ok(map { "connected": true, "banner": "220 smtp.example.com ESMTP", "bytes_read": 27 })
```

Rules:

- Default `max_bytes`: 512.
- Hard cap: 4096.
- No send payload in the initial `tcp_banner()` implementation. Just read after connect.
- `timeout_ms` is a total deadline covering DNS resolution, TCP connect, and the banner read. A service that accepts the connection but sends no banner must return a bounded timeout result instead of blocking indefinitely.
- Return both text and bytes only if there is an established stdlib convention for byte arrays; otherwise use lossy string plus `bytes_read` and document it.
- Apply the same private-target policy as `tcp_connect()`.

Do not add generic `tcp_exchange(send, read)` unless a real app needs it. That becomes a mini socket API, and then the walls start whispering about protocols.

Priority: optional / separate PR only.

---

### 7. Capability reporting improvements

`net_capabilities()` should stay traffic-free and should help apps decide which examples/features are usable in the current runtime.

Potential added fields:

```ntnt
net_capabilities()
// map {
//   "icmp": map { "supported": true, "requires_raw_socket": false },
//   "traceroute": map { "icmp": true, "udp": true, "tcp": true },
//   "interfaces": map { "supported": true },
//   "dns_custom_resolver": map { "supported": true },
//   "platform": "linux"
// }
```

Rules:

- Do not perform outbound probes.
- Do not require elevated privileges just to ask capabilities.
- If a future helper is platform-specific, capabilities must expose that clearly.

Priority: low-to-medium; bundle with whichever PR adds a feature that benefits from capability reporting.

---

## Recommended v0.4.12 PR Plan

This plan is intentionally ordered from safest to riskiest. Stop after any PR if the release feels large enough.

### PR 1: Result consistency and release cleanup

**Goal:** make existing `std/net` result shapes and docs more stable before adding features.

Scope:

- Normalize timeout/reason strings if not already fixed in v0.4.11 follow-ups.
- Add `reason_code` only if the implementation is small and covers all relevant result maps.
- Add/extend regression tests for:
  - TCP timeout
  - refused connection
  - no resolved addresses
  - policy denial
  - port scan closed/timeout result
- Update `// @ntnt` docs and regenerate generated docs if visible result fields change.

Files likely touched:

- `src/stdlib/net/mod.rs`
- `src/stdlib/net/policy.rs` if policy reason codes are centralized
- `tests/std_net_tests.rs`
- `src/typechecker.rs` only if signatures change, which they should not
- `docs/AI_AGENT_GUIDE.md`
- generated docs via `./target/dev-release/ntnt docs --generate`

Verification:

```bash
cargo fmt
cargo build --profile dev-release
./target/dev-release/ntnt docs --generate
cargo test --lib stdlib::net -- --nocapture
cargo test --test std_net_tests -- --nocapture
cargo test
```

### PR 2: Local interface introspection

**Goal:** add low-risk environment diagnostics.

Scope:

- Add `local_interfaces(opts?)`.
- Optionally add `local_addr_for(target, opts?)` if the UDP socket trick is clean and testable.
- Defer `default_route()` unless the cross-platform implementation is simple.
- Add `net_capabilities().interfaces` if useful.

Files likely touched:

- `Cargo.toml` if adding an interface crate
- `src/stdlib/net/mod.rs`
- maybe `src/stdlib/net/local.rs` for focused implementation
- `src/typechecker.rs`
- `tests/std_net_tests.rs`
- `docs/AI_AGENT_GUIDE.md`
- generated docs

Tests:

- Unit test result normalization from mocked interface data if possible.
- Integration smoke test that `local_interfaces()` returns `Ok(Array)` without requiring a specific interface name.
- Do not assert environment-specific IPs in CI.

### PR 3: DNS resolver controls and batch lookup

**Goal:** improve DNS ergonomics without adding a full DNS toolkit.

Scope:

- Extend DNS options with bounded explicit `nameservers` if the resolver crate supports it cleanly.
- Add `dns_lookup_many(name, record_types, opts?)` only if it shares existing DNS rendering code without duplication.
- Keep default system resolver behavior unchanged.

Files likely touched:

- `src/stdlib/net/mod.rs`
- maybe `src/stdlib/net/dns.rs` if extraction makes the code cleaner
- `src/typechecker.rs`
- `tests/std_net_tests.rs`
- docs/generated docs

Tests:

- Unsupported/duplicate record types rejected.
- Empty record type array rejected.
- Too many record types rejected.
- Explicit private nameserver requires process + per-call opt-in.
- No-answer behavior remains `Ok([])` for single lookup and empty arrays for batch lookup.

### PR 4: Address-family controls

**Goal:** let app authors choose IPv4/IPv6 behavior without manual DNS plumbing.

Scope:

- Add `family` and possibly `address_order` options to:
  - `tcp_connect()`
  - `reachable()` TCP fallback
  - `port_scan()`
  - `tls_info()`
- Consider traceroute only if it reuses the same resolver helper cleanly.
- Extract/centralize address filtering to avoid four subtly different implementations.

Tests:

- IPv4-only filter keeps IPv4 and drops IPv6.
- IPv6-only filter keeps IPv6 and drops IPv4.
- Empty filtered result returns clear error.
- Policy still checks every candidate address after filtering.
- Resolver ordering is deterministic in tests.

### PR 5: TLS check convenience or TCP banner read

**Goal:** add one more user-facing diagnostic primitive only if PRs 1-4 are stable.

Choose one, not both, unless the release still feels tiny:

- `tls_info()` policy options / `tls_check()` for certificate audit UX.
- `tcp_banner()` for bounded service banner diagnostics.

Recommendation:

- Prefer TLS check if the target use case is certificate monitoring.
- Prefer TCP banner only if there is a real app wanting SMTP/SSH/FTP/etc. banner checks.
- Do not add generic TCP send/receive in v0.4.12.

### PR 6: v0.4.12 docs and examples

**Goal:** make the final release coherent.

Scope:

- Add or update examples only for features that actually landed.
- Update DD-064 status from Draft to Implemented/Partially Implemented with PR links.
- Update `ROADMAP.md` if the release process uses it as the canonical checklist.
- Ensure `docs/STDLIB_REFERENCE.md` and `docs/AI_AGENT_GUIDE.md` agree.

Verification:

```bash
cargo fmt
cargo build --profile dev-release
./target/dev-release/ntnt docs --generate
git diff --check
cargo test
```

---

## Suggested v0.4.12 Minimum Viable Scope

If we want a tight release, ship only:

1. Result consistency cleanup.
2. `local_interfaces()`.
3. Address-family controls for `ping()`, TCP, TLS, reachability, and `port_scan()`.
4. DNS `nameservers` option, if clean.

Defer:

- `dns_lookup_many()` if DNS extraction gets too big.
- `default_route()` if cross-platform behavior is ugly.
- `local_addr_for()` if policy semantics become confusing.
- `tls_check()` unless certificate monitoring needs it immediately.
- `tcp_banner()` unless a real app needs service banners.

This gives v0.4.12 a clear theme: **make the existing primitives easier to use correctly across real host/network environments**.

---

## Open Decisions

1. Should `reason_code` be added in v0.4.12, or should we keep string-only reasons until a broader result-shape cleanup?
2. Which crate, if any, should back `local_interfaces()`? Requirement: maintained, cross-platform, no excessive dependency tree.
3. Should explicit DNS nameservers be allowed for public targets by default, or require an opt-in because custom resolvers can be used for internal discovery?
4. Should `family` apply to DNS lookup itself, or only to functions that resolve and then connect/probe?
5. Is `tls_check()` enough convenience to justify a separate public API, or should it remain options on `tls_info()`?
6. Does `tcp_banner()` belong in default `std/net`, or is it better as a private monitoring helper until a real app proves the need?

---

## Release Acceptance Criteria

For any PR implementing this DD:

- Every public function has:
  - runtime implementation
  - typechecker signature
  - `// @ntnt` docs
  - generated docs
  - AI guide coverage when the behavior is non-obvious
  - unit and/or integration tests
- No test requires public internet, raw socket privileges, root, or Docker capabilities by default.
- Private/internal targets preserve DD-046 policy:
  - process opt-in: `NTNT_NET_ALLOW_PRIVATE=1`
  - per-call opt-in: `allow_private: true`
  - user-controlled opts alone cannot bypass policy
- Bounds are documented and tested.
- Result ordering is deterministic.
- `cargo fmt`, `cargo build --profile dev-release`, `./target/dev-release/ntnt docs --generate`, focused std/net tests, and full `cargo test` pass before push.

---

## Final Recommendation

For v0.4.12, resist the temptation to build a grand network diagnostics suite. Add the boring primitives that remove friction:

- local interface visibility
- address-family controls
- better DNS resolver ergonomics
- consistent probe result metadata

Leave parallel traceroute, SNMP, topology, sweeps, and monitoring state to `std/netmon` or app-level jobs. That keeps `std/net` useful, safe, and not wearing a fake mustache while pretending to be Nagios.
