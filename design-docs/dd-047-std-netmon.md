# DD-047: `std/netmon` — Network Monitoring Toolkit

**Status:** In progress — bundled standard-library module
**Author:** Larri
**Created:** 2026-06-02
**Updated:** 2026-07-26
**Related:** [DD-046: `std/net`](dd-046-std-net.md)
**Target baseline:** post-v0.5.1; first public surface targets v0.5.2

---

## Summary

Design and incrementally ship a bundled `std/netmon` module for building real network monitoring systems in ntnt: bounded SNMP device telemetry first, followed by offline-compiled third-party MIB catalogs, data-driven device recognition and inventory plans, interface counters, topology hints, composite checks, and carefully scoped monitoring data helpers.

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
- offline-compiled MIB schemas, device profiles, and finite walk plans
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

That decision does **not** make it a monitoring product framework. The stdlib owns reusable protocol, schema/catalog, normalization, policy, and bounded-computation primitives. Applications continue to own target inventories, tenants, schedules, incidents, notification delivery, persistence schemas, catalog rollout policy, and product-specific state machines.

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

9. **MIB schema is not execution policy.** MIB modules describe names, types, tables, indexes, access, and display semantics. Separate declarative device profiles classify observed identity, and separate finite inventory plans select read-only walks. No layer may contain credentials, targets, callbacks, templates, shell commands, or arbitrary ntnt code.

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
- OID lists, encoded requests (8 KiB), response datagrams (8 KiB), timeouts, retries, and normalized results are bounded.
- SNMP communities must be opaque `Secret` values. Validation and transport errors never render them, and credential-bearing request/response buffers are zeroized after use.
- The strict ntnt-owned codec verifies response version, community, request ID, PDU type, complete BER consumption, exact varbind count, and requested OID order.
- `Secret` handling does not make SNMPv2c confidential on the wire; deploy it only on trusted management networks or protected tunnels.
- Default examples perform no traffic; protocol tests use a deterministic localhost mock agent.

A future deployment-issued `NetCapability` can replace process-global authority once that cross-cutting contract ships. DD-047 does not invent a monitoring-only capability system first.

## Third-Party MIB Catalog Model

Third-party updates are distributed as operator-owned source bundles, but raw ASN.1/SMI text is never parsed on a poll, HTTP request, or ordinary stdlib call. ntnt uses two stages:

1. `ntnt netmon mib compile <source-root> --output-dir <directory>` validates bounded MIB/profile/plan source in an isolated offline compiler worker and emits one immutable content-addressed catalog.
2. Application startup loads only that canonical catalog from the application manifest. Runtime readers clone one immutable `Arc<CatalogSnapshot>` for the complete operation.

The compiler performs no network fetch, system MIB-directory discovery, shell execution, dynamic library loading, template evaluation, or `.tnt` execution. A parser crash, memory-budget failure, or timeout cannot publish a new immutable artifact or change application configuration; previously selected artifacts remain untouched.

### Three separately typed layers

One catalog publishes three independently validated snapshots together:

1. **MIB schemas** — modules, imports, qualified symbols, numeric OIDs, syntax, textual conventions, enum/bit metadata, table/index relationships, access, and provenance.
2. **Device profiles** — advisory exact/prefix `sysObjectID` and bounded secondary identity matchers plus vendor/family/model labels. Recognition never grants network authority or selects credentials.
3. **Walk/inventory plans** — finite read-only roots and an allowlisted normalization DSL compiled to numeric OIDs against the candidate MIB snapshot.

MIB source cannot define recognition or polling behavior. Profiles cannot contain targets, ports, credentials, protocol authority, SNMP SET operations, cap increases, or executable hooks. Plans cannot contain callbacks, general expressions, or data-dependent unbounded expansion.

The catalog is published atomically only after every profile and plan recompiles against the candidate MIB schema. A MIB-only update that invalidates an active profile or plan rejects the complete candidate.

### Source schemas

The source root contains an explicit manifest and separately typed files:

```text
netmon/
├── catalog.toml
├── mibs/
│   ├── ACME-SMI.mib
│   └── ACME-SWITCH-MIB.mib
├── profiles/
│   └── acme-switch.toml
└── plans/
    └── acme-switch-inventory.toml
```

`catalog.toml` v1 is a closed schema:

```toml
schema = 1
id = "acme-network"
version = "2026.07.26"

[[mibs]]
path = "mibs/ACME-SMI.mib"
module = "ACME-SMI"

[[mibs]]
path = "mibs/ACME-SWITCH-MIB.mib"
module = "ACME-SWITCH-MIB"

[[profiles]]
path = "profiles/acme-switch.toml"
id = "acme-switch"

[[plans]]
path = "plans/acme-switch-inventory.toml"
id = "acme-switch-inventory"
```

Every file and expected module/profile/plan identity is explicit; each profile or plan file contains exactly one top-level record. Imports resolve only within the candidate source set plus ntnt's synthetic SMI foundation modules. Absolute paths, parent components, symlinks, duplicate paths, case/Unicode-normalization collisions, unknown fields, duplicate identities, unresolved required imports, and cycles fail closed.

