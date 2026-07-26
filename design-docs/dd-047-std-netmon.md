# DD-047: `std/netmon` — Network Monitoring Toolkit

**Status:** In progress — bundled standard-library module
**Author:** Larri
**Created:** 2026-06-02
**Updated:** 2026-07-26
**Related:** [DD-046: `std/net`](dd-046-std-net.md)
**Target baseline:** post-v0.5.1; first public surface targets v0.5.2

---

## Summary

Design and incrementally ship a bundled `std/netmon` module for building real network monitoring systems in ntnt: bounded SNMP device telemetry first, followed by interface counters, topology hints, composite checks, and carefully scoped monitoring data helpers.

The packaging decision is now settled: `std/netmon` is a standard-library module. That makes its contracts compatibility-sensitive, so each protocol and data-shape slice must be independently useful, bounded, and fixture-tested before the next layer lands.

DD-046 gives ntnt safe network primitives. DD-047 is where those primitives become an MSP/ISP-grade monitoring toolkit instead of a polite loop that asks routers whether they are alive and then shrugs.

---

## Relationship to DD-046

DD-046 owns the low-level, generally safe primitives:

- IP/CIDR/IPAM helpers
- `ping()` with protocol-honest ICMP behavior
- `reachable()` for explicit high-level reachability fallback
- `tcp_connect()`
- DNS lookup/reverse lookup
- bounded port scanning
- TLS certificate inspection
- one-shot traceroute (`icmp`/`udp`/`tcp`)

DD-047 owns higher-level monitoring concerns:

- SNMP polling/walking
- interface telemetry and counter normalization
- device inventory and profile helpers
- topology hints from LLDP/CDP/SNMP tables
- check definitions that combine DD-046 primitives
- alert-state transitions, flap suppression, and maintenance windows
- storage-friendly metric/event shapes

`std/netmon` should consume `std/net`; it should not duplicate low-level connection, DNS, IPAM, or TLS logic.

---

## Product Positioning

`std/netmon` ships as a bundled, explicitly imported standard-library module.

That decision does **not** make it a monitoring product framework. The stdlib owns reusable protocol, normalization, policy, and bounded-computation primitives. Applications continue to own device inventories, tenants, schedules, incidents, notification delivery, persistence schemas, and product-specific state machines.

The implementation is intentionally sliced:

1. secure, bounded protocol primitives with stable normalized output;
2. reusable interface/inventory normalization;
3. composition over `std/net` and `std/jobs` without a new scheduler;
4. only broadly reusable state helpers whose contracts survive real applications.

APIs remain additive. Experimental breadth belongs in applications or extension libraries until its contract is ready for the standard-library compatibility promise.

Future app-local libraries may add vendor adapters or product abstractions without expanding the bundled module. Promotion to prelude remains explicitly out of scope.

### Strategic alignment with DD-077

The correctness-primitives roadmap is broader than this module, but five parts apply now:

- protocol calls are bounded operations with strict option maps and one global deadline;
- target authorization is connection-bound: Slice 1 accepts literal IP addresses and connects transport only to the policy-checked `SocketAddr`;
- credentials are opaque `Secret` capabilities exposed only at the protocol sink, with credential-bearing application buffers zeroized after use;
- normalized varbinds use storage-friendly shapes and preserve values such as `Counter64` without truncation;
- scheduling, durable retry state, incidents, notifications, and tenant policy remain application concerns.

DD-077's future general `NetCapability` object is not invented locally here. `std/netmon` reuses DD-046's deployed network policy until that cross-cutting capability boundary is designed and shipped.

## Design Principles

1. **Operationally useful before academically complete.** Start with the checks and telemetry a small MSP actually needs: device up/down, interface status, bandwidth/error counters, TLS/DNS/TCP checks, and alert state.

2. **Private/internal by default.** Monitoring tools intentionally touch private networks. `std/netmon` should assume explicit deployment intent and should not weaken DD-046's public-web safety posture.

3. **Bounded polling, no surprise scanners.** Inventory/discovery APIs must require explicit targets/subnets, strict caps, and clear opt-ins. A monitoring library should not accidentally become a LAN Roomba with a clipboard.

