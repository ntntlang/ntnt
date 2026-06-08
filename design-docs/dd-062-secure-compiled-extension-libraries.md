# DD-062: Secure Compiled Extension Libraries

**Status:** Draft  
**Author:** Larri + Josh  
**Created:** 2026-06-07  
**App:** ntnt  
**Related:** DD-046 `std/net`, DD-047 `std/netmon`, DD-058 stdlib gaps, DD-060 AI-native developer experience  

---

## Executive Summary

ntnt should support **extension libraries**: Rust-native modules that feel exactly like the standard library when present, but are selected, built, signed, and verified as part of a specific ntnt binary distribution.

This is intentionally **not** a Node/npm-style package manager. ntnt's core advantage is that common capabilities are built in, documented, typed, and stable. Extension libraries should preserve that experience while giving ntnt room to grow into specialized domains like network monitoring, email, media, AI tooling, industrial control, geospatial, finance, or organization-specific modules.

The guiding idea:

> Every ntnt binary has a declared, signed **Module Universe**. Apps can require a specific universe, official build channel, extension set, and trust level before they run.

An ntnt app should be able to say:

```toml
# ntnt.toml
[requires]
ntnt = ">=0.4.10"
modules = ["std/net", "std/netmon"]
trust = "official-or-org"
allow_unsigned_extensions = false
```

Then `ntnt check`, `ntnt run`, and deployment smoke checks fail before execution if the binary is not the right build.

This DD proposes:

1. A shared compiled module registry for runtime, typechecker, docs, and CLI.
2. Extension crates that compile into ntnt binaries and expose first-class module specs.
3. Build-time extension selection via distribution manifests, not app-time downloads.
4. Embedded signed module-universe manifests inside every ntnt binary.
5. CLI and runtime verification APIs for official builds, signed extensions, and app-required module sets.
6. A trust model that supports official, certified, community, and organization-local extensions without letting arbitrary packages masquerade as stdlib.
7. A supply-chain posture based on reproducible builds, SLSA/in-toto provenance, Sigstore/TUF-style signing, cargo-vet/cargo-deny, SBOMs, and binary transparency.

The result is a language that can grow beyond a single public stdlib without inheriting the chaos of ambient package ecosystems. More kitchen, fewer drawers full of npm-shaped knives.

---

## Problem

ntnt currently gets a lot of safety and developer experience from having core capabilities in the standard library:

- imports are stable and obvious;
- docs are generated centrally;
- the typechecker knows the APIs;
- the runtime has native Rust implementations;
- apps do not need to `npm install` a dependency tree just to send email, parse JSON, or query a database.

That model becomes strained when capabilities are:

- specialized but important, e.g. SNMP/device telemetry/network monitoring;
- legally, commercially, or operationally sensitive;
- too heavy for the default free/public binary;
- organization-specific;
- useful enough to be first-class when present, but not universal enough to ship to everyone.

The obvious alternatives are bad:

### Bad Option: Runtime package manager

Dynamic runtime packages are convenient, but create the same attack surface that has repeatedly hurt npm/PyPI ecosystems:

- typosquatting;
- dependency confusion;
- maintainer account takeover;
- malicious transitive updates;
- install scripts;
- runtime code swapping during app deploy;
- AI agents adding dependencies because the import name looks plausible.

This undermines ntnt's core value.

### Bad Option: Sidecar runtime binary

A second runtime process can work for operational tools, but not for stdlib-grade APIs. It creates:

- lifecycle coordination;
- IPC serialization;
- local auth/secrets;
- version skew;
- debugging across process boundaries;
- deployment seams.

If a feature is meant to feel like `std/net`, it should not secretly be a localhost microservice wearing a trench coat.

### Bad Option: One-off private forks

Forking the interpreter and manually adding modules to runtime/typechecker/docs works once and decays immediately. It creates drift and makes official/public promotion painful.

---

## Goals

1. **First-class developer experience**
   - Extension modules import and behave like stdlib modules when included in the binary.
   - Typechecking, docs, examples, errors, and CLI discovery work the same way.

2. **No app-time code download by default**
   - Apps do not fetch arbitrary code during `ntnt run`.
   - Extensions are selected and compiled into a binary distribution ahead of time.

