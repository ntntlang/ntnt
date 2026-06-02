# DD-046: `std/net` — Safe Network Primitives for ntnt

**Status:** PR 1 implemented and under review; PRs 2-5 planned
**Author:** Larri
**Created:** 2026-03-22
**Updated:** 2026-06-02
**Target baseline:** v0.4.10 (`std/net` track)

---

## Summary

Add a focused `std/net` module for safe, bounded network primitives: IP/CIDR utilities, first-shot host reachability checks, TCP connectivity probes, DNS lookup, limited port scanning, and TLS certificate inspection.

This is **not** a grab-bag for every network-adjacent operation. The first version should make common monitoring and diagnostics possible without shelling out, while preserving ntnt's safety posture for apps that run on the public web.

Initial scope:

- `ip_parse(ip_or_cidr)`
- `subnet_contains(cidr, ip)`
- `subnet_overlaps(a, b)`
- `subnet_split(cidr, new_prefix, opts?)`
- `subnet_supernet(cidr, new_prefix?)`
- `subnet_summarize(cidrs)`
- `ip_range_to_cidrs(start_ip, end_ip)`
- `ping(host, opts?)`
- `tcp_connect(host, port, opts?)`
- `dns_lookup(name, record_type?, opts?)`
- `dns_reverse(ip, opts?)`
- `port_scan(host, ports, opts?)` with strict bounds
- `tls_info(host, opts?)`

Explicitly deferred:

- `traceroute` and other raw packet path-discovery tools
- SSH remote command execution
- SNMP polling/walking
- WHOIS
- packet collectors, ARP scans, BGP, bandwidth tests, and other specialized monitoring tools

The original DD had the right instinct — network operations are a great ntnt use case — but the scope was too broad and not sharp enough about SSRF/security, blocking behavior, testability, and dependency reality. Tiny router-shaped confetti cannon. We are putting the pin back in.

---

## Motivation

ntnt already covers high-level HTTP (`std/http`), web serving (`std/http/server`), databases, KV, jobs, auth, and concurrency. But below HTTP, developers still have to drop to external binaries or custom Rust/Python for basic diagnostics:

- Is this host:port reachable?
- Is this host reachable at all, without making the user understand Linux capabilities first?
- Which address records does this name resolve to?
- Is this CIDR containing that IP or child subnet?
- Which subnets overlap before I allocate one twice and ruin Tuesday?
- How do I split a `/24` into `/28`s, summarize routes, or convert an IP range into CIDRs?
- Which of these explicitly listed ports are open?
- Is this TLS certificate close to expiry?

Those primitives matter for:

- uptime monitoring
- internal service health checks
- infrastructure dashboards
- DNS/certificate audits
- agent-built diagnostics tooling
- small MSP/ISP/network-engineering tools

The important product idea: `std/net` should make ntnt good at **bounded, auditable network checks** that work on the first try, not turn every ntnt app into a raw socket toolkit.

---

## Current State

As of the v0.4.9 baseline:

- There is no `src/stdlib/net.rs`.
- `src/stdlib/mod.rs` has no `std/net` registry entry.
- `src/typechecker.rs` has no `std/net` module signatures.
- `docs/STDLIB_REFERENCE.md` and `docs/AI_AGENT_GUIDE.md` do not document `std/net`.
- `std/http::fetch()` already has SSRF/private-IP policy logic. `std/net` must not silently bypass that posture.
- `Cargo.toml` uses `reqwest` with its current native TLS dependency path. `rustls` / `webpki` should not be assumed available unless explicitly added.

---

## Design Principles

1. **Safe by default.** Network primitives can become SSRF and scanning tools. Public-web apps must not accidentally probe metadata services, localhost, or private networks from user-controlled input.

2. **Diagnostics use `Ok` for expected probe outcomes.** A closed port, refused connection, DNS NXDOMAIN, or TLS validation failure is often the thing being measured. Reserve `Err(String)` for invalid input, unsupported options, missing permissions, policy-denied targets, or system/config failures.

3. **Strict bounds.** Every operation has timeout clamps and input limits. `port_scan` must have max ports, max concurrency, deterministic ordering, and cancellation/yield checks.

4. **Synchronous first, honest about it.** Current stdlib native functions are synchronous. Do not claim async/non-blocking behavior unless the implementation actually has an async execution model. Long-running checks belong in `std/jobs` or `std/concurrent`, not inline in latency-sensitive route handlers.

5. **No shellouts.** The implementation should not invoke `ping`, `dig`, `nmap`, `openssl`, `whois`, or `ssh`. If a capability requires a large/privileged dependency, defer it rather than pretending it is simple.