4. **SNMP is protocol-specific enough to deserve its own layer.** Keep SNMP parsing, OID helpers, and device-profile logic out of `std/net`.

5. **Normalize the boring parts.** Return consistent maps for counters, gauges, status, latency, loss, and timestamps so app code can store and display metrics without knowing every vendor's charming personal problems.

6. **Make failure states first-class.** Timeouts, auth failures, noSuchName/noSuchObject, counter wraps, stale data, and partial polls are monitoring data, not just errors.

7. **Credentials are opaque capabilities.** Community strings, SNMPv3 credentials, webhook tokens, and device credentials enter protocol sinks as `Secret` values from `std/secrets`. Public APIs do not accept plaintext compatibility fallbacks.

8. **Prefer jobs over request-path polling.** Heavy polling belongs in `std/jobs` / worker loops, not HTTP route handlers.

---

## Security and Deployment Model

`std/netmon` is explicitly imported and each poll names one bounded target and OID set. Every protocol call also requires the process-level `NTNT_NETMON_ENABLE=1` gate. Slice 1 reuses DD-046's target policy rather than weakening or duplicating address classification.

Private/internal targets require both deployment and call-site intent:

```bash
NTNT_NETMON_ENABLE=1 NTNT_NET_ALLOW_PRIVATE=1 ntnt run monitor.tnt
```

`NTNT_NETMON_ENABLE=1` is required for public and private calls. Private calls must also pass `allow_private: true` and set `NTNT_NET_ALLOW_PRIVATE=1`.

Policy:

- Public targets are allowed only after the netmon process gate is enabled.
- Private, loopback, link-local, and unique-local targets require the dual opt-in.
- Metadata, multicast, broadcast, unspecified, and documentation targets remain denied even with opt-in.
- Slice 1 accepts literal IPv4/IPv6 addresses only. Hostname resolution is deferred until one outer deadline can bound all resolver candidates and A/AAAA activity.
- UDP transport is connected directly to the checked `SocketAddr`; there is no second resolution step.
- OID lists, encoded requests (8 KiB), response datagrams (16 KiB), timeouts, retries, and normalized results are bounded.
- SNMP communities must be opaque `Secret` values. Validation and transport errors never render them, and credential-bearing request/response buffers are zeroized after use.
- The strict ntnt-owned codec verifies response version, community, request ID, PDU type, complete BER consumption, exact varbind count, and requested OID order.
- `Secret` handling does not make SNMPv2c confidential on the wire; deploy it only on trusted management networks or protected tunnels.
- Default examples perform no traffic; protocol tests use a deterministic localhost mock agent.

A future deployment-issued `NetCapability` can replace process-global authority once that cross-cutting contract ships. DD-047 does not invent a monitoring-only capability system first.

---

## Proposed Module: `std/netmon`

### Future configuration

#### `netmon_configure(opts) -> Result<Map, String>` *(deferred)*

A process-global monitoring configuration remains a possible future convenience for multi-poll applications. Slice 1 deliberately keeps policy and bounds explicit per call; this helper must not become an authority bypass.

```ntnt
import { netmon_configure } from "std/netmon"

netmon_configure(map {
    "poll_timeout_ms": 2000,
    "max_targets_per_run": 256,
    "max_ports_per_target": 64,
    "allow_discovery": false,
    "snmp_defaults": map {
        "version": "2c",
        "timeout_ms": 1500,
        "retries": 1
    }
})
```

Return effective config with clamps applied. Do not expose secrets in the returned map.

### Device model helpers

#### `device(target, opts?) -> Map`

Build a normalized device descriptor used by other helpers.

```ntnt
let router = device("10.0.50.1", map {
    "name": "core-router",
    "role": "router",
    "site": "home-lab",
    "vendor": "mikrotik",
    "snmp": map { "credential": "CORE_ROUTER_SNMP_COMMUNITY" }
})
```

Recommended fields:

- `id`
- `name`
- `target`
- `role`
- `site`
- `vendor`
- `tags`
- `snmp` credential reference metadata only, not raw secrets