3. **Explicit binary identity**
   - Every binary can declare what modules and extensions it contains.
   - Apps and CI can fail closed when the wrong binary is used.

4. **Strong supply-chain posture**
   - Official builds have signed provenance.
   - Extension source, dependency set, build inputs, and binary digest are attestable.
   - Optional organization-local builds can be signed by organization keys.

5. **Path to public, official, and organization-local extensions**
   - The system should support public community extensions, official promoted extensions, and internal/org-only extensions.
   - The word “private” should describe a distribution choice, not the architecture.

6. **No stdlib impersonation**
   - Only official ntnt module authorities can publish modules under reserved `std/*` names, unless an organization distribution explicitly owns an internal namespace policy.

7. **Extensible without becoming npm**
   - Adding a new Rust extension should be straightforward for serious developers.
   - Running arbitrary unreviewed extension code should not be casual or invisible.

---

## Non-Goals

- A general-purpose npm/PyPI-style package manager.
- Runtime loading of unsigned native code.
- Dynamic Rust plugin ABI as the first implementation.
- Solving full sandboxing for native Rust extension code in v1.
- Allowing community extensions to silently occupy `std/*` names.
- Making self-attestation magically protect against a malicious binary that lies about itself. External verification remains necessary for strong binary-swap protection.

---

## Design Principles

### 1. The binary is the unit of trust

An ntnt runtime is not just an interpreter version. It is:

- interpreter/runtime version;
- compiled module universe;
- build channel;
- source commit;
- dependency lock;
- build provenance;
- extension authorities.

The app should be able to inspect and require that identity.

### 2. Extensions are not packages; they are compiled capabilities

A normal ntnt app should never download extension code as a side effect of app deployment. Instead, a distribution builder chooses an extension set and produces a binary.

### 3. The module registry is single-source-of-truth

Runtime, typechecker, docs, and CLI discovery must read the same module metadata. If adding an extension requires touching five registries, the design has already failed.

### 4. Trust is visible

Every module should have visible trust metadata:

- module id;
- publisher;
- trust tier;
- source digest;
- build digest;
- capabilities;
- docs status;
- signatures/provenance.

### 5. `std/*` is reserved

`std/*` should mean “built into this binary and governed by the ntnt standard-library authority.” Some `std/*` modules may be optional extension modules, but they are still official.

Community and organization extensions should use non-`std` namespaces unless explicitly promoted.

### 6. AI agents need guardrails, not vibes

As AI writes more code, supply-chain mistakes become faster and more plausible. ntnt should make dependency risk machine-checkable:

- no implicit package installation;
- explicit module requirements;
- signed build manifests;
- audit statuses;
- CLI diagnostics;
- policy files that agents can read and obey.

---

## Terminology

### Module Universe

The complete set of modules built into a specific ntnt binary, including core stdlib modules and compiled extensions.

### Extension Library

A Rust-native library that contributes one or more ntnt modules to the module universe at compile time.

### Distribution

A named ntnt binary build profile/channel, such as:

- `ntnt-community` — public default binary;
- `ntnt-official-full` — official binary with optional official extensions;
- `ntnt-larri-network` — organization-local binary with network monitoring extensions;
- `ntnt-enterprise` — commercial or certified distribution.

The executable may still be named `ntnt`; the distribution identity lives in the embedded manifest.

### Module Authority

The entity allowed to sign and publish module identity claims for a namespace.

Examples:

- ntnt project authority for `std/*`;
- organization authority for `org/larri/*`;
- community publisher authority for `x/<publisher>/*` or another namespace.

### Extension Trust Tier

Proposed tiers:

- `core` — always included public stdlib;
- `official` — ntnt-governed extension, may be optional by distribution;
- `certified` — third-party or commercial extension certified by ntnt policy;
- `community` — signed by publisher, not ntnt-certified;
- `organization` — signed by organization-local authority;
- `local-dev` — unsigned or locally built; never accepted unless explicitly allowed.

---

## Proposed Developer Experience

### Official optional extension

```ntnt
import { interface_counters, snmp_get } from "std/netmon"

let counters = interface_counters("10.0.50.1", map {
    "community_env": "SWITCH_SNMP_COMMUNITY",
    "timeout_ms": 1500
})
```

