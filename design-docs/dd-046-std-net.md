# DD-046: `std/net` — Safe Network Primitives for ntnt

**Status:** Complete — shipped across PRs [#113](https://github.com/ntntlang/ntnt/pull/113), [#114](https://github.com/ntntlang/ntnt/pull/114), [#115](https://github.com/ntntlang/ntnt/pull/115), and [#117](https://github.com/ntntlang/ntnt/pull/117)
**Author:** Larri
**Created:** 2026-03-22
**Updated:** 2026-06-07
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

- ~~`traceroute` and other raw packet path-discovery tools~~ — `traceroute()` shipped in Phase 2 (PR 7)
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

## Implemented State

As of the merged PR #117 baseline:

- `src/stdlib/net.rs` exists and registers safe `std/net` primitives.
- `src/stdlib/mod.rs` exposes the `std/net` module.
- `src/typechecker.rs` includes `std/net` signatures.
- `docs/STDLIB_REFERENCE.md` is generated from the `// @ntnt` docs, and `docs/AI_AGENT_GUIDE.md` includes practical `std/net` examples.
- `examples/std_net_ipam.tnt`, `examples/std_net_ping.tnt`, `examples/std_net_dns.tnt`, `examples/std_net_scan.tnt`, and `examples/std_net_tls.tnt` cover the shipped slices.
- `std/net` uses a dedicated outbound safety posture: public targets are easy, private/internal targets require process-level `NTNT_NET_ALLOW_PRIVATE=1` plus per-call `allow_private: true`.
- TLS inspection deliberately added `rustls`, `rustls-pki-types`, `webpki-roots`, and `x509-parser`; it does not depend on `reqwest`'s TLS stack.

---

## Design Principles

1. **Safe by default.** Network primitives can become SSRF and scanning tools. Public-web apps must not accidentally probe metadata services, localhost, or private networks from user-controlled input.

2. **Diagnostics use `Ok` for expected probe outcomes.** A closed port, refused connection, DNS NXDOMAIN, or TLS validation failure is often the thing being measured. Reserve `Err(String)` for invalid input, unsupported options, missing permissions, policy-denied targets, or system/config failures.

3. **Strict bounds.** Every operation has timeout clamps and input limits. `port_scan` must have max ports, max concurrency, deterministic ordering, and cancellation/yield checks.

4. **Synchronous first, honest about it.** Current stdlib native functions are synchronous. Do not claim async/non-blocking behavior unless the implementation actually has an async execution model. Long-running checks belong in `std/jobs` or `std/concurrent`, not inline in latency-sensitive route handlers.

5. **No shellouts.** The implementation should not invoke external command-line network tools. If a capability requires a large/privileged dependency, defer it rather than pretending it is simple.

6. **No permission hell, no silent protocol switch.** Default APIs should not require root, `CAP_NET_RAW`, Docker `cap_add`, or sysctl tuning. If ICMP is unavailable in the current runtime, `ping()` should return a clear `Err(String)` rather than quietly trying TCP ports. TCP reachability is available as an explicit developer choice through `tcp_connect()`, or through `reachable()` when the app intentionally wants high-level reachability semantics with default TCP 80/443 plus optional extra `tcp_ports`.

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

Implemented approach:

- `std/net` uses `NTNT_NET_ALLOW_PRIVATE=1` as the process-level monitoring opt-in.
- Apps still pass `allow_private: true` per call, so user-controlled option maps cannot disable policy by themselves.
- The blessed monitoring setup is:

  ```bash
  NTNT_NET_ALLOW_PRIVATE=1 ntnt run monitor.tnt
  ```

- For Docker examples, set the env var only. Do **not** require `cap_add: [NET_RAW]` for the default `ping()` path.

The dedicated env var keeps monitoring intent auditable. `std/http` and `std/net` have related SSRF risks, but `std/net` is much more likely to be intentionally used for internal monitoring, so this DD keeps the knobs separate unless a future shared outbound-policy helper is designed explicitly.

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
- Cloud metadata, multicast, broadcast, unspecified, and documentation targets: never allowed, even with private-network opt-in.
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
- `ping.count`: default 1, hard maximum 10
- `ping.interval_ms`: default 0ms, hard maximum 5000ms
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
- `ping(..., map { "method": "icmp" })` and default `ping()` may return `Err` when ICMP is unavailable or denied. They must not silently fall back to TCP. Apps that intentionally want TCP reachability should use `tcp_connect()`; apps that intentionally want high-level reachability should use `reachable()`, which probes ICMP plus default TCP 80/443 and optional extra `tcp_ports`.
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

Ship first because this gives developers an immediately useful `std/net` module. IP/CIDR helpers should be good enough for real IPAM/routing calculators, not just demo parsing. `ping()` intentionally stays protocol-honest: it does not pretend a TCP connect to arbitrary ports is an ICMP ping. If ICMP support is unavailable in Phase 1, callers get a clear `Err`; explicit TCP reachability remains an app/developer decision.

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
- classification booleans: `is_private`, `is_loopback`, `is_link_local`, `is_multicast`, `is_unspecified`, `is_documentation`, `is_broadcast`, `is_unique_local`

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

`ping()` is a host reachability probe. Its default behavior is protocol-honest: ICMP ping should either produce ICMP results or fail clearly; it must not silently switch to TCP ports.

Default method:

```ntnt
import { ping } from "std/net"

let result = ping("example.com")
// Ok(map {
//   "host": "example.com",
//   "reachable": true,
//   "method": "icmp",
//   "sent": 1,
//   "received": 1,
//   "loss_percent": 0,
//   "min_ms": 12.4,
//   "avg_ms": 14.1,
//   "max_ms": 18.7,
//   "permission_limited": false
// })
```

If ICMP is unavailable in the runtime, `ping()` fails clearly instead of falling back to TCP:

```ntnt
ping("example.com")
// Err("ICMP ping unavailable: native socket failed: permission denied")
```

Apps that intentionally want TCP reachability should use `tcp_connect()` for a single explicit port or `reachable()` for a high-level check that tries ICMP plus TCP ports 80 and 443 by default. Extra `tcp_ports` add more explicit TCP ports:

```ntnt
tcp_connect("example.com", 443, map {
  "count": 5
})
// Ok(map {
//   "host": "example.com",
//   "port": 443,
//   "connected": true,
//   "method": "tcp",
//   "sent": 5,
//   "received": 5,
//   "failed": 0,
//   "loss_percent": 0,
//   "min_ms": 18.6,
//   "avg_ms": 21.4,
//   "max_ms": 25.2,
//   "attempts": [...]
// })

reachable("example.com", map {
  "tcp_ports": [8080],
  "count": 5
})
// Ok(map {
//   "host": "example.com",
//   "reachable": true,
//   "method": "icmp", // or "tcp" when TCP establishes reachability
//   "tcp_ports_tried": [80, 443, 8080],
//   "tcp_attempts": [...]
// })
```

Options:

- `count`: default 1, clamped to `1..=10`
- `timeout_ms`: default 2000, clamped to shared timeout bounds
- `interval_ms`: default 0, clamped to `0..=5000`
- `tcp_ports`: optional extra TCP ports for `reachable()`; defaults 80 and 443 are always included, max 10 total ports; no random ports
- `allow_private`: default false, requires process-level `NTNT_NET_ALLOW_PRIVATE=1`

Semantics:

- `ping()` means ICMP ping. If ICMP is unavailable/unsupported, return `Err("ICMP ping unavailable: ...")` with actionable guidance.
- `ping(..., map { "method": "tcp" })` is rejected; TCP checks belong to `tcp_connect()` or `reachable()`.
- `tcp_connect(host, port, opts?)` performs TCP connect probes only and returns `connected: true` if the configured port connects.
- `reachable(host, opts?)` is the high-level reachability API. It probes ICMP and TCP ports 80/443 by default, adds caller-provided `tcp_ports`, and records whether reachability was established by `method: "icmp"` or `method: "tcp"`.
- `reachable: false` / `connected: false` is an ordinary probe result, not an `Err`, once the selected method is actually available and permitted.
- Include `resolved_addrs` only if doing so does not leak denied/private resolved targets in policy errors.

Implementation approach:

- Keep `ping()` ICMP-only and protocol-honest.
- Share the lower-level TCP probe substrate between `tcp_connect()` and `reachable()`.
- Do not implement automatic TCP fallback inside `ping()`; protocol fallback belongs in `reachable()` or app code.
- Add ICMP support through the native socket path. Try datagram ICMP first so Linux can use unprivileged ping sockets when `/proc/sys/net/ipv4/ping_group_range` permits the process group; fall back to raw sockets where datagram ICMP is unavailable. Raw sockets still require `CAP_NET_RAW`/root/admin depending on OS.
- Do not shell out to system `ping`.
- Do not require Docker `cap_add` for app code that deliberately chooses TCP reachability; ICMP-specific deployment docs belong to the ICMP path.

Tests:

- option clamps for `count`, `interval_ms`, `timeout_ms`, and `tcp_ports`
- default `ping()` returns a clear ICMP-unavailable `Err` rather than TCP fallback when ICMP is unsupported
- `ping(..., map { "method": "tcp" })` rejects with guidance to use `tcp_connect()` or `reachable()`
- `tcp_connect()` can run multiple bounded attempts and returns per-attempt plus aggregate summary fields
- `reachable()` probes ICMP plus default TCP ports 80/443, appends extra `tcp_ports`, and reports the method that established reachability.
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

Supported record types:

- `A`
- `AAAA`
- `ANAME`
- `CAA`
- `CDNSKEY`
- `CDS`
- `CNAME`
- `CSYNC`
- `DNSKEY`
- `DS`
- `HINFO`
- `HTTPS`
- `KEY`
- `MX`
- `NAPTR`
- `NS`
- `NSEC`
- `NSEC3`
- `NSEC3PARAM`
- `NULL`
- `OPENPGPKEY`
- `PTR`
- `RRSIG`
- `SIG`
- `SOA`
- `SRV`
- `SSHFP`
- `SVCB`
- `TLSA`
- `TXT`

Operational/meta query types such as `ANY`, `AXFR`, `IXFR`, `OPT`, `TSIG`, and `ZERO` remain unsupported.

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
- `dns_lookup` uses the system resolver and does not expose custom nameserver configuration in this slice.
- DNS answers are returned as DNS data; target probing/safety policy applies to connect-style functions (`tcp_connect`, `reachable`, `port_scan`, `tls_info`) rather than filtering A/AAAA answers from lookup results.
- Record maps report the actual returned DNS record type. If a resolver includes related records in a response, those records are not relabeled as the requested type.
- Custom nameserver support is deferred until safety implications are explicit. A `server` option can bypass enterprise DNS policy and should not be a casual Phase 1 feature.

Tests:

- record-type parser accepts the supported data-bearing query types and rejects operational/meta/unknown types
- no-answer behavior
- invalid type returns `Err`
- deterministic result rendering for generic record maps
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

Implementation decision:

- Use `rustls`, roots, and `x509-parser`.

Tests:

- local TLS server with deterministic self-signed cert
- returns certificate fields even when validation fails
- expiry date parsing/days-left math with fixed test clock helper if needed
- invalid host/port/options
- policy-denied target

---

## Phase 2 — Native Probe Toolset

Phase 1 shipped `std/net` with no shellouts except the original `ping` subprocess
backend, which Phase 2 replaced with native ICMP sockets. Phase 2 continues the
same direction: every probe is native Rust, no external tools, and the probe
substrate is shared so each new diagnostic builds on classified failure
semantics instead of reinventing them.

### Probe substrate (PR 6)

The ICMP work is structured as a reusable substrate under `src/stdlib/net/`:

- `net/probe.rs` — `ProbeFailure { Target, Backend }` carries the
  target-failure vs backend-failure distinction as a type through every probe
  layer. Classification happens once at the io boundary; string sniffing on
  composed error messages is not allowed. Shared deadline budgeting
  (`probe_attempt_budget`) divides one global timeout across attempts and
  intervals so a requested count either completes or fails loudly.
- `net/icmp.rs` — socket creation (datagram-first, raw fallback), echo packet
  construction, checksums, reply/error parsing, capability detection, and the
  `ping()` driver. Traceroute reuses everything except the driver loop.
- `net_capabilities()` — reports which probe paths the current process can use
  (datagram/raw ICMP per family, TCP) without sending any traffic, so apps and
  deploy docs can check before probing instead of parsing `Err` strings.

### `traceroute(host, opts?)` (PR 7)

Traceroute is the existing echo substrate with a stepped TTL
(`IP_TTL`/`IPV6_UNICAST_HOPS`) and Time Exceeded treated as a hop report
rather than a failure:

- [x] runtime capability detection via `net_capabilities()` (including a
      `traceroute` flag: raw ICMP available)
- [x] graceful `Err` when unavailable (shared `ProbeFailure::Backend` path,
      message names `CAP_NET_RAW` / Docker `cap_add`)
- [x] no default CI dependency on raw sockets (capability detection is
      creation-only; the integration test branches on reported capability)
- [x] per-hop TTL stepping and hop aggregation driver
      (`src/stdlib/net/traceroute.rs`)
- [x] clear Docker docs (`cap_add: [NET_RAW]`) in
      `docs/DEPLOYMENT_GUIDE.md` ("Network Probe Capabilities")
- [x] CI-safe coverage: `traceroute_is_honest_about_raw_socket_capability`
      asserts the result always matches what `net_capabilities()` reported

Design decisions:

- **Raw ICMP only.** Datagram ICMP sockets surface TTL-expiry as bare errno
  values without the reporting router's address (recovering it needs
  `IP_RECVERR` + `MSG_ERRQUEUE` cmsg parsing). A hop list without router
  addresses is not a traceroute, so the v1 driver requires a raw socket and
  fails loudly otherwise. An unprivileged errqueue-based path can be added
  later without changing the API.