A device profile is classification data only:

```toml
schema = 1
id = "acme-switch"
version = "1"
vendor = "Acme"
family = "Switch"
model = "1000 series"
plans = ["acme-switch-inventory"]
default_plan = "acme-switch-inventory"

[match]
sys_object_ids = ["ACME-SWITCH-MIB::acmeSwitch1000"]
sys_object_id_prefixes = ["1.3.6.1.4.1.424242.10"]

[[match.sys_descr]]
op = "contains"
value = "ACME Switch"
ascii_case_insensitive = true
```

Profile v1 permits only `equals`, `prefix`, and `contains` literal `sysDescr` operators over a compiler-capped 512-byte value. It has no regex, inheritance, includes, priorities, negation, or executable predicates. Recognition precedence is exact `sysObjectID`, longest `sysObjectID` prefix, then `sysDescr`. Multiple profiles matching at the winning tier return runtime `ambiguous`; duplicate exact rules and equal static prefixes are rejected during compilation.

An inventory plan is finite read-only acquisition data:

```toml
schema = 1
id = "acme-switch-inventory"
version = "1"

[[sections]]
id = "interfaces"
kind = "table"
root = "IF-MIB::ifTable"
index = "IF-MIB::ifIndex"
required = true
max_results = 1024

[[sections.fields]]
name = "name"
oid = "IF-MIB::ifName"
format = "string"

[[sections.fields]]
name = "in_octets"
oid = "IF-MIB::ifHCInOctets"
format = "counter"
```

Section v1 permits `scalars` and `table` only. Field formats are the allowlisted values `string`, `integer`, `unsigned`, `counter`, `enum`, `bits`, `mac`, `ipv4`, `ipv6`, `oid`, and `hex`. Every symbol is module-qualified and compiles to a numeric OID. V1 has no joins, callbacks, general expressions, templates, includes, inheritance, scripts, targets, credentials, SNMP SET, or data-dependent expansion.

All source objects use closed TOML schemas; unknown or duplicate keys are errors. IDs are 1–128-byte ASCII values matching `[A-Za-z0-9][A-Za-z0-9._-]*`; versions are 1–128-byte strings; paths are 1–512-byte normalized relative paths. Additional normative fields and constraints are:

| Record | Required fields | Optional fields | Cross-field constraints |
|---|---|---|---|
| Manifest | `schema=1`, `id`, `version`, `mibs`, `profiles`, `plans` | none | `mibs` has 1–128 unique `(module,path)` entries; `profiles` and `plans` each have 1–256 entries; their combined source-file count is at most 512; profile/plan entry IDs and paths are unique and match the referenced file's top-level ID |
| MIB entry | `path`, `module` | none | `module` follows SMI module-identifier syntax and exactly matches parsed module identity |
| Profile/plan entry | `path`, `id` | none | referenced file contains exactly one record with that ID |
| Profile | `schema=1`, `id`, `version`, `vendor`, `family`, `plans`, `default_plan`, `match` | `model` | `plans` has 1–32 unique IDs; `default_plan` is a member of `plans`; at least one matcher exists |
| Match set | `sys_object_ids`, `sys_object_id_prefixes`, `sys_descr` | none | arrays may individually be empty; OID entries are unique, numeric or module-qualified, and compile to at most 128 arcs |
| `sys_descr` matcher | `op`, `value`, `ascii_case_insensitive` | none | `op` is `equals`, `prefix`, or `contains`; `value` is 1–512 bytes |
| Plan | `schema=1`, `id`, `version`, `sections` | none | 1–32 sections with unique IDs; aggregate request formula below must fit |
| Section | `id`, `kind`, `root`, `required`, `max_results`, `fields` | `index` | `max_results` is 1–2,048; `fields` has 1–256 unique names/OIDs; `index` is required for `table` and forbidden for `scalars` |
| Field | `name`, `oid`, `format` | none | `name` is a unique ID; `oid` is module-qualified; `format` is from the v1 allowlist and compatible with resolved syntax |

### Canonical catalog artifact

`ntnt netmon mib compile <source-root> --output-dir <directory>` compiles all three source schemas together. There is no independently published MIB-only intermediate: compiler and runtime reader for `netmon.catalog/v1` ship in the same slice.

The artifact extension is `.ntntc`, not JSON. Its framing is exactly:

```text
8 bytes   magic = "NTNTMIB\0"
2 bytes   format_version, unsigned big-endian = 1
2 bytes   compiler_semantics_version, unsigned big-endian = 1
8 bytes   payload_length, unsigned big-endian
32 bytes  payload_sha256
N bytes   canonical UTF-8 JSON payload
```

Canonical JSON uses RFC 8785 JSON Canonicalization Scheme (JCS), with the additional restrictions that ntnt emits no floating-point values and accepts only valid Unicode scalar-value strings. JCS defines UTF-8 encoding, object-key ordering, string escaping, and integer rendering; no alternate escaping is accepted as canonical output.