If the binary includes `std/netmon`, this behaves like any other stdlib import.

If not:

```text
Unknown stdlib module: std/netmon

This binary does not include official extension module std/netmon.
Run `ntnt modules` to see available modules.
If this app requires std/netmon, use a distribution that includes it or declare it in ntnt.toml [requires].
```

### Organization extension

```ntnt
import { device_inventory } from "org/larri/netops"
```

Organization modules can be first-class in a distribution without claiming to be universal stdlib.

### App-level requirement

```toml
# ntnt.toml
[requires]
ntnt = ">=0.4.10"
modules = [
  "std/net",
  "std/netmon",
]
trust = "official-or-org"
allow_unsigned_extensions = false

[requires.build]
official = true
channels = ["official-full", "larri-network"]

[requires.modules."std/netmon"]
min_version = "0.1.0"
trust = "official"
capabilities = ["network.udp", "network.private", "env.read"]
```

### CLI discovery

```bash
ntnt modules
ntnt modules --json
ntnt modules --trust
ntnt verify
ntnt verify --binary ./target/release/ntnt
ntnt check server.tnt
```

Example `ntnt modules --trust`:

```text
Module           Version  Tier          Publisher      Signed  Capabilities
std/string       0.4.10   core          ntnt           yes     pure
std/net          0.4.10   core          ntnt           yes     network.tcp, network.dns, network.tls
std/netmon       0.1.0    official      ntnt           yes     network.udp, network.private, env.read
org/larri/netops 0.1.0    organization  larri          yes     network.private, env.read
```

### Runtime/API introspection

Add a small build/module inspection API, likely under `std/runtime` or a compiler builtin:

```ntnt
import { build_info, modules, require_modules, require_official_build } from "std/runtime"

require_official_build(map {
    "allow_organization": true,
    "allow_unsigned_extensions": false
})

require_modules([
    "std/net",
    "std/netmon"
])

let info = build_info()
```

This helps apps fail early, but it is not the only security layer. A malicious swapped binary could lie from inside the process; deployment should also verify the binary externally.

---

## Architecture

### `NtntModuleSpec`

Introduce a single module-spec structure consumed by the interpreter, typechecker, docs generator, and CLI.

```rust
pub struct NtntModuleSpec {
    pub id: ModuleId,
    pub version: &'static str,
    pub tier: ModuleTier,
    pub publisher: PublisherId,
    pub init: fn() -> StdlibModule,
    pub signatures: fn() -> ModuleSigs,
    pub docs: fn() -> ModuleDocs,
    pub capabilities: &'static [ModuleCapability],
    pub source: ModuleSource,
}
```

Where:

```rust
pub enum ModuleTier {
    Core,
    Official,
    Certified,
    Community,
    Organization,
    LocalDev,
}

pub enum ModuleCapability {
    Pure,
    EnvRead,
    FsRead,
    FsWrite,
    NetworkDns,
    NetworkTcp,
    NetworkUdp,
    NetworkPrivate,
    NetworkRawSocket,
    ProcessSpawn,
    CryptoKeyAccess,
}
```

Current modules migrate into specs:

```rust
pub fn compiled_modules() -> Vec<NtntModuleSpec> {
    let mut modules = vec![
        std_string::spec(),
        std_json::spec(),
        std_net::spec(),
        // ...
    ];

    modules.extend(crate::extensions::compiled_extension_modules());
    modules
}
```

### Extension crate interface

An extension crate exports specs:

```rust
pub fn ntnt_extension_specs() -> Vec<NtntModuleSpec> {
    vec![netmon::spec()]
}
```

A distribution build links extension crates and provides them to the registry:

```rust
fn main() {
    ntnt_cli::run_with_extensions(vec![
        ntnt_ext_netmon::netmon::spec(),
        larri_ext_netops::netops::spec(),
    ])
}
```

The public/default binary calls:

```rust
ntnt_cli::run_with_extensions(vec![])
```

### Runtime/typechecker/docs integration

The registry drives:

- interpreter module loading;
- typechecker import signatures;
- docs generation;
- `ntnt modules`;
- app requirement checks;
- build manifest generation.