6. **No permission hell.** Default APIs should not require root, `CAP_NET_RAW`, Docker `cap_add`, or sysctl tuning. If ICMP is available through unprivileged OS support, use it. If it is not, `ping(..., method: "auto")` must fall back to an unprivileged TCP reachability probe and report which method was used.

7. **CI-safe by default.** Tests should not require public internet, raw socket capabilities, root, or Docker privileges. External-network and raw-socket tests are opt-in.

8. **Cross-layer contract from day one.** Every public function needs runtime registration, `// @ntnt` docs, typechecker signatures, generated docs, examples where safe, and tests.

---

## Network Safety Policy

`std/net` must reuse or generalize the existing outbound safety posture used by `std/http`.

### Default policy

For production/web-server use, deny targets that resolve to or are directly specified as:

- loopback (`127.0.0.0/8`, `::1`)
- link-local (`169.254.0.0/16`, `fe80::/10`)
- RFC1918 private IPv4 ranges
- unique-local IPv6 ranges
- cloud metadata endpoints such as `169.254.169.254`
- multicast, unspecified, and documentation ranges where appropriate

This is conservative. The use case for `std/net` includes internal monitoring, but the default must not hand an SSRF primitive to every web app.

### One-command opt-in for internal monitoring

Internal/private network checks should require explicit configuration, but the happy path for monitoring apps must be a single process-level setting, not a scavenger hunt through OS capabilities and per-call flags.

Preferred approach:

- Reuse `NTNT_ALLOW_PRIVATE_IPS` if we decide it should govern all outbound network access.
- Or introduce `NTNT_NET_ALLOW_PRIVATE=1` if `std/net` needs a separate monitoring-specific opt-in.
- Document the blessed monitoring setup as:

  ```bash
  NTNT_NET_ALLOW_PRIVATE=1 ntnt run monitor.tnt
  ```

- For Docker examples, set the env var only. Do **not** require `cap_add: [NET_RAW]` for the default `ping()` path.

The implementation plan should decide this in PR 0, not scatter policy choices per function.

Recommended decision: introduce `NTNT_NET_ALLOW_PRIVATE=1` for `std/net`, while allowing a future shared outbound policy helper to read both names. `std/http` and `std/net` have related SSRF risks, but `std/net` is much more likely to be intentionally used for internal monitoring. A dedicated env var makes that intent auditable.

### Per-call options

If per-call override exists, it must be explicit and auditable:

```ntnt
tcp_connect("10.0.0.5", 5432, map { "allow_private": true })
```

That option should only work when the process-level config also allows private targets. A user-controlled map must not be enough to bypass server policy.

### Default safety matrix

`std/net` should be easy for public targets and deliberate for private ones:

- Public IP/hostname target: allowed by default.
- Loopback/private/link-local target: denied by default in all server/runtime modes.
- Loopback/private/link-local target with `allow_private: true` but no process-level opt-in: `Err("Network target denied by policy: private targets require NTNT_NET_ALLOW_PRIVATE=1")`.
- Loopback/private/link-local target with process-level opt-in and `allow_private: true`: allowed.
- User-controlled target strings in public web apps: still the app's responsibility to validate input, but stdlib policy blocks the worst SSRF targets by default.

This keeps the first-shot developer experience clean for the common public-host case while making internal monitoring a visible deployment choice, not a hidden code-path surprise.

### DNS rebinding and multi-address behavior

Functions that accept hostnames must apply policy after resolution to **every resolved address** they might connect to. If all addresses are denied, return `Err("Network target denied by policy: ...")`.

For `tcp_connect`, choose and document one of these:

- try addresses in resolver order until one connects or all fail
- prefer IPv6/IPv4 with a simple ordering
- implement Happy Eyeballs later

Initial recommendation: try resolved addresses in order with per-address timeout budget, returning the first success and a structured failure when all fail.

---

## API Contracts

### Common option conventions

Use millisecond suffixes to avoid ambiguity:

- `timeout_ms`
- `interval_ms`
- `connect_timeout_ms`

Clamp rather than trust unbounded inputs:

- minimum timeout: 50ms
- default timeout: 2000ms for TCP/scan, 5000ms for DNS/TLS
- maximum timeout: 30000ms unless a future API justifies more
- `ping.count`: default 4, hard maximum 10
- `ping.interval_ms`: default 250ms, hard minimum 100ms, hard maximum 5000ms
- `subnet_split.max_results`: default 4096, hard maximum 65536 unless streaming/iterator support exists
- `subnet_summarize.max_inputs`: default/hard maximum 4096 for first implementation
- `ip_range_to_cidrs.max_results`: default 4096, hard maximum 65536
- `port_scan.max_ports`: 1024 by default/hard maximum for first implementation
- `port_scan.concurrency`: default 20, hard maximum 100

Exact numbers can be tuned during implementation, but the DD must require clamps.

### Result semantics

Use these rules consistently:

- `Ok(map { ... "reachable": true/false ... })` for ordinary ping/reachability outcomes.
- `Ok(map { ... "connected": false ... })` for ordinary TCP refused/timeout results.
- `Ok([])` for DNS names that have no records of the requested type when the resolver returns a clean no-answer response.
- `Err(String)` for invalid host/port/options, resolver/system errors, policy denial, or permission/capability failure.
- `ping(..., map { "method": "icmp" })` may return `Err` when the OS denies ICMP capability. `ping(..., map { "method": "auto" })` should not: it should fall back to TCP reachability and include `method: "tcp"`, `fallback_from: "icmp"`, and `permission_limited: true`.
- `Ok(map { "valid": false, "validation_error": ... })` for TLS certificates that connect but fail validation.
- `Err(String)` for TLS handshake/connect failures where no certificate information can be obtained.

This gives monitoring apps useful data without making normal probe failures feel like language/runtime failures.

---

## Phase 0 — Scaffolding, Policy, and Shared Helpers

Goal: create the module and safety foundation before exposing broad network behavior.

Files:

- Create: `src/stdlib/net.rs`
- Modify: `src/stdlib/mod.rs`
- Modify: `src/typechecker.rs`
- Later generated: `docs/STDLIB_REFERENCE.md`
- Later manual docs: `docs/AI_AGENT_GUIDE.md`, `examples/README.md`

Work:

- Add `std/net` runtime module registration.
- Add `std/net` typechecker module signatures as functions land.
- Add shared helpers for:
  - options-map parsing
  - integer-to-port validation (`1..=65535`)
  - timeout/concurrency clamps
  - IP/host normalization
  - target safety classification
  - resolved-address policy checks
- Add `// @ntnt` docs from the first public function onward.
- Add deterministic unit tests for parsing/clamping/policy classification.

Verification:

```bash
cargo fmt
cargo build --profile dev-release
cargo test net -- --test-threads=1
./target/dev-release/ntnt docs --generate
```

---

## Phase 1 — IPAM-Grade IP/CIDR Utilities + First-Shot Ping

Ship first because this gives developers an immediately useful `std/net` module without making them solve Docker capabilities before breakfast. IP/CIDR helpers should be good enough for real IPAM/routing calculators, not just demo parsing. `ping()` ships with an unprivileged default path so the first example works on ordinary machines and containers.

Phase 1 IPAM goals:

- IPv4 and IPv6 support from the start.
- Deterministic, pure functions that do not touch the network.
- No silent integer overflow. Large IPv6 counts are strings.
- No unbounded array generation. Splits/ranges require result caps.
- Function names should distinguish parsing/classification from subnet math. Do not make `ip_parse()` the one function that knows everything and eventually needs a therapist.

### `ip_parse(ip_or_cidr) -> Result<Map, String>`

Parse either an address or CIDR and return canonical fields plus classification. This includes IPv6.

Examples:

```ntnt
import { ip_parse } from "std/net"

let info = ip_parse("192.168.1.0/24")
// Ok(map {
//   "ip": "192.168.1.0",
//   "prefix": 24,
//   "network": "192.168.1.0",
//   "broadcast": "192.168.1.255",
//   "first_host": "192.168.1.1",
//   "last_host": "192.168.1.254",
//   "host_count": 254,
//   "version": 4,
//   "is_private": true
// })
```

IPv6 note: IPv6 host counts can exceed `i64`. Return `host_count` as a string for large IPv6 networks or omit host-count fields for IPv6 ranges where they are not useful. Do not silently overflow.

Recommended fields:

- `input`: original input string
- `kind`: `"address"` or `"network"`
- `version`: `4` or `6`
- `ip`: canonical/compressed address string
- `expanded`: expanded IPv6 form when useful; omit or equal `ip` for IPv4
- `prefix`: present for CIDR input, absent or `None` for bare IP input
- `network`: present for CIDR input
- `first`: first address in the range
- `last`: last address in the range
- `total_addresses`: string for all CIDRs to avoid overflow surprises
- `usable_hosts`: string or `None`; IPv4 `/31` and `/32` semantics must be explicit
- `broadcast`: IPv4 only, `None` for IPv6
- `netmask`: IPv4 dotted-quad for IPv4 networks
- `wildcard_mask`: IPv4 dotted-quad for IPv4 networks
- `reverse_zone`: reverse DNS zone for CIDR prefixes that align cleanly on nibble/octet boundaries; otherwise `None`
- classification booleans: `is_private`, `is_loopback`, `is_link_local`, `is_multicast`, `is_unspecified`, `is_documentation`, `is_unique_local`

### `subnet_contains(cidr, ip_or_cidr) -> Result<Bool, String>`

```ntnt
import { subnet_contains } from "std/net"

subnet_contains("10.0.0.0/8", "10.50.100.200")
// Ok(true)

subnet_contains("10.0.0.0/8", "10.50.0.0/16")
// Ok(true)
```

If the second argument is a CIDR, return true only when the entire child subnet fits inside the parent.

### `subnet_overlaps(a, b) -> Result<Bool, String>`

```ntnt
import { subnet_overlaps } from "std/net"

subnet_overlaps("10.0.0.0/24", "10.0.0.128/25")
// Ok(true)
```

Return `Err` for mixed address families instead of pretending IPv4 and IPv6 are comparable.

### `subnet_split(cidr, new_prefix, opts?) -> Result<Array<String>, String>`

```ntnt
import { subnet_split } from "std/net"

subnet_split("192.168.1.0/24", 28)
// Ok(["192.168.1.0/28", "192.168.1.16/28", ...])
```

Rules:

- `new_prefix` must be longer than the input prefix.
- IPv4 and IPv6 both supported.
- Enforce `max_results` to prevent generating enormous IPv6 arrays.
- Return subnets in numeric order.

### `subnet_supernet(cidr, new_prefix?) -> Result<String, String>`

```ntnt
import { subnet_supernet } from "std/net"

subnet_supernet("192.168.1.0/24")
// Ok("192.168.0.0/23")

subnet_supernet("192.168.1.0/24", 16)
// Ok("192.168.0.0/16")
```

Default `new_prefix` should be one bit shorter. Explicit `new_prefix` must be shorter than the current prefix.

### `subnet_summarize(cidrs) -> Result<Array<String>, String>`

Summarize adjacent/overlapping CIDRs into the shortest equivalent route list.

```ntnt
import { subnet_summarize } from "std/net"

subnet_summarize(["10.0.0.0/25", "10.0.0.128/25"])
// Ok(["10.0.0.0/24"])
```

Rules:

- IPv4 and IPv6 supported.
- Mixed families return `Err`.
- Inputs are normalized before summarizing.
- Output is sorted by address then prefix.

### `ip_range_to_cidrs(start_ip, end_ip) -> Result<Array<String>, String>`

Convert an inclusive IP range into the minimal CIDR cover.

```ntnt
import { ip_range_to_cidrs } from "std/net"

ip_range_to_cidrs("192.168.1.20", "192.168.1.45")
// Ok(["192.168.1.20/30", "192.168.1.24/29", ...])
```

Rules:

- Same address family required.
- `start_ip <= end_ip` required.
- Enforce result caps.

Candidate follow-up helpers for a later IPAM phase:

- `ip_add(ip, offset)` / `ip_sub(ip, offset)` for safe address arithmetic
- `cidr_exclude(parent, child)` for carve-outs/reservations
- `subnet_nth(cidr, new_prefix, index)` for allocating a subnet without materializing every sibling
- `subnet_index(parent, child)` for locating a child subnet inside a larger block
- `prefix_to_netmask(prefix)` and `netmask_to_prefix(mask)` if users want Cisco-style calculators
- `longest_prefix_match(ip, routes)` for route-table tooling

Implementation choice:

- Prefer `ipnet` or equivalent for boring-correct IPv4/IPv6 CIDR math unless manual implementation is clearly smaller and well-tested.
- Avoid crates that only partially support IPv6 or make host iteration too easy to misuse.
- Internally represent addresses as `u128` plus version/prefix for arithmetic; convert at the boundary.

Tests:

- IPv4 address only
- IPv4 CIDR `/24`, `/32`, `/0`
- IPv6 address and CIDR
- invalid IP, invalid prefix, mismatched IP families
- private/loopback/link-local classification
- IPv6 compression/expansion/canonicalization
- IPv6 `/64`, `/127`, `/128`, and large-count behavior
- subnet contains IP and subnet
- overlap true/false cases
- split/supernet/summarize/range-to-CIDR deterministic ordering
- result caps for explosive splits/ranges