- **One path per call.** Traceroute traces the first resolved address in
  resolver preference order; it does not iterate A/AAAA records the way
  `ping()` does, because hops from different target addresses cannot be
  merged meaningfully.
- **Options**: `max_hops` default 30 / hard max 64; `timeout_ms` default
  8000ms as a single global budget split across outstanding hops via the
  shared `probe_attempt_budget`; `allow_private` with the same two-level
  opt-in policy as every other probe.
- **Terminal events.** An echo reply sets `reached: true` and stops; a
  non-Time-Exceeded ICMP error (destination unreachable, administratively
  prohibited) records the hop with `error` and stops; per-hop timeouts
  record `timed_out: true` and continue.

`ping()` shipped in Phase 1 and is protocol-honest: when ICMP is unavailable it
returns a clear `Err` rather than falling back to TCP reachability. TCP
reachability remains explicit app/developer intent.

---

## Phase 3 — Multi-Protocol Traceroute & mtr-style Diagnostics (in progress)

Phase 2's `traceroute()` probes with ICMP echo. Phase 3 extends path discovery
to UDP and TCP probes (shipped) and adds the continuous, per-hop statistics
that make `mtr` useful (planned). None of this requires rethinking the substrate: the hard part of
traceroute — receiving and matching the ICMP Time Exceeded messages that
intermediate routers emit — is **independent of the probe protocol**, because a
router whose TTL/hop-limit reaches zero sends Time Exceeded regardless of what
the expiring packet contained. The receive side built in PR 7
(`create_raw_icmp_socket`, the TTL-stepping driver, `probe_attempt_budget`,
target policy, capability detection, per-hop result shaping) is reused as-is.