This removes the current drift-prone pattern where `src/stdlib/mod.rs`, `interpreter.rs`, `typechecker.rs`, and docs logic can diverge.

---

## Namespaces

### Reserved official namespace

`std/*` is reserved for ntnt-governed modules.

A module may be:

- `core` and always included, e.g. `std/string`;
- `official` and optional by distribution, e.g. `std/netmon` if accepted as an official extension.

Community extensions cannot claim `std/*`.

### Organization namespace

Organizations use:

```text
org/<org>/<module>
```

Example:

```ntnt
import { rack_inventory } from "org/larri/netops"
```

### Community namespace

Community modules use a non-reserved namespace:

```text
x/<publisher>/<module>
```

or another naming scheme chosen later.

Example:

```ntnt
import { convert } from "x/acme/units"
```

### Promotion path

A community or organization module can be promoted:

1. `x/acme/netmon` or `org/larri/netops` proves useful.
2. API stabilizes.
3. Security/audit/docs/signature requirements pass.
4. ntnt accepts it as `std/netmon` or another official module.

Promotion is explicit; namespace squatting is not.

---

## Build-Time Extension Selection

### Preferred: distribution manifest

Use a build manifest to define a binary distribution:

```toml
# ntnt-dist.toml
[distribution]
id = "larri-network"
channel = "organization"
base = "ntnt-community@0.4.10"

[extensions]
"std/netmon" = { crate = "ntnt-ext-netmon", version = "0.1.0", trust = "official" }
"org/larri/netops" = { crate = "larri-ext-netops", version = "0.1.0", trust = "organization" }

[policy]
allow_unsigned_extensions = false
require_provenance = true
require_vetted_dependencies = true
```

Build command:

```bash
ntnt dist build ntnt-dist.toml
```

or internally:

```bash
cargo build --release -p ntnt-distribution-larri-network
```

### Why not auto-detect source files?

Auto-detection is convenient but dangerous and non-reproducible:

- hidden local files can alter a binary;
- CI and local builds drift;
- dependency declarations become implicit;
- provenance is harder to reason about.

Prefer explicit distribution manifests. If a developer wants convenience, provide scaffolding tools, not magical source discovery.

---

## Embedded Module Universe Manifest

Every built binary embeds a manifest:

```json
{
  "schema": "ntnt.module-universe.v1",
  "ntnt_version": "0.4.10",
  "distribution_id": "larri-network",
  "channel": "organization",
  "source_commit": "...",
  "cargo_lock_sha256": "...",
  "rustc": "1.94.0",
  "target": "x86_64-unknown-linux-gnu",
  "build_profile": "release",
  "modules": [
    {
      "id": "std/net",
      "version": "0.4.10",
      "tier": "core",
      "publisher": "ntnt",
      "source_digest": "...",
      "capabilities": ["network.dns", "network.tcp", "network.tls"]
    },
    {
      "id": "std/netmon",
      "version": "0.1.0",
      "tier": "official",
      "publisher": "ntnt",
      "source_digest": "...",
      "capabilities": ["network.udp", "network.private", "env.read"]
    }
  ],
  "sbom_digest": "...",
  "provenance_digest": "...",
  "signatures": []
}
```

This manifest is used by:

- `ntnt modules --json`;
- `ntnt verify`;
- `ntnt check` app requirements;
- docs generation;
- deployment inspection;
- crash reports/diagnostics.

---

## Verification and Supply-Chain Security

### Threat model

We want to protect against:

1. App accidentally running under a binary missing required modules.
2. App accidentally running under a binary with unapproved extensions.
3. A malicious actor replacing a binary with a lookalike binary.
4. A malicious extension masquerading as official stdlib.
5. Dependency confusion or transitive dependency compromise during extension builds.
6. AI-generated code pulling in unreviewed modules or trusting plausible names.
7. CI/release infrastructure producing a binary that cannot be tied to source.

We do not fully solve in v1:

- native Rust extension sandboxing after it is compiled into the binary;
- a malicious binary lying about its own identity once already executed;
- compromised OS/package-manager trust roots.

Those require external verification, sandboxing, or host attestation.

### Verification layers

#### Layer 1: App requirement policy

`ntnt.toml` declares required modules, versions, trust tiers, and unsigned-extension policy. `ntnt check` and `ntnt run` enforce before app code executes.