### SNMP primitives

#### `snmp_get(target, auth, oids, opts?) -> Result<Map, String>` *(Slice 1)*

Read one bounded set of numeric OIDs from a literal IPv4/IPv6 SNMP agent target. Slice 1 supports SNMPv2c only and requires `NTNT_NETMON_ENABLE=1`.

```ntnt
import { require_secret } from "std/secrets"
import { snmp_get } from "std/netmon"

let auth = map {
    "version": "2c",
    "community": require_secret("ROUTER_SNMP_COMMUNITY")
}
let system = snmp_get(
    "10.0.50.1",
    auth,
    ["1.3.6.1.2.1.1.1.0", "1.3.6.1.2.1.1.5.0"],
    map { "allow_private": true }
)
```

The auth map is strict: `version` must be `"2c"`, `community` must be an opaque `Secret`, and unknown fields fail closed. The options map accepts only:

- `port`: default 161, range 1–65535
- `timeout_ms`: one global request-encoding, UDP send/receive, and retry budget; default 2000, range 50–30000
- `retries`: additional bounded attempts, default 0, maximum 3
- `allow_private`: default false; still requires `NTNT_NET_ALLOW_PRIVATE=1`

The OID array accepts 1–64 unique numeric OIDs, each at most 255 bytes and 128 unsigned 32-bit arcs. Named MIB syntax is deferred. Encoded requests are capped at 8 KiB and response datagrams at 16 KiB. Explicit values outside any bound fail rather than clamp.

Result shape:

```ntnt
Ok(map {
    "target": "10.0.50.1",
    "address": "10.0.50.1",
    "port": 161,
    "version": "2c",
    "duration_ms": 12,
    "attempts": 1,
    "values": [
        map {
            "oid": "1.3.6.1.2.1.1.1.0",
            "type": "octet_string",
            "encoding": "utf8",
            "value": "RouterOS"
        }
    ]
})
```

`Counter64` values use decimal strings so ntnt's signed 64-bit `Int` cannot truncate them, including legal values above `i64::MAX`. Binary octet strings and opaque values use lowercase hex with explicit encoding metadata. Protocol exceptions such as `no_such_object` retain their type and use `None` as the value. Agent `error_status`/`error_index`, transport timeouts, malformed BER, and authentication mismatches return `Err(String)` rather than partial telemetry.

#### `snmp_walk(target, auth, oid, opts?) -> Result<Array<Map>, String>` *(next slice)*

Walk a subtree with strict row/result caps. This follows GET rather than sharing its first compatibility commit.

Planned options add `max_results` with a conservative default and hard cap. WALK reuses the same strict auth contract, global timeout budget, checked-address transport binding, and normalized varbind shapes.

#### `snmp_bulk_walk(target, auth, oid, opts?) -> Result<Array<Map>, String>`

Optional optimization after basic walk is stable. SNMP GETBULK can reduce polling overhead but should not be in PR 1 unless the implementation stays small and testable.

#### `snmp_capabilities(target, auth, opts?) -> Result<Map, String>`

Probe SNMP availability and supported basics without doing a full inventory poll.

```ntnt
snmp_capabilities("10.0.50.1", auth)
// Ok(map { "reachable": true, "version": "2c", "sys_object_id": "...", "vendor_hint": "mikrotik" })
```

### Interface telemetry

#### `interface_list(target, opts?) -> Result<Array<Map>, String>`

Return normalized interface inventory from IF-MIB.

Recommended fields:

- `index`
- `name`
- `description`
- `alias`
- `type`
- `mtu`
- `speed_bps`
- `admin_status`
- `oper_status`
- `mac`
- `last_change`

#### `interface_counters(target, opts?) -> Result<Array<Map>, String>`

Return 64-bit counters where available, falling back to 32-bit with explicit metadata.

Recommended fields:

- `index`
- `name`
- `in_octets`
- `out_octets`
- `in_packets`
- `out_packets`
- `in_errors`
- `out_errors`
- `in_discards`
- `out_discards`
- `counter_bits`
- `timestamp`

#### `interface_rates(previous, current, opts?) -> Result<Array<Map>, String>`