Array order is also normative: modules by `name`; symbols by `(module, name)`; profiles and plans by `id`; profile plan IDs and aliases by UTF-8 bytes; numeric OID rules by numeric arc sequence; secondary rules by `(op, value, ascii_case_insensitive)`; sections and fields by `id` and `name`; enums and bits by numeric value/bit then label. Runtime rejects any noncanonical array order.

The payload is a closed object with these required top-level keys:

```text
schema: "netmon.catalog/v1"
format_version: 1
compiler_semantics_version: 1
catalog: { id: String, version: String }
hashes: { mib_hash, profiles_hash, plans_hash, catalog_hash }
modules: Array<ModuleRecord>
symbols: Array<SymbolRecord>
profiles: Array<ResolvedProfileRecord>
plans: Array<ResolvedPlanRecord>
```

`ModuleRecord` requires `name`, `language`, and `module_hash`. `SymbolRecord` requires `module`, `name`, `qualified_name`, canonical numeric `oid`, `kind`, `syntax`, `access`, `status`, sorted `aliases`, nullable table/index metadata, sorted enum/bit metadata, and `symbol_hash`. `ResolvedProfileRecord` requires ID/version/hash, vendor/family/model labels, default/supported plan IDs, numeric exact/prefix rules, and literal secondary rules. `ResolvedPlanRecord` requires ID/version/hash and sorted sections; each section contains ID/kind/numeric root/index, required flag, result ceiling, and sorted fields with name/numeric OID/format. Nullable fields are explicit JSON `null`; omitted keys and unknown keys are invalid.

Every record hash is computed from a named **hashless preimage record** containing every field above except its own `*_hash` field. Define the framing function:

```text
H(domain, parts...) = SHA256(
    u16be(len(domain_utf8)) || domain_utf8 ||
    for each part: u64be(len(part)) || part
)
```

Versions are two-byte unsigned big-endian parts. Digests passed to `H` are raw 32-byte values, never hexadecimal text. Digest fields stored in canonical JSON and manifest configuration are lowercase 64-character hexadecimal.

Hash definitions are:

```text
symbol_hash  = H("ntnt-netmon-symbol-v1", JCS(hashless SymbolRecord))
module_hash  = H("ntnt-netmon-module-v1",
                 JCS(hashless ModuleRecord),
                 each raw symbol_hash for that module in canonical symbol order)
profile_hash = H("ntnt-netmon-profile-v1", JCS(hashless ResolvedProfileRecord))
plan_hash    = H("ntnt-netmon-plan-v1", JCS(hashless ResolvedPlanRecord))
mib_hash      = H("ntnt-netmon-mibs-v1", each raw module_hash in canonical module order)
profiles_hash = H("ntnt-netmon-profiles-v1", each raw profile_hash in canonical profile order)
plans_hash    = H("ntnt-netmon-plans-v1", each raw plan_hash in canonical plan order)
catalog_hash  = H("ntnt-netmon-catalog-v1",
                  u16be(format_version), u16be(compiler_semantics_version),
                  raw mib_hash, raw profiles_hash, raw plans_hash)
payload_sha256  = SHA256(the exact canonical payload bytes)
artifact_sha256 = SHA256(the complete framed artifact bytes)
```

The 32-byte header field is raw `payload_sha256`. Runtime reconstructs every hashless record, recomputes every symbol/module/profile/plan and aggregate hash, compares all stored digest fields, and only then publishes the snapshot. This binds unreferenced symbol/type/table metadata as strongly as profile-referenced data.

`expected_sha256` always means `artifact_sha256`, never `payload_sha256` or semantic `catalog_hash`. Filesystem metadata, installation paths, source ordering, and timestamps do not affect semantic hashes. Declared versions are human labels, not identity; the compiler rejects a same-ID/version artifact with a different `catalog_hash` when that prior artifact is present in the selected output directory, and deployment policy must otherwise use hashes as authority.

### Compiler and runtime ceilings

Initial hard ceilings are defensive implementation limits rather than SMI-standard limits:

| Resource | Hard cap |
|---|---:|
| Manifest bytes | 256 KiB |
| Source files | 512 |
| One MIB source file | 4 MiB |
| Total MIB source bytes | 64 MiB |
| One profile or plan source | 256 KiB |
| Total profile source bytes | 4 MiB |
| Total plan source bytes | 4 MiB |
| Lexical tokens | 1,000,000 total |
| Identifier/path bytes | 128 / 512 |
| Quoted string bytes | 1 MiB |
| MIB modules | 128 |
| Definitions/symbols/AST nodes | 100,000 / 100,000 / 250,000 |
| Imports per module / total import edges | 256 / 4,096 |
| Import/type/OID chain depth | 64 |
| OID arcs | 128 |
| Profiles / recognition rules | 256 / 8,192 |
| Plans / sections / fields | 256 / 4,096 / 100,000 |
| Sections per plan | 32 |
| Collected diagnostics / diagnostic text | 10,000 / 256 bytes each |
| Compiler wall clock | 30 seconds default / 120 seconds hard |
| Compiler worker tracked heap | 512 MiB |
| Canonical catalog bytes | 64 MiB |
| Runtime registry tracked heap | 256 MiB |