#### Layer 2: Embedded module-universe manifest

The binary declares exactly what modules it contains.

#### Layer 3: Signed build provenance

Official builds produce signed provenance using in-toto/SLSA-style attestations:

- source repository and commit;
- build workflow identity;
- build inputs;
- Cargo.lock digest;
- extension manifest digest;
- output binary digest;
- SBOM digest.

#### Layer 4: Binary signature and transparency

Release artifacts are signed and published to a transparency log. Sigstore/cosign is a strong candidate for public official builds; organization-local builds can use org keys and optionally private transparency logs.

#### Layer 5: TUF-style trust root/delegation

Use a root trust metadata file to define who can sign:

- ntnt official builds;
- `std/*` module authorities;
- certified extension authorities;
- organization-local namespaces.

TUF-style delegation matters because one key should not sign everything forever.

#### Layer 6: Dependency audit gate

Builds for official/certified extensions require:

- `cargo vet` audits for third-party crates;
- `cargo deny` for advisories/licenses/duplicate-risk policy;
- pinned `Cargo.lock`;
- SBOM generation;
- CI fails on unvetted dependency changes.

#### Layer 7: Reproducible build checks

Where practical, official releases should support reproducible or independently corroborated builds. SLSA describes “verified reproducible” as independent build systems corroborating provenance. ntnt can aim for this for release binaries even if early versions are not perfectly bit-for-bit reproducible.

---

## Important Limitation: Self-Verification Is Not Enough

An app can call:

```ntnt
require_official_build()
```

This catches accidental mismatch. It does **not** fully stop a maliciously swapped binary that simply lies.

Strong binary-swap protection requires external verification before execution:

```bash
ntnt-verify ./ntnt --require official --require-no-unsigned-extensions
./ntnt run server.tnt
```

or package-manager/container-image verification:

- signed release artifact;
- signed container image;
- provenance verification in CI/CD;
- deploy only if digest matches an allowed transparency-log entry.

A small standalone verifier may be useful because relying on the possibly-malicious `ntnt verify-self` command is weaker. `ntnt verify` is still valuable for diagnostics, but a separate verifier gives operators a cleaner trust boundary.

---

## Capability Model

Native Rust extensions cannot be fully sandboxed by declarations. Still, capability declarations are valuable because they let apps and CI reason about risk.

Example capabilities:

```text
pure
fs.read
fs.write
env.read
network.dns
network.tcp
network.udp
network.private
network.raw_socket
process.spawn
crypto.keys
```

App policy can reject dangerous capabilities:

```toml
[requires.policy]
deny_capabilities = ["process.spawn", "fs.write"]
allow_capabilities = ["network.dns", "network.tcp", "network.udp", "env.read"]
```

This is not a sandbox. It is a contract and audit surface. If a community extension claims `pure` but uses network APIs internally, certification should fail and the extension should lose trust.

Future work can explore real sandboxed extensions using WASM components for dynamic/public extension ecosystems. For native stdlib-grade modules, trust and build provenance remain the primary defense.

---

## Extension Trust Policy

### Official extension

Requirements:

- ntnt-governed namespace or accepted `std/*` module;
- signed by ntnt module authority;
- docs and type signatures included;
- tests in official CI;
- cargo-vet/cargo-deny pass;
- provenance and SBOM published;
- semantic versioning and compatibility policy.

### Certified extension

Requirements:

- signed by publisher;
- audited or reviewed against ntnt certification criteria;
- provenance available;
- dependency audit available;
- namespace not `std/*` unless promoted.

### Organization extension

Requirements:

- signed by organization authority;
- app policy explicitly allows organization authority;
- suitable for internal binaries;
- can be distributed publicly or kept internal.

### Community extension

Requirements:

- signed by publisher;
- namespace scoped to publisher;
- not trusted by default for production apps unless app policy allows it.

### Local-dev extension

Requirements:

- allowed only with explicit dev policy;
- never accepted by `trust = "official-or-org"`;
- `ntnt run` should warn loudly in production mode.

---

## CLI Surface

### `ntnt modules`

Lists compiled modules.

```bash
ntnt modules
ntnt modules --json
ntnt modules --trust
ntnt modules --capabilities
```

