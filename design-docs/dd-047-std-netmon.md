# DD-047: `std/netmon` — Network Monitoring Toolkit

**Status:** Draft / private-library candidate
**Author:** Larri
**Created:** 2026-06-02
**Related:** [DD-046: `std/net`](dd-046-std-net.md)
**Target baseline:** after DD-046 Phase 2+ primitives stabilize

---

## Summary

Design a `std/netmon` module for building real network monitoring systems in ntnt: SNMP device telemetry, interface counters, topology hints, check orchestration helpers, alert-state modeling, and opinionated monitoring data shapes.

Unlike DD-046, this is **not necessarily part of the default standard library**. The current recommendation is to treat `std/netmon` as a private or separately distributed library first, then promote stable pieces only if they prove broadly useful and safe enough for the standard distribution.

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

`std/netmon` has three possible packaging modes:

1. **Private library first** — preferred initial path. Iterate fast, support real-world device weirdness, and avoid making unstable SNMP/device contracts part of ntnt's default stdlib promise.
2. **Optional bundled library** — ships with ntnt but is explicitly optional/experimental, similar to a batteries-included contrib module.
3. **Promoted standard module** — only after the API shape has survived real monitoring apps, device diversity, and operational abuse.

Recommendation: start as a private library using the public module path `std/netmon` in examples/design so the eventual promotion path is clean, but do not commit to bundling it in the default standard library yet.

---

## Design Principles

1. **Operationally useful before academically complete.** Start with the checks and telemetry a small MSP actually needs: device up/down, interface status, bandwidth/error counters, TLS/DNS/TCP checks, and alert state.

2. **Private/internal by default.** Monitoring tools intentionally touch private networks. `std/netmon` should assume explicit deployment intent and should not weaken DD-046's public-web safety posture.

3. **Bounded polling, no surprise scanners.** Inventory/discovery APIs must require explicit targets/subnets, strict caps, and clear opt-ins. A monitoring library should not accidentally become a LAN Roomba with a clipboard.

4. **SNMP is protocol-specific enough to deserve its own layer.** Keep SNMP parsing, OID helpers, and device-profile logic out of `std/net`.

5. **Normalize the boring parts.** Return consistent maps for counters, gauges, status, latency, loss, and timestamps so app code can store and display metrics without knowing every vendor's charming personal problems.

6. **Make failure states first-class.** Timeouts, auth failures, noSuchName/noSuchObject, counter wraps, stale data, and partial polls are monitoring data, not just errors.

7. **No secrets in design-level configs.** Community strings, SNMPv3 credentials, webhook tokens, and device credentials must come from env/secret stores, not checked-in examples.

8. **Prefer jobs over request-path polling.** Heavy polling belongs in `std/jobs` / worker loops, not HTTP route handlers.

---

## Security and Deployment Model

`std/netmon` is intentionally for internal monitoring. It still needs guardrails.

Recommended process-level opt-in:

```bash
NTNT_NETMON_ENABLE=1 NTNT_NET_ALLOW_PRIVATE=1 ntnt run monitor.tnt
```

Suggested policy:

- `std/netmon` refuses to poll unless `NTNT_NETMON_ENABLE=1` is set.
- Private targets still require DD-046's private-network opt-in where DD-046 primitives are used.
- Per-call target lists or subnets must be explicit and bounded.
- Discovery/sweep helpers require stricter opt-ins than single-device checks.
- Secrets must be passed as secret references or loaded from env, never embedded in examples.
- Default examples use local/mock agents or documentation-only sample targets.

Open question: whether `std/netmon` should also require an app-level config object like `netmon_configure(...)` before any polling. Recommendation: yes for private-library v1; explicit setup makes accidental usage harder.

---

## Proposed Module: `std/netmon`

### Configuration

#### `netmon_configure(opts) -> Result<Map, String>`

Configure global monitoring behavior for the current process.

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
    "snmp": map { "community_env": "CORE_ROUTER_SNMP_COMMUNITY" }
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

#### `snmp_get(target, oid, opts?) -> Result<Map, String>`

Read one OID from a device.

```ntnt
import { snmp_get } from "std/netmon"

let sys_name = snmp_get("10.0.50.1", "1.3.6.1.2.1.1.5.0", map {
    "version": "2c",
    "community_env": "ROUTER_SNMP_COMMUNITY"
})
```

Result shape:

```ntnt
Ok(map {
    "target": "10.0.50.1",
    "oid": "1.3.6.1.2.1.1.5.0",
    "name": "sysName.0",
    "type": "OctetString",
    "value": "core-router",
    "latency_ms": 12.4,
    "timestamp": "2026-06-02T12:00:00Z"
})
```