Convert counter snapshots into rates while handling wraps, resets, and impossible deltas.

Recommended fields:

- `index`
- `interval_seconds`
- `in_bps`
- `out_bps`
- `in_pps`
- `out_pps`
- `error_rate`
- `discard_rate`
- `utilization_in_percent`
- `utilization_out_percent`
- `counter_reset_detected`

This helper is important enough to design early. Good monitoring lives on rates, not raw counters. Raw counters are just odometers with commitment issues.

### Device inventory

#### `device_identity(target, opts?) -> Result<Map, String>`

Read basic identity:

- `sys_name`
- `sys_descr`
- `sys_object_id`
- `sys_contact`
- `sys_location`
- `uptime_seconds`
- `vendor_hint`
- `model_hint`
- `os_hint`

#### `device_inventory(target, opts?) -> Result<Map, String>`

Higher-level inventory bundle:

```ntnt
Ok(map {
    "identity": map { ... },
    "interfaces": [...],
    "neighbors": [...],
    "routes_summary": map { ... },
    "poll_status": "partial",
    "warnings": ["LLDP table unavailable"]
})
```

V1 should tolerate partial results. A device that refuses LLDP should not discard interface counters.

### Topology hints

#### `lldp_neighbors(target, opts?) -> Result<Array<Map>, String>`

Read LLDP-MIB where available.

#### `cdp_neighbors(target, opts?) -> Result<Array<Map>, String>`

Optional Cisco CDP support. Candidate for vendor-profile phase, not initial core.

#### `topology_edges(devices, opts?) -> Result<Array<Map>, String>`

Normalize neighbor data into graph edges.

Fields:

- `local_device`
- `local_interface`
- `remote_device`
- `remote_interface`
- `protocol`
- `confidence`
- `raw`

### Composite checks

Composite checks combine `std/net` and `std/netmon` primitives into reusable monitoring definitions.

#### `check_ping(target, opts?) -> Result<Map, String>`

Wrapper over DD-046 `ping()` with monitoring result shape.

#### `check_tcp(target, port, opts?) -> Result<Map, String>`

Wrapper over DD-046 `tcp_connect()`.

#### `check_dns(name, record_type?, opts?) -> Result<Map, String>`

Wrapper over DD-046 DNS helpers with latency and expected-record matching.

#### `check_tls(host, opts?) -> Result<Map, String>`

Wrapper over DD-046 `tls_info()` with expiry thresholds.

#### `check_snmp(target, opts?) -> Result<Map, String>`

SNMP liveness/identity check.

#### `run_checks(targets, checks, opts?) -> Result<Array<Map>, String>`

Run bounded checks for multiple targets. This should be job-friendly and deterministic, not an unbounded fan-out.

Result shape:

```ntnt
Ok([
    map {
        "target": "core-router",
        "check": "snmp",
        "status": "ok",
        "latency_ms": 18.2,
        "observed_at": "2026-06-02T12:00:00Z",
        "data": map { ... }
    }
])
```

### Alert-state helpers

`std/netmon` should not own delivery channels, but it can own state transitions.

#### `evaluate_threshold(metric, rule, window?) -> Result<Map, String>`

Examples:

- interface utilization over 90% for 5 minutes
- packet loss over 10% for 3 checks
- TLS expiry below 14 days
- device down for 2 consecutive polls

#### `alert_transition(previous_state, current_result, opts?) -> Result<Map, String>`

Return `open`, `resolved`, `unchanged`, `suppressed`, or `flapping`.

Fields:

- `state`
- `severity`
- `reason`
- `dedupe_key`
- `opened_at`
- `resolved_at`
- `last_observed_at`
- `suppressed_until`

#### `maintenance_window(target, schedule, now?) -> Result<Map, String>`

Small helper for suppressing expected outages. Full calendar integration belongs elsewhere.

---

## Data Model Recommendations

A reference monitoring app should store these durable shapes:

### `devices`

- `id`
- `name`
- `target`
- `site`
- `role`
- `vendor`
- `tags`
- credential references, not raw secrets
- enabled/disabled state