### `ntnt verify`

Verifies binary against signatures/provenance when available.

```bash
ntnt verify
ntnt verify --binary ./ntnt
ntnt verify --require official
ntnt verify --require-no-unsigned-extensions
ntnt verify --require-module std/netmon
```

### `ntnt doctor`

Includes extension diagnostics:

- missing required modules;
- unsigned/local-dev extensions;
- stale trust roots;
- provenance unavailable;
- capability mismatch.

### `ntnt dist build`

Optional future build helper:

```bash
ntnt dist build ntnt-dist.toml
ntnt dist verify target/release/ntnt
```

This could start as scripts/CI templates before becoming a first-class CLI command.

---

## Runtime API Surface

Possible `std/runtime` functions:

```ntnt
import {
    build_info,
    modules,
    module_info,
    verify_requirements,
    require_modules,
    require_official_build,
    require_no_unsigned_extensions,
} from "std/runtime"
```

Proposed signatures:

```ntnt
build_info() -> Map
modules() -> Array<Map>
module_info(id: String) -> Option<Map>
verify_requirements(policy: Map) -> Result<Map, String>
require_modules(ids: Array<String>) -> Result<Unit, String>
require_official_build(opts?: Map) -> Result<Unit, String>
require_no_unsigned_extensions() -> Result<Unit, String>
```

Use cases:

- fail early in app startup;
- expose safe diagnostics in health endpoints;
- test that CI used the expected binary;
- let agents inspect capabilities before modifying app code.

---

## Missing Module UX

Missing module errors should distinguish:

1. Unknown namespace/path.
2. Known official extension not included in this binary.
3. Module present but disallowed by app policy.
4. Module present but unsigned/untrusted.
5. Version mismatch.

Example:

```text
Import failed: std/netmon is an official extension module, but this ntnt binary does not include it.

Current distribution: ntnt-community 0.4.10
Available modules: run `ntnt modules`
Required by app: ntnt.toml [requires.modules]
Fix: use a distribution that includes std/netmon, or remove the import.
```

---

## Relationship to `libs()` and File Modules

`libs()` and file imports are still useful for app-local code:

```ntnt
libs("lib/")
import helpers from "./lib/helpers.tnt"
```

But compiled extension libraries are different:

| Capability | App file/libs | Compiled extension library |
|---|---|---|
| Written in `.tnt` | yes | no, Rust-native |
| Native OS/protocol access | limited | yes |
| Typechecker/docs first-class | partial | yes |
| Included in binary manifest | no | yes |
| Signed/provenance attached | app/deploy dependent | yes |
| Suitable for SNMP/raw network | no | yes |

File modules should not become the extension ecosystem. They are app code.

---

## Relationship to DD-047 `std/netmon`

DD-047 currently frames `std/netmon` as a candidate advanced network monitoring library. DD-062 generalizes the extension mechanism needed for `std/netmon` and future modules.

`std/netmon` should probably become the first major test case for official optional extension modules because it has exactly the right properties:

- useful and high-leverage;
- too specialized/heavy for every binary;
- needs native Rust implementation;
- benefits from stdlib-grade typechecking/docs;
- has security-sensitive capabilities (`network.udp`, `network.private`, `env.read`).

---

## Implementation Plan

### Phase 0: Design and threat model

- [x] Draft DD-062.
- [ ] Review with Josh.
- [ ] Decide namespace policy: `std/*` official-only, `org/*`, `x/*`/community.
- [ ] Decide whether `std/runtime` is the right module for build/module introspection.
- [ ] Decide initial signing/provenance toolchain: Sigstore/cosign vs minisign plus TUF-style root.

### Phase 1: Shared module registry foundation

- [ ] Add `NtntModuleSpec` and `ModuleTier`.
- [ ] Move runtime stdlib registration through module specs.
- [ ] Move typechecker signatures toward module specs.
- [ ] Move docs metadata toward module specs or generated registry views.
- [ ] Add `ntnt modules` and `ntnt modules --json`.
- [ ] Add tests proving existing stdlib imports behave unchanged.

### Phase 2: App requirement checks