### `ping(host, opts?) -> Result<Map, String>`

`ping()` is a host reachability probe. Its default behavior prioritizes "works on the first shot" over purity about ICMP.

Default method:

```ntnt
import { ping } from "std/net"

let result = ping("example.com")
// Ok(map {
//   "host": "example.com",
//   "reachable": true,
//   "method": "icmp",
//   "sent": 4,
//   "received": 4,
//   "loss_percent": 0,
//   "min_ms": 12.4,
//   "avg_ms": 14.1,
//   "max_ms": 18.7,
//   "permission_limited": false
// })
```

If ICMP is unavailable, default `method: "auto"` falls back to TCP reachability:

```ntnt
ping("example.com")
// Ok(map {
//   "host": "example.com",
//   "reachable": true,
//   "method": "tcp",
//   "fallback_from": "icmp",
//   "permission_limited": true,
//   "ports_tried": [443, 80],
//   "connected_port": 443,
//   "latency_ms": 22.8
// })
```

Options:

- `method`: `"auto"` (default), `"icmp"`, or `"tcp"`
- `count`: default 4, clamped to `1..=10`
- `timeout_ms`: default 2000, clamped to shared timeout bounds
- `interval_ms`: default 250, clamped to shared interval bounds
- `tcp_ports`: default `[443, 80]`, max 10 explicit ports
- `allow_private`: default false, requires process-level `NTNT_NET_ALLOW_PRIVATE=1`

Semantics:

- `method: "auto"` should never fail solely because ICMP permissions are missing. It should use ICMP when available and TCP fallback otherwise.
- `method: "icmp"` means the caller explicitly asked for ICMP. If the OS lacks permission/capability, return `Err("ICMP ping unavailable: ...")` with actionable guidance.
- `method: "tcp"` performs TCP connect probes only and returns `reachable: true` if any configured port connects.
- `reachable: false` is an ordinary probe result, not an `Err`.
- Include `resolved_addrs` only if doing so does not leak denied/private resolved targets in policy errors.

Implementation approach:

- Build the API around a small internal `ReachabilityMethod::{Auto, Icmp, Tcp}` parser.
- Implement TCP fallback first using the same lower-level connect helper planned for `tcp_connect`.
- Add ICMP support only through a crate/path that supports unprivileged operation where the OS allows it. Linux may use datagram ICMP sockets when `/proc/sys/net/ipv4/ping_group_range` permits the process group; raw sockets still require `CAP_NET_RAW` and must be optional.
- Do not shell out to system `ping`.
- Do not require Docker `cap_add` for the default example. Docker capability docs are for `method: "icmp"`, not the default `auto` path.

Tests:

- `method` parser accepts `auto`, `icmp`, `tcp` and rejects unknown values
- option clamps for `count`, `interval_ms`, `timeout_ms`, and `tcp_ports`
- local TCP listener fallback returns `reachable: true` without ICMP capability
- closed local TCP fallback returns `reachable: false`
- forced `method: "icmp"` maps missing capability to clear `Err` in an opt-in/unit-testable path
- policy-denied private/loopback target unless explicitly allowed by config/test override

---

## Phase 2 — TCP Connectivity Probe

### `tcp_connect(host, port, opts?) -> Result<Map, String>`

```ntnt
import { tcp_connect } from "std/net"

let result = tcp_connect("db.internal", 5432, map {
    "timeout_ms": 1000,
    "allow_private": true
})

// Ok(map {
//   "host": "db.internal",
//   "port": 5432,
//   "connected": true,
//   "latency_ms": 2.3,
//   "remote_addr": "10.0.1.10:5432",
//   "local_addr": "10.0.1.5:48230"
// })
```

Failure example:

```ntnt
tcp_connect("example.com", 81, map { "timeout_ms": 500 })
// Ok(map {
//   "host": "example.com",
//   "port": 81,
//   "connected": false,
//   "reason": "timeout"
// })
```

Implementation notes:

- Use `std::net::ToSocketAddrs` and `TcpStream::connect_timeout` initially.
- Apply safety policy to every resolved address before connecting.
- Measure latency with `Instant`.
- Set read/write timeouts on the connected stream before inspecting local/remote addr, then drop it.
- Include host:port in diagnostic errors.
- Avoid unbounded DNS/connect time; clamp options.

Tests:

- local `TcpListener` open port returns `connected: true`
- closed local port returns `connected: false`
- invalid port and invalid opts return `Err`
- policy-denied private/loopback target unless explicitly allowed by config/test override
- typechecker rejects non-string host and non-int port

---

## Phase 3 — DNS Lookup

### `dns_lookup(name, record_type?, opts?) -> Result<Array<Map>, String>`

Supported initial record types:

- `A`
- `AAAA`
- `PTR`

Candidate follow-up record types once the shape is proven:

- `MX`
- `TXT`
- `NS`
- `CNAME`
- `SOA`
- `SRV`

```ntnt
import { dns_lookup } from "std/net"

dns_lookup("example.com", "A")
// Ok([map { "type": "A", "value": "93.184.216.34", "ttl": 300 }])
```

### `dns_reverse(ip, opts?) -> Result<Array<String>, String>`

Return an array, not a single string. PTR can have multiple names, and `[]` is a clearer no-answer shape than choosing one.

```ntnt
import { dns_reverse } from "std/net"

dns_reverse("8.8.8.8")
// Ok(["dns.google."])
```

Implementation notes:

- Use `hickory-resolver` only when ready to add the dependency intentionally.
- Do not rely on public DNS in CI.
- Build a resolver abstraction or local/mock DNS test fixture before adding many record types.
- Custom nameserver support is deferred until safety implications are explicit. A `server` option can bypass enterprise DNS policy and should not be a casual Phase 1 feature.

Tests:

- record-type parser
- no-answer behavior
- invalid type returns `Err`
- local/mock resolver results for A/AAAA/PTR
- optional external DNS smoke test gated behind env var, not default CI

---

## Phase 4 — Bounded Port Scan

### `port_scan(host, ports, opts?) -> Result<Array<Map>, String>`

```ntnt
import { port_scan } from "std/net"

let results = port_scan("192.168.1.1", [22, 80, 443], map {
    "timeout_ms": 500,
    "concurrency": 20,
    "allow_private": true
})
```

Result shape:

```ntnt
Ok([
    map { "port": 22, "open": true, "latency_ms": 1.2 },
    map { "port": 80, "open": false, "reason": "refused" },
    map { "port": 443, "open": true, "latency_ms": 0.9 }
])
```

Implementation notes:

- Build on the same lower-level connect helper used by `tcp_connect`.
- Accept arrays of ints first. Range syntax can be added only if runtime and typechecker behavior are confirmed together.
- Enforce max ports and max concurrency.
- Return results sorted by port.
- Check cancellation/yield points between batches.
- Treat this as the highest abuse-risk initial API; policy checks are mandatory.

Tests:

- local open/closed ports
- deterministic sorted order
- rejects duplicate/invalid/out-of-range ports
- rejects too many ports
- clamps concurrency
- policy-denied targets

---

## Phase 5 — TLS Certificate Inspection

### `tls_info(host, opts?) -> Result<Map, String>`

Use an options map instead of a positional optional port to keep room for SNI and validation options:

```ntnt
import { tls_info } from "std/net"

let cert = tls_info("example.com", map {
    "port": 443,
    "timeout_ms": 5000,
    "server_name": "example.com"
})
```

Result shape:

```ntnt
Ok(map {
    "host": "example.com",
    "port": 443,
    "subject": "example.com",
    "issuer": "DigiCert Inc",
    "not_before": "2025-01-15T00:00:00Z",
    "not_after": "2026-12-01T23:59:59Z",
    "days_left": 254,
    "serial": "0A:1B:2C:...",
    "san": ["example.com", "www.example.com"],
    "valid": true,
    "validation_error": None,
    "protocol": "TLSv1.3"
})
```

Implementation decision required before coding:

- Either use `native-tls` / platform cert store and an X.509 parser, or explicitly add `rustls`, roots, and `x509-parser`.
- Do not claim `rustls` is already available via `reqwest`; current dependency state does not justify that.

Tests:

- local TLS server with deterministic self-signed cert
- returns certificate fields even when validation fails
- expiry date parsing/days-left math with fixed test clock helper if needed
- invalid host/port/options
- policy-denied target

---

## Deferred / Out of Scope for Initial `std/net`

### `traceroute` and raw packet tooling

Traceroute and other raw packet path-discovery tools require a separate capability story:

- runtime capability detection or `net_capabilities()` helper
- clear Docker docs (`cap_add: [NET_RAW]`) for APIs that truly require raw sockets
- CI opt-in via something like `NTNT_RUN_RAW_NET_TESTS=1`
- graceful `Err` when unavailable
- no default CI dependency on raw sockets