The CLI parent directly spawns a hidden compiler-worker mode of the same executable, never a shell. The parent enforces the wall-clock deadline and kills the worker on expiry. The worker uses one parser thread by default, a counting allocator for its heap ceiling, pre-parser byte/token/string checks, checked counters before collection growth, and iterative/depth-bounded graph traversal. Worker crash, timeout, or budget failure leaves no publishable candidate. Diagnostics expose bounded code/module/line metadata without absolute host paths, source excerpts, or terminal control characters.

### Publication, startup, and job semantics

The compiler writes a same-directory temporary artifact with exclusive creation, validates it through the production runtime reader, and fsyncs the file. Publication uses an OS/filesystem atomic **no-replace** primitive; environments that cannot guarantee no-replace fail closed. On `AlreadyExists`, the compiler opens the existing destination, validates its complete bytes and `artifact_sha256`, and discards the temporary file only when they are identical. It never replaces an existing path. After successful publication it fsyncs the output directory and never changes application configuration. Existing content-addressed artifacts therefore remain the deployment-owned rollback set.

The optional application manifest configuration is:

```toml
[netmon.catalog]
path = "netmon/catalogs/<artifact_sha256>.ntntc"
expected_sha256 = "<artifact_sha256>"
```

The path is interpreted beneath the canonical directory containing the closest `ntnt.toml`. Absolute paths, `..`, empty/control-character components, symlinks in any component, non-regular files, and artifacts larger than the framing bytes plus 64 MiB payload cap are rejected. Runtime opens through a rooted directory handle with no-follow semantics and validates length, complete `artifact_sha256`, payload, and registry from that same opened file handle, so a path swap cannot change the validated bytes.

A new common application-bootstrap step runs before main-source execution in `run`, `test`, HTTP server workers, `ntnt worker`, and job-worker startup. Repeated configuration for the same canonical application root and artifact is idempotent; a second distinct root or artifact in one process fails as `conflicting_application`. If the section is absent, numeric `snmp_get`/`snmp_walk` remain usable and catalog APIs return `Err("catalog_not_configured")`. If the section is present but its path, digest, format, or contents are invalid, process startup fails before serving traffic or claiming jobs. This is a specific netmon bootstrap contract, not a claim that ntnt already has a generic readiness subsystem.

Every process loads one active immutable `Arc<CatalogSnapshot>` and reports both `artifact_sha256` and `catalog_hash` through `netmon_catalog_info()`. Live reload and runtime garbage collection are deferred. Deployment performs a rolling restart to select a new immutable artifact and retains/removes old files by application policy.

Queued monitoring runs persist catalog/profile/plan IDs and hashes, but Slice 1C does not alter generic `std/jobs` retry semantics or claim a netmon pre-execution hook. PR 2 adds an optional closed `expected` hash fence to `device_inventory`; on mismatch it performs no network I/O and returns an `Ok` inventory envelope with `poll_status="catalog_mismatch"`. The application job handler records that terminal outcome and returns success to `std/jobs`, then may explicitly create a new run under the current catalog. Omitting `expected` remains valid for immediate interactive calls. Multi-version runtime lookup, leases, and garbage collection are deferred.

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

The OID array accepts 1–64 unique numeric OIDs, each at most 255 bytes and 128 unsigned 32-bit arcs. `snmp_get` remains numeric; named resolution is provided separately through the immutable catalog APIs. Encoded requests are capped at 8 KiB and response datagrams at 8 KiB. Explicit values outside any bound fail rather than clamp.

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

#### `snmp_walk(target, auth, oid, opts?) -> Result<Map, String>` *(next slice)*

Walk a numeric subtree with strict row, request, byte, and result caps. This follows GET rather than sharing its first compatibility commit. Low-level transport stays numeric and never resolves a mutable MIB symbol implicitly.

Options add `max_results` (default 256, hard maximum 2,048) and `on_limit` (`"error"` by default; optional `"partial"`). WALK reuses the same strict auth contract, global timeout budget, checked-address transport binding, and normalized varbind shapes.

Before transport, compute:

```text
logical_request_ceiling = max_results + 1       # includes mandatory look-ahead
datagram_attempt_ceiling = logical_request_ceiling * (retries + 1)
```

Reject the option set when checked arithmetic overflows or `datagram_attempt_ceiling > 4,096`. Therefore `max_results = 2,048` is valid with `retries = 0`; callers requesting retries must lower `max_results`. The 8 MiB actual cumulative receive-byte ceiling and 4 MiB conservative normalized-output ceiling are dynamic independent budgets: exceeding either returns `Err` even when the count ceilings have not been reached. A count maximum is not a promise that maximum-size values fit the byte budgets.

Result shape:

```ntnt
Ok(map {
    "target": "10.0.50.1",
    "address": "10.0.50.1",
    "port": 161,
    "version": "2c",
    "root_oid": "1.3.6.1.2.1.2.2",
    "duration_ms": 123,
    "requests": 17,
    "attempts": 18,
    "complete": true,
    "stop_reason": "out_of_subtree",
    "values": [...]
})
```