- [ ] Add `ntnt.toml` `[requires]` parsing or extend existing project config if present.
- [ ] Implement module requirement validation in `ntnt check` and `ntnt run` startup.
- [ ] Add missing-module error categories.
- [ ] Add tests for missing module, version mismatch, trust mismatch, unsigned extension mismatch.

### Phase 3: Embedded module-universe manifest

- [ ] Generate build-time module universe manifest.
- [ ] Embed manifest in binary.
- [ ] Expose via CLI and `std/runtime`.
- [ ] Include source commit, Cargo.lock digest, target, build profile, module list, capabilities, trust tiers.

### Phase 4: Extension crate hook

- [ ] Add `run_with_extensions(...)` or equivalent extension registration hook.
- [ ] Add test-only sample extension module in the repo.
- [ ] Verify runtime import, typechecking, docs, and CLI listing for the sample extension.
- [ ] Ensure public binary still builds without any extension crates.

### Phase 5: Verification and signing MVP

- [ ] Create release artifact signing process.
- [ ] Add `ntnt verify --binary PATH`.
- [ ] Publish/verify binary digest and manifest digest.
- [ ] Generate SBOM.
- [ ] Add cargo-deny/cargo-vet policy for official/certified extension builds.

### Phase 6: First real extension distribution

- [ ] Build `std/netmon` as an official optional extension candidate.
- [ ] Produce an official/organization distribution that includes it.
- [ ] Add example app requirement policy.
- [ ] Verify Net Bacon or a network-monitoring app can require and use it.

### Phase 7: Hardening

- [ ] TUF-style trust root and delegated signing authorities.
- [ ] Transparency log integration.
- [ ] Reproducible-build/corroborated-build checks.
- [ ] Standalone verifier for deploy pipelines.
- [ ] Certification workflow for community/organization extensions.

---

## Open Questions

1. Should official optional extensions use `std/*` paths, or should all optional modules use a new namespace such as `ext/*`?
   - Current recommendation: official optional extensions may use `std/*`; community/org modules may not.

2. Should `std/runtime` exist, or should build/module inspection be CLI-only plus a smaller builtin?
   - Current recommendation: add `std/runtime`; apps need startup checks and health diagnostics.

3. Should org distributions be allowed to place organization-local modules under `std/*`?
   - Current recommendation: no, unless the module is formally accepted as official. Use `org/<org>/*` to avoid confusing portability.

4. What is the first signing stack?
   - Current recommendation: start simple with signed release artifacts + manifest digest; design toward Sigstore/cosign + SLSA/in-toto + TUF-style delegation.

5. Can native Rust extension capabilities be enforced?
   - Current answer: not fully in v1. Capabilities are declared and policy-checked, not sandbox-enforced. For true sandboxing, investigate WASM component extensions later.

6. Should dynamic extension loading ever be supported?
   - Current recommendation: not for native code in v1. Consider signed WASM components later for lower-risk dynamic extensions.

---

## Acceptance Criteria

A successful first implementation should make all of these true:

1. Public/default ntnt builds exactly as before, with no extension crates required.
2. A separate distribution can compile in an extension module.
3. Extension modules import like stdlib modules when present.
4. Runtime, typechecker, docs, and `ntnt modules` agree on available modules.
5. Missing extension modules produce clear errors.
6. Apps can declare required modules and fail before execution when the binary is wrong.
7. The binary exposes a module-universe manifest.
8. Official builds can be externally verified against a signed digest/provenance trail.
9. Unsigned/local-dev extensions are impossible to hide from diagnostics.
10. Community/org extensions cannot silently impersonate official `std/*` modules.

---

## Recommended Decision

Proceed with **Secure Compiled Extension Libraries**:

- no runtime package manager;
- no sidecar runtime for stdlib-grade APIs;
- no dynamic native plugin ABI in v1;
- compiled Rust extensions selected by distribution manifest;
- official optional extensions may live under `std/*`;
- community/org modules use explicit scoped namespaces;
- every binary declares a signed module universe;
- apps can require module/trust/capability policy before execution.

This gives ntnt a way to grow without becoming another ambient dependency ecosystem. It preserves the “batteries included” soul while allowing bigger batteries, specialized batteries, and the occasional suspiciously expensive enterprise battery — all labeled, signed, and unable to sneak into the flashlight at runtime.