#### `snmp_walk(target, oid, opts?) -> Result<Array<Map>, String>`

Walk a subtree with strict row/result caps.

Options:

- `max_results`: default 2048, hard cap 20000 for private-library v1
- `timeout_ms`: default 1500, clamp to shared netmon max
- `retries`: default 1, hard cap 3
- `version`: `"2c"` first; SNMPv3 later
- `community_env`: environment variable containing community string

#### `snmp_bulk_walk(target, oid, opts?) -> Result<Array<Map>, String>`

Optional optimization after basic walk is stable. SNMP GETBULK can reduce polling overhead but should not be in PR 1 unless the implementation stays small and testable.

#### `snmp_capabilities(target, opts?) -> Result<Map, String>`

Probe SNMP availability and supported basics without doing a full inventory poll.

```ntnt
snmp_capabilities("10.0.50.1", map { "community_env": "ROUTER_SNMP_COMMUNITY" })
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

- [ ] **PR 0 — packaging decision and private-library skeleton**
- [ ] **PR 1 — SNMP v2c basics**
- [ ] **PR 2 — interface inventory and counters**
- [ ] **PR 3 — counter-rate normalization**
- [ ] **PR 4 — device identity and inventory bundle**
- [ ] **PR 5 — topology hints from LLDP**
- [ ] **PR 6 — composite checks over DD-046 primitives**
- [ ] **PR 7 — alert-state helpers**
- [ ] **PR 8 — reference monitoring app / examples**

### PR 0 — Packaging Decision and Skeleton

Scope:

- [ ] Decide private library vs optional bundled module for first implementation.
- [ ] Choose module layout and import path strategy for `std/netmon`.
- [ ] Add a small private-library skeleton or experimental module gate.
- [ ] Define credential-reference conventions (`community_env`, later secret refs).
- [ ] Add deterministic mock SNMP fixture plan.
- [ ] Add README/docs caveat that this is not default stdlib until promoted.

Acceptance:

- [ ] Consumers can import the private module in a documented way.
- [ ] No raw secrets appear in examples, tests, or generated docs.
- [ ] `NTNT_NETMON_ENABLE=1` requirement is documented or implemented.
- [ ] DD-046 boundary is preserved.

### PR 1 — SNMP v2c Basics

Scope:

- [ ] `snmp_get(target, oid, opts?)`
- [ ] `snmp_walk(target, oid, opts?)`
- [ ] Basic ASN.1/BER value decoding or dependency choice.
- [ ] SNMP response/error normalization.
- [ ] Timeout and retry clamps.
- [ ] Mock SNMP agent test fixture.

Acceptance:

- [ ] GET returns normalized map for scalar OIDs.
- [ ] WALK returns ordered rows with strict result cap.
- [ ] noSuchObject/noSuchName/timeouts produce monitoring-useful results.
- [ ] Tests do not require a real network device.
- [ ] Community strings are loaded via env/secret references only.

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

SNMP implementation options:

1. Use an existing Rust SNMP crate if it is maintained, dependency-light, supports v2c cleanly, and can be tested with fixtures.
2. Implement minimal SNMP v2c GET/WALK encoding/decoding directly if dependency quality is poor.
3. Defer SNMPv3 until v2c and data shapes are stable.

Recommendation: evaluate crates during PR 0/1, but bias toward a small, auditable implementation path. SNMP v2c is not pretty, but it is finite. Unlike humans in meetings.

Potential helper crates/features:

- ASN.1/BER encoder/decoder if direct implementation wins
- fixed test clock helper for rate and alert tests
- mock UDP SNMP agent fixture for CI

---

## Testing Strategy

Default CI must not require private network access or real devices.

Required tests:

- [ ] mock SNMP GET scalar success
- [ ] mock SNMP WALK table success
- [ ] timeout / retry behavior
- [ ] malformed SNMP response handling
- [ ] noSuchObject/noSuchName normalization
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

1. **Packaging:** private library only, optional bundled module, or experimental stdlib module?

   Recommendation: private library first, preserving `std/netmon` as the intended import path.

2. **SNMP dependency:** external crate or direct minimal v2c implementation?

   Recommendation: decide after crate spike. Prefer boring-maintained over clever-abandoned.

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

`std/netmon` should make ntnt credible for network monitoring by adding SNMP/device telemetry and monitoring-specific state helpers on top of DD-046's safe primitives.

Start private. Keep the API honest. Normalize the data shapes. Avoid pretending a full NMS is a stdlib function. That way we get a useful monitoring toolkit without turning ntnt's default stdlib into a closet full of enterprise networking adapters wearing one trench coat.