### Core refactor: `ProbeMethod` abstraction

What varies per protocol is only (1) how a probe is sent at a given TTL and
(2) how "the destination was reached" is recognized. Introduce a small
`ProbeMethod` (Icmp | Udp | Tcp) responsible for exactly three things:

- create/configure its send socket,
- send one probe to the target at TTL *n*,
- classify a received packet as `Hop` (Time Exceeded quoting our probe),
  `Reached` (destination response), or `Other`.

The one substrate change this requires: the matcher that confirms a Time
Exceeded *quotes our probe* currently matches the embedded ICMP echo header
(ident + sequence). It must generalize to match an embedded **UDP** header
(src/dst ports) or **TCP** header (ports + sequence). The shared driver loop,
budgeting, and result assembly stay unchanged. `traceroute()` gains a
`method` option (`"icmp"` default).

**Shipped (PR 8/9).** The abstraction is a `TraceProbe` trait (one
implementation per method) plus a shared `HopProbe` outcome enum, both in
`net/probe.rs`. The IP-header-walking that finds the quoted transport header
(`quoted_inner_v4`/`quoted_inner_v6`) and the checksums
(`internet_checksum`/`transport_checksum`) were lifted into `net/probe.rs` and
are now shared by ICMP, UDP, and TCP. The ICMP echo matcher was refactored
onto these shared helpers (behaviour unchanged, guarded by its existing
tests). UDP/TCP packet construction, the quoted-transport matcher, and their
`TraceProbe` impls live in `net/transport.rs`; the driver in
`net/traceroute.rs` is generic over `Box<dyn TraceProbe>`.