`requests` counts logical GETNEXT cursors, including look-ahead. `attempts` counts every transmitted datagram, including retries. GETNEXT requests use one cursor and require exactly one response varbind. Every accepted OID must be lexicographically greater than the prior cursor. Equal, descending, or repeated OIDs are protocol errors.

The one global deadline begins before first request construction and covers every cursor, retry, decode, normalization step, mandatory look-ahead, and final result build. Terminal behavior is normative:

| Condition | Return | `complete` | `stop_reason` | Include terminal varbind? |
|---|---|---:|---|---:|
| First OID outside root subtree | `Ok` | `true` | `out_of_subtree` | no |
| `endOfMibView` | `Ok` | `true` | `end_of_mib_view` | no |
| `noSuchObject` | `Ok` | `true` | `no_such_object` | no |
| `noSuchInstance` | `Ok` | `true` | `no_such_instance` | no |
| Empty walk terminated by any row above | same `Ok` shape with empty `values` | `true` | corresponding reason | no |
| Exactly `max_results`, look-ahead terminates | `Ok` | `true` | look-ahead reason | no |
| Exactly `max_results`, look-ahead finds another valid in-subtree value and `on_limit="partial"` | `Ok` | `false` | `max_results` | no |
| Same condition and `on_limit="error"` | `Err` | n/a | n/a | no |
| Deadline, datagram, receive-byte, or output budget exhausted, including during look-ahead | `Err` | n/a | n/a | no |
| Agent error status, malformed BER, wrong correlation, invalid OID progression, or transport failure | `Err` | n/a | n/a | no |

The look-ahead request is mandatory whenever `max_results` values have been accepted and completion is to be claimed. Low-level WALK never returns mid-operation transport/protocol failures as partial telemetry.

#### `snmp_bulk_walk(target, auth, oid, opts?) -> Result<Map, String>`

Optional optimization after basic walk is stable. SNMP GETBULK can reduce polling overhead but should not be in PR 1 unless the implementation stays small and testable.

#### `snmp_capabilities(target, auth, opts?) -> Result<Map, String>`

Probe SNMP availability and supported basics without doing a full inventory poll.

```ntnt
snmp_capabilities("10.0.50.1", auth)
// Ok(map { "reachable": true, "version": "2c", "sys_object_id": "...", "vendor_hint": "mikrotik" })
```

### Catalog and recognition APIs

The active catalog is configured by deployment, not loaded from a path supplied by ordinary ntnt code. The manifest points at an immutable `.ntntc` artifact and pins the SHA-256 of the entire framed file as defined above. Catalog absence leaves numeric SNMP primitives available; invalid configured catalog data aborts application startup.

#### `netmon_catalog_info() -> Result<Map, String>`

Return only schema/compiler versions, declared catalog identity, `artifact_sha256`, semantic hashes, counts, and load state. Never return host paths, source text, or the full registry. When no catalog is configured, return `Err("catalog_not_configured")`.

#### `mib_resolve(symbol_or_oid) -> Result<Map, String>`

Resolve a module-qualified symbol, unambiguous bare symbol, or numeric OID against one snapshot:

```ntnt
mib_resolve("IF-MIB::ifHCInOctets")
// Ok(map {
//   "oid": "1.3.6.1.2.1.31.1.1.1.6",
//   "qualified_symbol": "IF-MIB::ifHCInOctets",
//   "module": "IF-MIB",
//   "kind": "column",
//   "syntax": "Counter64",
//   "catalog_hash": "..."
// })
```

The result may include bounded access, status, table/index, enum, bit, and alias metadata. Ambiguous names are errors; there is no file-order primary symbol.

#### `profile_match(identity) -> Result<Map, String>`

Purely classify an already observed identity map without network I/O. The closed input requires `sys_object_id: String`, permits `sys_descr: String`, and rejects unknown keys. The result always has one of these shapes:

```ntnt
Ok(map {
    "status": "matched",
    "catalog_hash": "...",
    "profile_id": "acme-switch",
    "profile_hash": "...",
    "default_plan_id": "acme-switch-inventory",
    "evidence": [map { "field": "sys_object_id", "kind": "exact", "rule": "..." }]
})

Ok(map { "status": "no_match", "catalog_hash": "...", "evidence": [] })

Ok(map {
    "status": "ambiguous",
    "catalog_hash": "...",
    "candidate_profile_ids": ["acme-a", "acme-b"],
    "evidence": [...]
})
```

Candidate IDs and evidence are deterministically sorted. Static duplicate exact/prefix rules fail compilation; runtime ambiguity remains possible for overlapping literal `sysDescr` rules and is never broken by file order.

#### `device_recognize(target, auth, opts?) -> Result<Map, String>`

Read a fixed bounded SYSTEM identity set, then pass its closed identity map through `profile_match`. Return the same recognition envelope plus sanitized observed identity. Device-controlled identity is advisory and never changes target policy, credentials, port, hard caps, or protocol authority.

#### `device_walk_plan(profile_id, plan_id, opts?) -> Result<Map, String>`

Return one selected plan's precompiled numeric sections without network I/O:

```ntnt
Ok(map {
    "catalog": map { "artifact_sha256": "...", "catalog_hash": "..." },
    "profile": map { "id": "acme-switch", "hash": "..." },
    "plan": map { "id": "acme-switch-inventory", "hash": "..." },
    "limits": map {
        "timeout_ms": 30000,
        "max_requests": 4096,
        "max_attempts": 4096,
        "max_rows": 4096,
        "max_received_bytes": 8388608,
        "max_output_bytes": 4194304,
        "max_concurrency": 1
    },
    "sections": [map {
        "id": "interfaces",
        "kind": "table",
        "root_oid": "1.3.6.1.2.1.2.2",
        "required": true,
        "max_results": 1024,
        "fields": [...]
    }]
})
```

Callers may lower effective limits through the closed options map but cannot raise plan or runtime hard caps. The explicit profile and plan IDs avoid a forgeable union-shaped `identity_or_profile` argument. The function rejects unknown profiles, plans not listed by that profile, and invalid options. Persisted jobs store the returned IDs and hashes.

#### `mib_walk(target, auth, root, opts?) -> Result<Map, String>`

Resolve one module-qualified root against the active catalog snapshot, then execute `snmp_walk` numerically while reporting `artifact_sha256` and `catalog_hash`. Symbol resolution occurs once at operation start; restart/reconfiguration cannot change the walk mid-operation.

### Interface telemetry

#### `interface_list(target, auth, opts?) -> Result<Array<Map>, String>`

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

#### `interface_counters(target, auth, opts?) -> Result<Array<Map>, String>`

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

#### `device_identity(target, auth, opts?) -> Result<Map, String>`

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

#### `device_inventory(target, auth, opts?) -> Result<Map, String>`

The closed options map may include `expected`, a closed all-or-none map containing `artifact_sha256`, `catalog_hash`, `profile_id`, `profile_hash`, `plan_id`, and `plan_hash`. Before any identity probe or other network I/O, runtime compares those values with the active snapshot and referenced profile/plan records. Any mismatch returns the terminal `Ok` envelope described below with `poll_status="catalog_mismatch"`; invalid field types or partial expected maps return `Err`. Applications pass this map for queued runs and may omit it for immediate calls.

Higher-level inventory bundle:

```ntnt
Ok(map {
    "schema_version": "netmon.inventory/v1",
    "catalog": map {
        "artifact_sha256": "...",
        "catalog_hash": "...",
        "mib_hash": "...",
        "profiles_hash": "...",
        "plans_hash": "..."
    },
    "recognition": map {
        "status": "matched",
        "profile_id": "acme-switch",
        "profile_hash": "...",
        "evidence": [...]
    },
    "plan": map {
        "id": "acme-switch-inventory",
        "hash": "..."
    },
    "identity": map { ... },
    "interfaces": [...],
    "neighbors": [...],
    "routes_summary": map { ... },
    "poll_status": "partial",
    "sections": [map {
        "id": "interfaces",
        "plan_id": "acme-switch-inventory",
        "plan_hash": "...",
        "root_oid": "1.3.6.1.2.1.2.2",
        "status": "complete",
        "rows": 24,
        "walk_complete": true,
        "walk_stop_reason": "out_of_subtree",
        "requests": 25,
        "attempts": 25
    }],
    "warnings": [map { "code": "lldp_unsupported", "section": "neighbors" }]
})
```

Hash-fence mismatch shape:

```ntnt
Ok(map {
    "schema_version": "netmon.inventory/v1",
    "poll_status": "catalog_mismatch",
    "network_attempted": false,
    "expected": map { ... },
    "actual": map {
        "artifact_sha256": "...",
        "catalog_hash": "...",
        "profile_id": "...",
        "profile_hash": "...",
        "plan_id": "...",
        "plan_hash": "..."
    },
    "sections": [],
    "warnings": [map { "code": "catalog_mismatch" }]
})
```

One `device_inventory` call holds one catalog snapshot and one whole-operation budget across recognition and every plan section:

| Inventory resource | Hard cap |
|---|---:|
| Whole-operation timeout | 30,000 ms |
| Recognition probes | 8 |
| Plan sections | 32 |
| Concurrent section walks | 1 |
| Logical requests, including probes/look-aheads | 4,096 |
| Datagram attempts, including retries | 4,096 |
| Accepted rows across all sections | 4,096 |
| Actual received bytes | 8 MiB |
| Conservative normalized output | 4 MiB |

The compiler requires `recognition_probe_ceiling + Σ(section.max_results + 1) <= 4,096`, where each `+ 1` reserves mandatory WALK look-ahead. Runtime recomputes the same checked equation from caller-lowered section limits, then multiplies every probe/section request ceiling by `(retries + 1)` and rejects the call before transport if its datagram-attempt ceiling exceeds 4,096. Each section's effective result limit is the lower of its compiled limit, caller-lowered limit, and remaining aggregate row/request budget. Recognition probes consume the same deadline/request/attempt/receive-byte budgets as inventory walks.