### `checks`

- `id`
- `device_id`
- `type`
- `options`
- `interval_seconds`
- `timeout_ms`
- enabled/disabled state

### `metric_samples`

- `device_id`
- `interface_index` optional
- `metric`
- `value`
- `unit`
- `observed_at`
- `source`

### `check_results`

- `check_id`
- `status`
- `latency_ms`
- `observed_at`
- `data`
- `error`

### `alerts`

- `dedupe_key`
- `state`
- `severity`
- `target`
- `reason`
- `opened_at`
- `resolved_at`
- `last_observed_at`

Retention, rollups, dashboards, and notification delivery should live in the reference app, not necessarily inside `std/netmon`.

---

## Module Boundary: What Belongs Where

### `std/net` / DD-046

- IPAM math
- TCP connect probe
- ping/reachability
- DNS lookup/reverse
- bounded port scan
- TLS inspection
- one-shot traceroute

### `std/netmon` / DD-047

- SNMP device telemetry
- interface counters/rates
- topology hints
- device inventory normalization
- composite monitoring checks
- alert-state helpers
- storage-friendly metric/check shapes

### Reference app, not library

- UI/dashboard
- auth/users/teams
- notification delivery integrations
- long-term retention policies
- chart rendering
- topology graph layout
- customer/site management
- billing/multi-tenant concerns

### Future raw/network toolbox, not initial `std/netmon`

- MTR-style repeated traceroute statistics / hop enrichment
- ARP scan
- packet capture
- bandwidth testing
- route-table reads
- BGP session tooling
- NetFlow/sFlow collectors

Some of these may become useful later, but they carry OS permissions, abuse risk, and platform-specific pain. Do not let them sneak into SNMP v1 wearing a fake mustache.

---

## Implementation Plan

### Status Dashboard

- [x] **Slice 0 — standard-library packaging and security contract**
- [x] **Slice 1A — bounded SNMPv2c GET**
- [ ] **Slice 1B — bounded SNMP WALK**
- [ ] **PR 2 — interface inventory and counters**
- [ ] **PR 3 — counter-rate normalization**
- [ ] **PR 4 — device identity and inventory bundle**
- [ ] **PR 5 — topology hints from LLDP**
- [ ] **PR 6 — composite checks over DD-046 primitives**
- [ ] **PR 7 — broadly reusable alert-state helpers, if proven**
- [ ] **PR 8 — reference monitoring app / examples**

### Slice 0 — Standard-Library Packaging and Security Contract

Shipped with Slice 1A:

- [x] Package `std/netmon` as an explicitly imported bundled stdlib module.
- [x] Preserve DD-046's low-level boundary and share its outbound target policy.
- [x] Use `std/secrets` opaque `Secret` values at credential-bearing sinks.
- [x] Require `NTNT_NETMON_ENABLE=1` before any protocol call.
- [x] Reject plaintext community strings and unknown auth/option keys.
- [x] Accept literal IP targets and connect protocol transport directly to the checked address.
- [x] Add a deterministic localhost mock-agent fixture.
- [x] Keep scheduling, storage, incidents, tenants, and notification delivery in applications.

Acceptance:

- [x] `std/netmon` is importable through the normal module registry.
- [x] No raw credentials appear in examples, generated docs, stdout, stderr, or errors.
- [x] Private targets retain the deployment-plus-call-site opt-in.
- [x] Special-purpose targets remain denied.
- [x] Public API and typechecker signatures agree.

### Slice 1A — SNMPv2c GET

Scope:

- [x] `snmp_get(target, auth, oids, opts?)`
- [x] Small ntnt-owned SNMPv2c GET codec with complete BER validation and zeroizing credential buffers.
- [x] Strict numeric OID parsing, canonicalization, deduplication, and a 64-OID cap.
- [x] Stable value normalization, including lossless `Counter64` and binary octets.
- [x] Global timeout budget, retry cap, request/response byte caps, and exact response OID/count checks.
- [x] Mock UDP SNMP agent fixture exercising the ntnt runtime end to end.
- [x] Independent golden BER coverage for BOOLEAN, full-width Counter64, wrong version/PDU/request ID/community, malformed lengths, truncation, and trailing bytes.