### `traceroute(..., method: "udp")` (PR 8) — shipped

Classic Unix traceroute. Sends a UDP datagram with stepped TTL; intermediate
hops yield Time Exceeded, and the destination yields ICMP **Port Unreachable**
(IPv4 type 3 / code 3, IPv6 type 1 / code 4) — but only when it comes *from the
target* (a Port Unreachable from an intermediate device is recorded as a
terminal error, not arrival). The destination port increments per hop from the
base (`port`, default 33434) so each hop's quoted UDP header is distinguishable
— without that, a delayed Time Exceeded from an earlier hop could be
mis-attributed. Reuses the raw ICMP recv socket plus a UDP send socket bound to
a known source port. `traceroute_udp` is gated to Linux: although UDP send is
unprivileged, the probe send + raw-ICMP reply handling are validated there
(Windows rejects the send with WSAEINVAL).

### `traceroute(..., method: "tcp")` (PR 9) — shipped

The highest-value mode (`mtr --tcp` / `tcptraceroute`): firewalls that drop
ICMP and UDP commonly pass TCP SYN to 80/443. Sends a raw TCP SYN (default
port 80) with stepped TTL; intermediate hops yield Time Exceeded quoting the
TCP header, and the destination yields SYN-ACK or RST — either means
"reached". Each hop sends from a distinct **source port** (base + hop), which
is recovered from the quoted TCP header on an ICMP error and from the
destination port of the target's reply — so a delayed reply is attributed to
the right hop, and the reply is matched for *any* flag combination (SYN-ACK and
every RST variant, including firewall RSTs that carry no ACK). The reply must
also come *from the target*. Because the destination reply is a TCP segment
(not ICMP), the TCP method watches two sockets — the raw ICMP socket for hops
and a raw TCP socket for the reply — with a dependency-free round-robin poll. Raw TCP reply capture only works on **Linux** (BSD/Windows
do not deliver TCP to raw sockets), so `traceroute_tcp` is reported true only
there, and a `method: "tcp"` call elsewhere returns a clear `Err`. Raw TCP
send/recv is privileged (`CAP_NET_RAW`).