V1 tolerates only explicit optional-section absence as partial inventory: `noSuchObject`, `noSuchInstance`, or an agent's supported-table absence marks an optional section `unsupported` and continues. The same outcome for a required section returns `Err`. Any authentication/policy failure, malformed or mismatched protocol response, transport failure, whole-operation deadline, aggregate budget exhaustion, or required-section truncation returns `Err`; it is never disguised as partial inventory. Optional-section truncation returns `Ok` with `poll_status="partial"`, `walk_complete=false`, and the exact stop reason. A fully executed plan returns `complete`; `no_match` or `ambiguous` recognition returns a stable non-inventory `Ok` envelope without executing a plan.

Device-controlled text is sanitized and capped, raw enum codes are preserved beside optional MIB labels, raw varbinds are omitted by default, and every section preserves plan/root/WALK completion provenance.

### Topology hints

#### `lldp_neighbors(target, auth, opts?) -> Result<Array<Map>, String>`

Read LLDP-MIB where available.

#### `cdp_neighbors(target, auth, opts?) -> Result<Array<Map>, String>`

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

#### `check_snmp(target, auth, opts?) -> Result<Map, String>`

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
- offline-compiled MIB catalogs and symbol metadata
- data-driven device recognition and finite walk plans
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
- [ ] **Slice 1B — bounded numeric SNMP WALK**
- [ ] **Slice 1C — canonical MIB catalog compiler, runtime registry, profiles, and plans**
- [ ] **PR 2 — device recognition and inventory execution**
- [ ] **PR 3 — interface inventory and counters**
- [ ] **PR 4 — counter-rate normalization**
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

### Slice 1B — Bounded Numeric SNMP WALK

Scope:

- [ ] `snmp_walk(target, auth, oid, opts?) -> Result<Map, String>`.
- [ ] GETNEXT with one cursor and exactly one correlated response varbind.
- [ ] Strict subtree, monotonic-order, result, request, datagram, cumulative-byte, and normalized-output enforcement.
- [ ] Explicit `complete` and `stop_reason` output, including bounded look-ahead at `max_results`.
- [ ] Loop, equal/descending OID, malformed-order, `endOfMibView`, and out-of-subtree handling.
- [ ] One whole-operation deadline covering retries, decode, normalization, and result construction.
- [ ] Reuse Slice 1A auth, target policy, packet caps, and normalization contracts.
- [ ] GETBULK only after equivalent fixture coverage.

Acceptance:

- [ ] No walk can silently return a truncated table as complete.
- [ ] Mid-walk transport/protocol failure returns `Err` rather than apparently complete telemetry.
- [ ] Independent UDP fixtures cover malicious loops, subtree escape, retries, caps, and valid termination.

### Slice 1C — Canonical MIB Catalog Compiler and Runtime Registry

Compiler scope:

- [ ] `ntnt netmon mib compile <source-root> --output-dir <directory>` parent/isolated-worker command.
- [ ] ntnt-controlled pinned/vendor SMIv1/SMIv2 parser-resolver substrate with one parser worker by default.
- [ ] Closed manifest/profile/plan v1 schemas, explicit sources, all documented byte/token/node/import/depth/time/heap limits, and deterministic import index.
- [ ] No request-path ASN.1 parsing, implicit directory recursion, system MIB discovery, network fetch, shell, plugin, or dynamic library.
- [ ] Exact `.ntntc` framing, RFC 8785 canonical payload, per-symbol/record/aggregate hashes, atomic no-replace content-addressed publication, and production-reader validation before publication.
- [ ] Valid, malformed, cyclic, conflicting, deeply nested, vendor-sloppy, timeout, memory-cap, and crash fixture coverage.

Runtime scope:

- [ ] Optional `[netmon.catalog]` manifest parsing and one common fallible bootstrap used by `run`, `test`, HTTP server, `ntnt worker`, and job workers.
- [ ] Process-global immutable `Arc<CatalogSnapshot>` with exact artifact-hash enforcement; no public mutable `mib_load(path)` API.
- [ ] `netmon_catalog_info()`, `mib_resolve(symbol_or_oid)`, pure `profile_match(identity)`, and `device_walk_plan(profile_id, plan_id, opts?)`.
- [ ] Separately typed MIB/profile/plan records compiled and published together against one snapshot.
- [ ] Restart/rolling-restart selection of immutable artifacts and catalog inspection for application-owned hash persistence; live reload, job enforcement, and multi-version runtime retention deferred.

Acceptance:

- [ ] Compiler failure cannot overwrite or select an application catalog.
- [ ] Duplicate/ambiguous definitions, unresolved imports/symbols, invalid profile ties, and plans exceeding aggregate caps fail closed.
- [ ] Artifact bytes and every semantic hash are deterministic across source creation/order and Linux/macOS/Windows.
- [ ] Diagnostics are bounded and sanitized; parser crash/timeout/heap exhaustion remains outside the application runtime.
- [ ] Missing catalog configuration leaves numeric GET/WALK working while catalog APIs return `catalog_not_configured`.
- [ ] Invalid configured artifacts abort every application/worker startup path before traffic or job claims.
- [ ] All interpreter workers in one process observe the same hashes; separate processes either report matching hashes or fail their configured hash check.
- [ ] Plans cannot contain credentials, targets, cap increases, callbacks, code, or SNMP SET.