`ping()` is **not** deferred. It belongs in Phase 1, but its default `auto` method must avoid permission hell by falling back to TCP reachability when ICMP is unavailable.

### WHOIS

WHOIS is domain registry plumbing, not core connectivity. It also involves referral chains, inconsistent formats, rate limits, and public-network CI flakiness. Defer until someone needs it enough to design it separately.

### SSH execution

Remote command execution is too privileged for `std/net`. If added, it should likely be `std/ssh` or an app/plugin-level module with explicit host-key verification and credential handling. Default-permissive known-host behavior is not acceptable for a stdlib primitive.

### SNMP / network monitoring

SNMP and higher-level monitoring concerns are covered by [DD-047: `std/netmon`](dd-047-std-netmon.md). Keep them out of the initial `std/net` module. `std/net` should provide safe primitives; `std/netmon` can build SNMP/device telemetry, interface counters, topology hints, composite checks, and alert-state helpers on top.

SNMP is a real network-monitoring need, but it is its own protocol family. It should likely start as a private or separately distributed `std/netmon` library rather than default stdlib surface area. Do not make the first `std/net` PR carry BER/ASN.1 and SNMP semantics.

---

## Implementation Plan

### Status Dashboard

As of 2026-06-02:

- [x] **PR 1 — `std/net` shell + IPAM helpers + `ping`**: implemented in [PR #113](https://github.com/ntntlang/ntnt/pull/113). Status: open, mergeable, CI green, no unresolved review threads.
- [ ] **PR 2 — Dedicated TCP probe refinement**: next planned slice. Reuses the Phase 1 TCP fallback helper as public `tcp_connect`.
- [ ] **PR 3 — DNS A/AAAA/PTR**: planned after TCP probe.
- [ ] **PR 4 — Bounded port scan**: planned after DNS.
- [ ] **PR 5 — TLS info**: planned after port scan/dependency decision.

PR 1 shipped the core module registration, runtime functions, typechecker signatures, generated docs, deterministic examples, and tests for Phase 1. Review hardening added policy fixes for private, link-local, multicast, documentation, mapped-address, and broadcast-style targets.

### PR 1 — `std/net` shell + IPAM helpers + `ping`

Status: **implemented in PR #113; awaiting merge/review completion.**

Scope:

- [x] `src/stdlib/net.rs`
- [x] `src/stdlib/mod.rs`
- [x] `src/typechecker.rs`
- [x] unit tests for helpers and IPv4/IPv6 IPAM behavior
- [x] `ip_parse`, `subnet_contains`, `subnet_overlaps`, `subnet_split`, `subnet_supernet`, `subnet_summarize`, `ip_range_to_cidrs`
- [x] `ping(host, opts?)` with `method: "auto"` default and TCP fallback
- [x] shared target safety checks used by `ping` and future TCP functions
- [x] generated docs for all Phase 1 functions
- [x] safe example showing `ping("example.com")` and an internal-monitoring env-var example
- [x] IPAM examples for IPv4 subnet splitting and IPv6 parsing/summarization

Acceptance:

- [x] Phase 1 imports work at runtime and lint/typecheck time.
- [x] IPv6 parsing, containment, overlap, splitting, supernet, summarization, and range conversion are supported and tested.
- [x] Large IPv6 counts/results do not overflow or generate unbounded arrays.
- [x] `ping("example.com")` has a documented no-root/no-capability path through TCP fallback.
- [x] Missing ICMP capability does not break default `ping()` usage.
- [x] no public internet dependency
- [x] no new dependency unless clearly justified; any ICMP dependency must keep the TCP fallback path clean
- [x] `cargo build --profile dev-release`, `cargo test net`, docs generation pass

### PR 2 — Dedicated TCP probe refinement

Scope:

- [ ] `tcp_connect`
- [ ] reuse/refactor the Phase 1 TCP fallback helper into the public `tcp_connect` API
- [ ] local-listener tests
- [ ] typechecker signature tests
- [ ] AI agent guide section for `std/net` safety and jobs/concurrency guidance

Acceptance:

- [ ] local open/closed port tests pass
- [ ] private/loopback policy behavior is explicit and tested
- [ ] `Err` vs `Ok({ connected: false })` semantics are documented and tested

### PR 3 — DNS A/AAAA/PTR

Scope:

- [ ] `dns_lookup`
- [ ] `dns_reverse`
- [ ] `hickory-resolver` or equivalent dependency decision
- [ ] resolver abstraction or mock/local fixture

Acceptance:

- [ ] deterministic CI tests
- [ ] no public DNS dependency by default
- [ ] initial records limited to A/AAAA/PTR unless implementation remains small and clean

### PR 4 — Bounded port scan

Scope:

- [ ] `port_scan` over explicit port arrays
- [ ] bounds and cancellation checks
- [ ] local open/closed test fixture

Acceptance:

- [ ] rejects too many ports/concurrency/invalid ports
- [ ] deterministic order
- [ ] no unbounded scanning

### PR 5 — TLS info

Scope:

- [ ] dependency choice
- [ ] `tls_info`
- [ ] local TLS test server
- [ ] generated docs and examples

Acceptance:

- [ ] returns certificate details for valid and validation-failing certs
- [ ] no dependency claim mismatch
- [ ] no public internet dependency by default

---

## Cross-Layer Touchpoints

Every public function must update all layers:

1. Runtime module:
   - `src/stdlib/net.rs`
   - `Value::NativeFunction` arity/max_arity matching actual args
   - returns `Value::ok(...)` / `Value::err(...)` for documented `Result`

2. Module registry:
   - `src/stdlib/mod.rs`
   - `pub mod net;`
   - `modules.insert("std/net", net::init())`

3. Typechecker:
   - `src/typechecker.rs`
   - `get_module_signatures("std/net")`
   - focused tests for wrong arg types and optional args

4. Docs:
   - `// @ntnt` blocks in source
   - `./target/dev-release/ntnt docs --generate`
   - `docs/AI_AGENT_GUIDE.md` for practical usage and safety posture

5. Examples:
   - deterministic examples only for default validation
   - public-network examples clearly marked as manual/optional if included

---

## Verification Checklist

For each implementation PR:

```bash
cargo fmt
cargo build --profile dev-release
cargo test --lib
cargo test --test type_checker_tests
./target/dev-release/ntnt docs --generate
./target/dev-release/ntnt validate examples/
./target/dev-release/ntnt lint examples/*.tnt
```

For network-specific PRs:

- tests use local listeners or mocks by default
- public internet tests are opt-in only
- raw socket tests are opt-in only
- timeout clamps are tested
- policy-denied targets are tested
- docs drift is checked with `git diff docs/`

---

## Open Decisions Before Coding

1. Should private/internal targets be governed by existing `NTNT_ALLOW_PRIVATE_IPS`, or should `std/net` introduce `NTNT_NET_ALLOW_PRIVATE`?

   Recommendation: introduce `NTNT_NET_ALLOW_PRIVATE=1` for `std/net`, while implementing the check through a shared outbound-policy helper that can also honor `NTNT_ALLOW_PRIVATE_IPS` if we later want one global knob. Dedicated monitoring intent beats making `std/http` and `std/net` quietly share a footgun drawer.

2. Should `tcp_connect` return `Ok({ connected: false })` for all ordinary connect failures?

   Recommendation: yes. It is a diagnostic probe. Invalid input and policy denial remain `Err`.

3. Should `dns_reverse` return one string or all PTR hostnames?

   Recommendation: return `Result<Array<String>, String>`.

4. Should `port_scan` accept range values in the first PR?

   Recommendation: no. Start with explicit arrays; add ranges only after runtime/typechecker handling is confirmed.

5. Should TLS inspection use `native-tls` or `rustls`?

   Recommendation: decide in the TLS PR based on cert-chain extraction quality and dependency weight. Do not block IP/TCP/DNS on this.

6. Should `ping()` mean strict ICMP or pragmatic reachability?

   Recommendation: pragmatic by default. `method: "auto"` should use ICMP when available and TCP fallback when not. Strict ICMP is still available via `method: "icmp"`, but that path may require OS capability and should fail with clear guidance instead of silently degrading.

7. Should internal targets be easy to enable for monitoring?

   Recommendation: yes, but at process scope. Use `NTNT_NET_ALLOW_PRIVATE=1` plus per-call `allow_private: true`. That makes a monitoring app easy to launch while preventing a random request body from flipping the SSRF guard off.

---

## Bottom Line

The refined `std/net` path is smaller and much safer:

1. deterministic IP helpers plus first-shot `ping()`
2. dedicated TCP probe with explicit safety policy
3. DNS with mockable tests
4. bounded port scan
5. TLS inspection after dependency choice

That gets ntnt real network-monitoring capability without making the first user trip over `CAP_NET_RAW`, and without smuggling in traceroute, SSH, SNMP, scanners, public-network CI flakes, and SSRF footguns in one heroic PR. Heroic PRs are where bugs go to get tenure.