**Not verifiable in CI.** Like ICMP traceroute, the raw-socket happy path
needs `CAP_NET_RAW`, which CI does not grant. Correctness rests on unit tests
of packet construction, checksums, and the quoted/​reply matchers; the
integration test asserts each method's outcome matches its capability flag.
End-to-end validation is a manual run on a capable host.

### The stdlib/app boundary (decided 2026-06-10)

Phase 2 + PR 8/9 ship the primitive: a single traceroute pass. Before building
further, a design question was resolved — **what belongs in the stdlib vs. the
application?** The rule that fell out, consistent with this DD's "primitives,
not a grab-bag" stance:

- **Stdlib provides only what the stdlib alone can**: raw-socket access
  (privileged, native) and anything *internal to a single trace* — because each
  `traceroute()` call is one complete trace, an app cannot reach inside it.
- **The application composes the rest**: looping and aggregating across traces
  is pure data math over the primitive's output — no privilege, no native code.

This reclassifies two previously-planned items as **out of stdlib scope**:

- **mtr-style cycle statistics** (loss%, last/avg/best/worst/stddev per hop):
  this is `for _ in 0..n { traceroute(host) }` plus arithmetic — a handful of
  lines of `.tnt` (or a small ntnt helper library). Baking a fixed stats shape
  into Rust bloats the privileged surface to wrap a loop, and is less flexible
  than letting the app accumulate. Left to application code.
- **Per-hop reverse DNS**: `dns_reverse(hop.from)` over the result — already
  composable from a primitive we ship (`dns_reverse`). Left to application code.

What *does* belong in the stdlib is anything internal to one trace, which an app
cannot do: parallel hop probing (below), and future per-trace options like
`probes_per_hop`, source/interface binding, and packet-size/DF controls.

### Parallel hop probing (PR 10) — in progress

`mtr` sends all TTLs concurrently rather than waiting hop-by-hop. The win is not
the happy path (a short path where every router answers is already fast); it is
the common slow cases — **silent hops** (routers that drop/limit ICMP Time
Exceeded) and **unreached destinations**, which today each cost a full per-hop
timeout *serially*. Sending TTL `1..max` up front and collecting replies turns
sum-of-timeouts into roughly a single timeout — a 5–30× wall-clock win on
exactly the traces a live/polling consumer hits, and something an app cannot do
itself (it is internal to one trace).