### PR 2 — Device Recognition and Inventory Execution

Scope:

- [ ] `device_recognize(target, auth, opts?)`.
- [ ] `mib_walk(target, auth, root, opts?)`.
- [ ] `device_identity(target, auth, opts?)`.
- [ ] `device_inventory(target, auth, opts?)` with the optional all-or-none `expected` catalog/profile/plan hash fence.
- [ ] Fixed bounded SYSTEM identity probes, profile matching, and profile-selected finite plan execution.
- [ ] Versioned normalized envelope with catalog/profile/plan hashes and section-level partial status.

Acceptance:

- [ ] Recognition returns exact evidence and ambiguity rather than treating device-controlled identity as authorization.
- [ ] Inventory preserves complete/partial/failed section status and stable warning codes.
- [ ] Every persisted run can record the exact catalog/profile/plan hashes used.
- [ ] A queued run passing mismatched `expected` hashes performs zero network I/O, returns terminal `catalog_mismatch`, and can be acknowledged successfully by the application handler without `std/jobs` retrying it.
- [ ] Optional table failure does not discard independently complete sections.

### PR 3 — Interface Inventory and Counters

Scope:

- [ ] `interface_list(target, auth, opts?)`.
- [ ] `interface_counters(target, auth, opts?)`.
- [ ] IF-MIB table/index normalization through compiled catalog metadata.
- [ ] Prefer high-capacity 64-bit counters when present.
- [ ] Explicit fallback metadata for 32-bit counters.

Acceptance:

- [ ] Interfaces have stable normalized fields and deterministic numeric-index ordering.
- [ ] Counters include timestamp and counter width.
- [ ] Missing optional fields degrade gracefully.
- [ ] Fixture covers up/down/admin-down interfaces.

### PR 4 — Counter-Rate Normalization

Scope:

- [ ] `interface_rates(previous, current, opts?)`.
- [ ] Counter wrap/reset detection.
- [ ] Utilization percentage from speed when available.
- [ ] Invalid delta filtering.

Acceptance:

- [ ] 64-bit and 32-bit counters calculate sane rates.
- [ ] Wraps/resets are detected and marked.
- [ ] Negative/impossible deltas do not produce bogus traffic spikes.
- [ ] Tests cover missing speed and zero interval.

### PR 5 — Topology Hints from LLDP

Scope:

- [ ] `lldp_neighbors(target, auth, opts?)`
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

Raw MIB compilation uses an ntnt-controlled, exact-source snapshot derived from the MIT-licensed `mib-rs` SMIv1/SMIv2 parser/resolver. A direct upstream parser API is not exposed to ntnt programs or the polling runtime. The compiler wrapper disables default CLI/Serde features not needed by ntnt, removes implicit/system path sources, fixes parser parallelism to one by default, adds checked resource budgets and graph-depth limits, and normalizes only the catalog records ntnt needs. If those controls cannot be maintained as a pinned fork/vendor snapshot with three-platform corpus coverage, the compiler slice does not ship.

The runtime catalog reader is ntnt-owned and does not depend on the ASN.1 parser. It validates only the canonical format and publishes immutable sorted schema/profile/plan data.

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
- [ ] MIB source manifest path/symlink/collision and byte/count/depth ceilings
- [ ] SMIv1/SMIv2 imports, table/index/type chains, cycles, conflicts, and vendor-sloppy fixtures
- [ ] deterministic canonical catalog bytes and hashes across source ordering/platforms
- [ ] runtime catalog framing/length/digest/version/structural validation and configured-artifact startup failure
- [ ] profile recognition precedence, no-match/ambiguity envelopes, and plan-to-numeric compilation
- [ ] multi-worker/process catalog hash consistency and immutable artifact rollback
- [ ] queued-run expected-hash mismatch with zero network I/O and application-owned terminal acknowledgement
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

4. **OID/MIB strategy — resolved:** use hardcoded numeric OIDs only for fixed bootstrap probes such as `sysObjectID.0`; compile third-party SMIv1/SMIv2 source offline into one canonical catalog for general symbols, table metadata, profiles, and plans. Ordinary ntnt code cannot load raw MIB paths, and polling never parses ASN.1.

5. **Discovery scope:** should `std/netmon` include subnet discovery?

   Recommendation: not initially. Require explicit device inventory first. Add bounded discovery later after policy and abuse controls are proven.

6. **Alert delivery:** should the module send notifications?

   Recommendation: no. It can calculate alert state; apps own email/webhook/Telegram/PagerDuty delivery.

---

## Bottom Line

`std/netmon` is now the standard-library home for bounded monitoring protocols and reusable normalization on top of DD-046's safe network policy.

Start narrow. Keep credentials opaque. Bind transport to checked addresses. Compile untrusted MIB source outside the polling runtime. Publish schema, recognition, and finite plan data as one immutable catalog. Normalize without truncating, and let applications own target inventory and the monitoring product around those primitives. This gives ntnt a credible third-party SNMP inventory foundation without turning the default stdlib into a closet full of enterprise networking adapters wearing one trench coat.