Acceptance:

- [x] GET returns normalized ordered varbinds for a bounded OID set.
- [x] Protocol exceptions retain explicit types rather than becoming fabricated values.
- [x] Timeouts, policy failures, malformed responses, and agent errors return `Err(String)`.
- [x] Tests require no real network device.
- [x] Community material enters only as `Secret`.

### Slice 1B — SNMP WALK

Scope:

- [ ] `snmp_walk(target, auth, oid, opts?)`
- [ ] Strict subtree and result-count enforcement.
- [ ] GETNEXT first; GETBULK only after equivalent fixture coverage.
- [ ] Loop, out-of-subtree, malformed-order, and premature-end detection.
- [ ] Reuse Slice 1A auth, policy, timeout, and normalization contracts.

### PR 2 — Interface Inventory and Counters

Scope:

- [ ] `interface_list(target, opts?)`
- [ ] `interface_counters(target, opts?)`
- [ ] IF-MIB OID constants/helpers.
- [ ] Prefer high-capacity 64-bit counters when present.
- [ ] Explicit fallback metadata for 32-bit counters.

Acceptance:

- [ ] Interfaces have stable normalized fields.
- [ ] Counters include timestamp and counter width.
- [ ] Missing optional fields degrade gracefully.
- [ ] Fixture covers up/down/admin-down interfaces.

### PR 3 — Counter-Rate Normalization

Scope:

- [ ] `interface_rates(previous, current, opts?)`
- [ ] Counter wrap/reset detection.
- [ ] Utilization percentage from speed when available.
- [ ] Invalid delta filtering.

Acceptance:

- [ ] 64-bit and 32-bit counters calculate sane rates.
- [ ] Wraps/resets are detected and marked.
- [ ] Negative/impossible deltas do not produce bogus traffic spikes.
- [ ] Tests cover missing speed and zero interval.

### PR 4 — Device Identity and Inventory Bundle

Scope:

- [ ] `device_identity(target, opts?)`
- [ ] `device_inventory(target, opts?)`
- [ ] sysDescr/sysObjectID/vendor-hint mapping.
- [ ] Partial-result warnings.

Acceptance:

- [ ] Identity includes name, description, object ID, location/contact, uptime, vendor hint.
- [ ] Inventory bundle returns partial data rather than failing the whole poll when optional tables are unavailable.
- [ ] Vendor-hint mapping is data-driven and easy to extend.

### PR 5 — Topology Hints from LLDP

Scope:

- [ ] `lldp_neighbors(target, opts?)`
- [ ] `topology_edges(devices, opts?)`
- [ ] Normalize neighbor identities and local/remote port names.
- [ ] Optional CDP design stub, but do not implement unless tiny.

Acceptance:

- [ ] LLDP neighbor table becomes stable edge maps.
- [ ] Missing LLDP support returns clear empty/unsupported result.
- [ ] Edges include confidence/protocol/source fields.

### PR 6 — Composite Checks Over DD-046 Primitives

Scope:

- [ ] `check_ping`
- [ ] `check_tcp`
- [ ] `check_dns`
- [ ] `check_tls`
- [ ] `check_snmp`
- [ ] `run_checks`

Acceptance:

- [ ] Check result shape is consistent across check types.
- [ ] `run_checks` enforces target/check/concurrency caps.
- [ ] DD-046 helpers are reused, not reimplemented.
- [ ] Results are storage-friendly and include observed timestamp.

### PR 7 — Alert-State Helpers

Scope:

- [ ] `evaluate_threshold(metric, rule, window?)`
- [ ] `alert_transition(previous_state, current_result, opts?)`
- [ ] Maintenance-window helper.
- [ ] Flap detection and dedupe-key conventions.

Acceptance:

- [ ] Down/up/open/resolved transitions are deterministic.
- [ ] Dedupe keys are stable.
- [ ] Maintenance windows suppress without losing raw check results.
- [ ] Flapping state is explicit and test-covered.

### PR 8 — Reference Monitoring App / Examples