Design: decouple send from receive in the `TraceProbe` trait (`send(ttl, seq)` +
`recv(deadline) -> Option<HopReply>`), demultiplex replies back to their hop by
the per-hop correlation token added in PR 8/9 (echo sequence / UDP destination
port / TCP sequence number), and assemble the ordered hop list from the
collected replies (silent hops → timed out, trim beyond the first hop that
reached the destination). Parallel becomes the trace engine; the result shape is
unchanged. Bounded by `max_hops` (≤64), so the up-front burst is small.

### `probes_per_hop` (PR 11) — immediate follow-up

Sending K probes per hop *within one trace* improves the odds of eliciting a
reply from a lossy hop and yields per-hop min-RTT. It rides directly on PR 10's
send/recv split and per-probe token map (token → hop), so it is a small,
focused follow-up rather than part of the parallel-hops change.

### Future per-trace primitives (candidates)

Genuinely primitive (native/privileged, internal to a probe, not
app-composable), in rough priority:

- **`path_mtu(host)`** — PMTU discovery: increasing packet sizes with
  Don't-Fragment set, find the smallest "fragmentation needed" ICMP. A distinct,
  useful primitive reusing the raw-ICMP substrate; not app-composable.
- **Source / interface binding** — bind probes to a specific local IP/egress
  (multi-homed hosts, monitoring a specific path).
- **DSCP/ToS marking** — trace QoS-differentiated paths via a setsockopt (niche).

Explicitly *not* stdlib: cycle stats, per-hop rDNS (both app-level above), and
ASN/AS-path annotation (needs an external BGP/whois data source — not a network
primitive).

### Capability & security continuity

Every method still relies on the raw ICMP **receive** socket for intermediate
hops, so the `CAP_NET_RAW` / Docker `cap_add` story and `net_capabilities()`
plumbing from Phase 2 carry over unchanged. Target policy (private/loopback/
metadata denial with the two-level opt-in) applies to every method exactly as
it does to ICMP traceroute.

---

## Deferred / Out of Scope

### WHOIS

WHOIS is domain registry plumbing, not core connectivity. It also involves referral chains, inconsistent formats, rate limits, and public-network CI flakiness. Defer until someone needs it enough to design it separately.

### SSH execution

Remote command execution is too privileged for `std/net`. If added, it should likely be `std/ssh` or an app/plugin-level module with explicit host-key verification and credential handling. Default-permissive known-host behavior is not acceptable for a stdlib primitive.

### SNMP / network monitoring

SNMP and higher-level monitoring concerns are covered by [DD-047: `std/netmon`](dd-047-std-netmon.md). Keep them out of the initial `std/net` module. `std/net` should provide safe primitives; `std/netmon` can build SNMP/device telemetry, interface counters, topology hints, composite checks, and alert-state helpers on top.

SNMP is a real network-monitoring need, but it is its own protocol family. It should likely start as a private or separately distributed `std/netmon` library rather than default stdlib surface area. Do not make the first `std/net` PR carry BER/ASN.1 and SNMP semantics.

---

## Implementation Record

### Status Dashboard

As of 2026-06-09:

- [x] **PR 1 — `std/net` shell + IPAM helpers + protocol-honest reachability**: merged in [PR #113](https://github.com/ntntlang/ntnt/pull/113).
- [x] **PR 2 — DNS lookup types**: merged in [PR #114](https://github.com/ntntlang/ntnt/pull/114).
- [x] **PR 3 — Bounded port scan**: merged in [PR #115](https://github.com/ntntlang/ntnt/pull/115).
- [x] **PR 4 — TLS certificate inspection**: merged in [PR #117](https://github.com/ntntlang/ntnt/pull/117).
- [x] **PR 5 — Native ICMP sockets**: merged in [PR #119](https://github.com/ntntlang/ntnt/pull/119). Replaced the Linux `ping` subprocess backend with in-tree datagram-first/raw-fallback ICMP sockets via `socket2`. `std/net` now has zero shellouts.
- [x] **PR 6 — Probe substrate + `net_capabilities()`**: typed `ProbeFailure` classification, `src/stdlib/net/` module split (`probe.rs`, `icmp.rs`), and capability detection — groundwork for traceroute.
- [x] **PR 7 — `traceroute()`**: TTL-stepped echo probes on the shared substrate (raw ICMP only, graceful `Err` otherwise), `traceroute` capability flag, Docker `cap_add` deployment docs; see Phase 2 above.
- [x] **PR 8/9 — UDP + TCP traceroute** (`method: "udp"`/`"tcp"`): shared `TraceProbe`/`HopProbe` abstraction, generalized quoted-transport matcher, UDP (Port Unreachable arrival) and raw TCP SYN (SYN-ACK/RST arrival, Linux), plus `traceroute_udp`/`traceroute_tcp` capability flags. See Phase 3.
- [ ] **PR 10 — Parallel hop probing**: send/recv split, concurrent TTL emission, demux by per-hop token, pure assembly. See Phase 3.
- [ ] **PR 11 — `probes_per_hop`**: K probes per hop on PR 10's token map. See Phase 3.
- [x] ~~**mtr-style cycle statistics**~~ — moved to application level (composition over the primitive); not stdlib scope. See "The stdlib/app boundary".
- [x] ~~**Per-hop reverse DNS**~~ — moved to application level (`dns_reverse(hop.from)` over the result); not stdlib scope.
- [ ] **Future primitives** (candidates): `path_mtu(host)` PMTU discovery, source/interface binding, DSCP marking. See Phase 3.
- [x] **Superseded PRs**: [PR #116](https://github.com/ntntlang/ntnt/pull/116) was closed in favor of the cleaner PR #117 branch; [PR #118](https://github.com/ntntlang/ntnt/pull/118) was closed in favor of the cleaner PR #119 branch.

The DD-046 initial scope is complete and Phase 2 is underway. The merged implementation includes runtime registration, typechecker signatures, generated stdlib docs, AI guide coverage, deterministic examples, CI-safe tests, public-network smoke tests gated behind environment variables, and review hardening for target policy, bounded scans, and TLS validation behavior.

### PR 1 — `std/net` shell + IPAM helpers + protocol-honest reachability

Status: **merged in PR #113.**

Scope:

- [x] `src/stdlib/net.rs`
- [x] `src/stdlib/mod.rs`
- [x] `src/typechecker.rs`
- [x] unit tests for helpers and IPv4/IPv6 IPAM behavior
- [x] `ip_parse`, `subnet_contains`, `subnet_overlaps`, `subnet_split`, `subnet_supernet`, `subnet_summarize`, `ip_range_to_cidrs`
- [x] `ping(host, opts?)` with strict no-implicit-TCP-fallback semantics
- [x] `tcp_connect(host, port, opts?)` for explicit TCP port probes
- [x] `reachable(host, opts?)` for explicit high-level ICMP-then-TCP fallback semantics using caller-provided `tcp_ports`
- [x] shared target safety checks used by `tcp_connect`, `reachable`, `port_scan`, and `tls_info`
- [x] generated docs for all Phase 1 functions
- [x] AI guide coverage for `ping`, `tcp_connect`, `reachable`, and private-target opt-in
- [x] IPAM examples for IPv4 subnet splitting and IPv6 parsing/summarization

Acceptance:

- [x] Phase 1 imports work at runtime and lint/typecheck time.
- [x] IPv6 parsing, containment, overlap, splitting, supernet, summarization, and range conversion are supported and tested.
- [x] Large IPv6 counts/results do not overflow or generate unbounded arrays.
- [x] `ping("example.com")` has documented protocol-honest failure when ICMP support is unavailable.
- [x] Missing ICMP capability does not silently fall back to TCP; explicit TCP reachability uses `tcp_connect()` or `reachable(..., map { "tcp_ports": [...] })`.
- [x] local open/closed TCP port tests pass
- [x] private/loopback policy behavior is explicit and tested
- [x] `Err` vs `Ok(map { "connected": false })` semantics are documented and tested
- [x] no public internet dependency
- [x] no new dependency unless clearly justified; any ICMP dependency must preserve no-implicit-TCP-fallback semantics
- [x] `cargo build --profile dev-release`, `cargo test`, docs generation, example validation pass

### PR 2 — DNS lookup types

Status: **merged in PR #114.**

Scope:

- [x] broad supported record-type set for data-bearing DNS lookup records
- [x] `dns_reverse`
- [x] `hickory-resolver` dependency decision
- [x] deterministic parser/result-shaping tests; external DNS smoke coverage gated behind env var

Acceptance:

- [x] deterministic CI tests
- [x] no public DNS dependency by default
- [x] operational/meta records (`ANY`, `AXFR`, `IXFR`, `OPT`, `TSIG`, `ZERO`) rejected rather than treated as ordinary lookup records

### PR 3 — Bounded port scan

Status: **merged in PR #115.**

Scope:

- [x] `port_scan` over explicit port arrays
- [x] bounds and bounded-concurrency batches
- [x] local open/closed test fixture

Acceptance:

- [x] rejects too many ports/concurrency/invalid ports
- [x] deterministic order
- [x] no unbounded scanning

### PR 4 — TLS info

Status: **merged in PR #117.**

Scope:

- [x] Rustls-based TLS connection and certificate validation
- [x] `tls_info`
- [x] local TLS test server with deterministic self-signed certificate
- [x] generated docs and examples
- [x] SNI via `server_name` option with host default
- [x] certificate metadata is returned even when validation fails
- [x] validation uses the observed handshake certificate chain rather than a second probe
- [x] `timeout_ms` is enforced as an overall connection/handshake budget across resolved addresses
- [x] private-target policy requires both process-level and per-call opt-in

Acceptance:

- [x] returns certificate details for valid and validation-failing certs
- [x] reports subject/common name, issuer/common name, not-before/not-after, days left, serial, SANs, protocol, cipher, remote/local address, and chain length
- [x] no dependency claim mismatch
- [x] no public internet dependency by default
- [x] local CI fixture accepts exactly the intended metadata connection
- [x] TLS example validates/lints/runs with public-network behavior gated behind `NTNT_NET_TLS_EXAMPLES=1`

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

## Decisions Made

1. Private/internal targets are governed by `NTNT_NET_ALLOW_PRIVATE=1` plus per-call `allow_private: true`. The std/net knob is intentionally separate and auditable for monitoring-style deployments.

2. `tcp_connect` returns `Ok(map { "connected": false, ... })` for ordinary refused/timeout outcomes. Invalid input and policy denial remain `Err`.

3. `dns_reverse` returns `Result<Array<String>, String>` so multiple PTR answers are preserved and no-answer is `Ok([])`.

4. `port_scan` accepts explicit integer arrays in stdlib. Range parsing remains an app-layer/UI concern unless a future language-level range contract is designed.

5. `tls_info` uses Rustls for the TLS connection and validation, x509-parser for metadata extraction, and webpki-roots for root trust.

6. `ping()` is strict/protocol-honest ICMP by default. It does not fall back to TCP automatically. Developers use `tcp_connect(host, port, opts?)` for explicit TCP checks or `reachable(host, map { "tcp_ports": [...] })` for high-level reachability with explicit TCP fallback.

7. Internal targets are easy to enable for monitoring, but only as an explicit deployment choice: `NTNT_NET_ALLOW_PRIVATE=1` plus per-call `allow_private: true`.

---

## Bottom Line

The refined `std/net` path shipped as four reviewable PRs:

1. deterministic IP helpers plus protocol-honest `ping()`
2. dedicated TCP and high-level reachability probes with explicit safety policy
3. DNS lookup/reverse lookup with CI-safe tests
4. bounded port scan and TLS certificate inspection

That gives ntnt real network-diagnostic capability without making users trip over `CAP_NET_RAW`, and without smuggling in traceroute, SSH, SNMP, broad scanners, public-network CI flakes, or SSRF footguns in one heroic PR. Heroic PRs are where bugs go to get tenure.

---

## PR #113 Cleanup Note

The PR cleanup deliberately split “can I ICMP ping this host?” from “can I reach the service I care about?” instead of hiding TCP connection attempts behind a function named `ping()`.

What changed:

1. `ping(host, opts?)` stays protocol-honest: ICMP only, with a clear error when ICMP is unavailable.
2. TCP probing is explicit via `tcp_connect(host, port, opts?)`, so the caller must name the port and own the semantics.
3. High-level reachability uses `reachable(host, opts?)`, which probes ICMP plus TCP 80/443 by default and accepts optional extra `tcp_ports`, so fallback behavior is useful but not surprising.
4. The safety model stays layered: per-call `allow_private: true` plus process-level `NTNT_NET_ALLOW_PRIVATE=1` for private/internal targets; special-purpose targets remain denied.
5. The typechecker, generated stdlib reference, agent guide, examples, and tests were updated to document and enforce the split.

Net result: PR #113 keeps the operational utility Josh wanted, but avoids calling TCP service checks “ping.” Tiny naming hill, worth defending.