Scope:

- [ ] Example ntnt app that polls devices through jobs.
- [ ] Storage schema for devices/checks/results/alerts.
- [ ] Small dashboard route examples.
- [ ] Mock-device mode for examples and CI.

Acceptance:

- [ ] Example works without real devices by default.
- [ ] Real-device usage is documented with env vars and private-network opt-ins.
- [ ] Polling runs in workers/jobs, not request handlers.
- [ ] Alerts are stateful and deduped.

---

## Dependencies / Implementation Choices

Slice 1A uses a small ntnt-owned SNMPv2c GET codec instead of a general SNMP dependency:

- only definite-length BER forms required for strict SNMPv2c GET/RESPONSE are accepted;
- the complete outer message, PDU, varbind list, and each varbind must be consumed with no trailing or silently truncated fields;
- version, community, request ID, response PDU type, agent status, varbind count, and exact OID order are verified;
- legal unsigned `Counter64` values through `u64::MAX` and correctly tagged BOOLEAN values decode losslessly;
- request and receive buffers that contain the community are wrapped in `Zeroizing` and cleared on every exit path;
- no Tokio, OpenSSL, SNMPv3 crypto, MIB loading, or trap dependency is pulled in.

SNMPv3 remains deferred until the v2c data shapes and failure contract survive real use. WALK/GETBULK must reuse this boundary rather than exposing the dependency directly.

Supporting test infrastructure:

- deterministic localhost UDP mock agent in default CI;
- hand-authored golden BER packets independent of the production codec;
- fixed clocks when rate and alert helpers arrive;
- optional real-device tests behind explicit environment gates.

---

## Testing Strategy

Default CI must not require private network access or real devices.

Required tests:

- [x] mock SNMP GET scalar success
- [ ] mock SNMP WALK table success
- [x] timeout / retry behavior
- [x] malformed or mismatched SNMP response handling
- [x] complete BER consumption, version/PDU/request/community correlation, and full-width Counter64 decoding
- [x] noSuchObject/noSuchName normalization
- [ ] interface inventory normalization
- [ ] 32-bit and 64-bit counter snapshots
- [ ] counter wrap/reset detection
- [ ] alert-state transitions
- [ ] maintenance-window suppression
- [ ] `run_checks` concurrency/cap enforcement

Optional/manual tests:

- [ ] real SNMP v2c device smoke test gated by `NTNT_RUN_NETMON_DEVICE_TESTS=1`
- [ ] vendor profile smoke tests gated by explicit target/env vars
- [ ] long-running polling soak test outside default CI

---

## Open Decisions

1. **Packaging — resolved:** bundled, explicitly imported standard-library module. Product-specific monitoring state remains application-owned.

2. **SNMP dependency — resolved for v2c GET:** use the small ntnt-owned strict codec. The previously evaluated `snmp2` path could not prove complete BER consumption, full-width Counter64 decoding, or zeroized credential buffers.

3. **SNMPv3 timing:** include in initial module or defer?

   Recommendation: defer. v2c still dominates small/internal monitoring; v3 adds auth/privacy/key-handling complexity that should not block normalized telemetry shapes.

4. **OID/MIB strategy:** ship hardcoded IF-MIB/SYSTEM/LLDP constants or parse MIB files?

   Recommendation: hardcoded curated constants first. MIB parsing is a swamp with paperwork.

5. **Discovery scope:** should `std/netmon` include subnet discovery?

   Recommendation: not initially. Require explicit device inventory first. Add bounded discovery later after policy and abuse controls are proven.

6. **Alert delivery:** should the module send notifications?

   Recommendation: no. It can calculate alert state; apps own email/webhook/Telegram/PagerDuty delivery.

---

## Bottom Line

`std/netmon` is now the standard-library home for bounded monitoring protocols and reusable normalization on top of DD-046's safe network policy.

Start narrow. Keep credentials opaque. Bind transport to checked addresses. Normalize without truncating. Let applications own the monitoring product around those primitives. This gives ntnt a credible SNMP foundation without turning the default stdlib into a closet full of enterprise networking adapters wearing one trench coat.
