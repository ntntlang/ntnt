# DD-078 Larrimon audit baseline

This appendix makes DD-078's first external reference-adoption facts reproducible. Larrimon is a demanding validation corpus for generalized ntnt mechanisms, not the scope or naming source for those mechanisms. This is an input baseline, not permission to delete files and not a substitute for the invariant/mutation ledger required before each migration deletion. Other adopters produce an equivalent appendix bound to their own repository, inventory, and invariants.

- Repository: `https://github.com/larimonious/larrimon.git`
- Commit: `ceadfd992d1435ac27afb054968ff5569d697ce1`
- Audited branch label: `feat/host-check-management`
- Audit rule: committed bytes from the commit above only; concurrent dirty-worktree changes were excluded.
- Shell/Python/JavaScript/MJS extension inventory digest: `sha256:403ed54624a6d99cfe5b05a08f966ffaeb8d73eba20e05dfddff6aacbce3f253`
- SQL-only test inventory digest: `sha256:0e7bbaa9ddbd064a75eb4d55ff1dcee499c4564acb14a1fb8658ce835620f998`
- Intent inventory digest: `sha256:84dc5b6150950056bb3f56485f8dc7438b923d59ce140752e6630aaef3d015b7`

Every digest is SHA-256 over UTF-8 rows in the exact form `path<TAB>line_count<TAB>git_blob_id<LF>`, sorted bytewise by Git tree path, with exactly one LF per row. `line_count` is LF count plus one only when non-empty content lacks a terminal LF. The extension digest includes every committed `.sh`, `.py`, `.js`, and `.mjs` path; the SQL-only digest includes committed `tests/*.sql`; the Intent digest includes every committed `*.intent`. Git tree paths and blob IDs are emitted by Git; duplicate paths are impossible in one tree and a non-UTF-8 path makes regeneration fail closed rather than substitute a lossy name. A rebase or changed Larrimon base invalidates all facts and requires a regenerated appendix and protected contract before a deletion gate.

## Reproducible findings

- 7 Intent files: 20 `Feature:` entries, 27 `Scenario:` entries, and 38 `→` outcome/assertion lines.
- 5 Intent files have no scenario: `jobs/run_probe.intent`, `jobs/schedule_due_checks.intent`, `lib/auth.intent`, `lib/settings.intent`, and `routes/users.intent`.
- Committed `.tnt` source contains 37 `@implements` and 0 `@supports` annotations.
- `tests/intent.sh` contains 18 direct `ntnt run tests/...` invocations and 0 `ntnt intent check` invocations.
- 14 shell files plus 11 Python files are project-owned migration, orchestration, fixture, policy, or test/support programs and must be replaced.
- 2 project-owned JavaScript/MJS test programs must be replaced. Product JavaScript and vendored HTMX remain product assets.
- 3 SQL-only test programs/fixtures contain 400 lines and are executable inputs to `tests/integration.sh`; Task 16 replaces them with typed PostgreSQL fixtures/assertions.
- Those 30 replacement files contain 4,549 committed lines. The audited `tests/` executable/spec set (`.sh`, `.py`, `.js`, `.mjs`, `.sql`, `.tnt`, `.intent`) contains 4,935 lines; non-test production `.tnt` contains 4,827 lines.

## Intent files

| Path | Lines | Git blob |
|---|---:|---|
| `jobs/run_probe.intent` | 26 | `73bcab32dadbfddfe80c001c185f47d12652e352` |
| `jobs/schedule_due_checks.intent` | 14 | `10dcb601c270ab03a708d6c328062a1018e61bf2` |
| `lib/auth.intent` | 13 | `67ff2100080c7e69c246298e3e1cbfc5883f9231` |
| `lib/settings.intent` | 13 | `519e6c189e636899072f891db4e7a9ab0e65beee` |
| `routes/users.intent` | 6 | `af85fdba78609986874af4b3e6b013b3fbe03748` |
| `server.intent` | 168 | `9c6b04ed72a364a052187091c9320de87b2df40f` |
| `tests/public_http.intent` | 46 | `8f34d8c53f28d3620a689ab21de64445e794cb67` |

## Project-owned support/test replacement inventory

`Range` is the full committed file range. The later invariant ledger must split these ranges into stable behavioral invariants, replacement obligation/case IDs, environment/resources, positive results, and mutation/fault witnesses.

| Path | Range | Lines | Git blob | Required destination |
|---|---:|---:|---|---|
| `scripts/check-migration-checksums.py` | 1–59 | 59 | `02b9297b8b5e7f42229def5fdbc2f2bd3a6013d9` | DD-077 PRs 1B–1C, DD-078 Task 8 and Slice 16M |
| `scripts/dev-down.sh` | 1–5 | 5 | `aef22fa45c331529b583f0a2ab9c632c27b3ee70` | Slices 14C–14D typed project environment lifecycle |
| `scripts/dev-up.sh` | 1–8 | 8 | `eccf803b8cbb7eb91fad8d9d32b2ced54e3cc962` | Slices 14C–14D plus DD-077 migration runner |
| `scripts/migrate-prod.sh` | 1–136 | 136 | `fa46418cc580d64e18e4e3d3c4e3705510ee2e05` | DD-077 PRs 1B–1C and DD-078 Slice 16M |
| `scripts/migrate.sh` | 1–26 | 26 | `840f6c602c40e8625e63fa07e4f0157cede53051` | DD-077 PRs 1B–1C and DD-078 Slice 16M |
| `scripts/staging-down.sh` | 1–19 | 19 | `a5f58cde88dc13942c38cbc74a8df17bfdc48e32` | Slices 14C–14D ownership-safe teardown |
| `scripts/staging-smoke.sh` | 1–64 | 64 | `1a856607ceb913bc2d1949a6550b7cb17c830cbb` | Stateful HTTP/browser verification and environment profile |
| `scripts/staging-state.py` | 1–215 | 215 | `5f51983508bb622fc662f9341d28375cad1e4b9b` | Slice 14C typed project state/lease plus 14D OCI allocation API |
| `scripts/staging-up.sh` | 1–40 | 40 | `41202e958db829c10d18db2d7a0c5976b3bd7375` | Slices 14C–14D typed OCI environment lifecycle |
| `tests/all.sh` | 1–15 | 15 | `dbcae166b36185e8a3a314033c1cda40b115ea55` | `ntnt.toml` profiles and direct ntnt CI |
| `tests/architecture_cases.py` | 1–97 | 97 | `478eda515b93907909d08e4557a8863c7430c587` | `std/test/project` facts and `.tnt` constraints |
| `tests/assets_provenance.py` | 1–11 | 11 | `362f3d66aac906029d7bd3f09d728daa8f4e30cc` | Git/project provenance facts and `.tnt` constraints |
| `tests/ci_cases.py` | 1–9 | 9 | `f9e628e58351ba0505f1d79f0f610da0b7920f61` | Workflow/project facts and `.tnt` constraints |
| `tests/db.sh` | 1–6 | 6 | `517f1ac220ba0efacad4c3d3a3efbcb3d9bfc6d9` | Full database profile |
| `tests/fast.sh` | 1–17 | 17 | `8b83e0a1544310b0f1b6b4602a0ba1675dae6626` | Fast profile |
| `tests/integration.sh` | 1–1810 | 1810 | `7245afe663c8f4308169e0a22b857f6dbce4c0f0` | HTTP/process/DB/job/concurrency provider and `.tnt` slices |
| `tests/intent.sh` | 1–115 | 115 | `0965c489f91f320269be4ebf11fb34a2f4be44fb` | Native Intent planner/executor and linked `.tnt` cases |
| `tests/migrate_prod_integration.sh` | 1–207 | 207 | `613216620b748778b0b4e3b523eff2c374712cc6` | DD-077 PRs 1B–1C and DD-078 Slice 16M |
| `tests/reconciliation_cases.js` | 1–503 | 503 | `98d342df7105e3e13ad7f017c4a564e3440e61e4` | Sandboxed browser `.tnt` cases |
| `tests/redirect_mock.py` | 1–48 | 48 | `65c5ed780489904fe543796a760b449b5bfbeb97` | Built-in scripted HTTP fixture |
| `tests/resend_mock.py` | 1–40 | 40 | `47b2aabc34fa035482f7d4e9d9e791f4dc2af42b` | Built-in scripted HTTP fixture |
| `tests/runtime_image_provenance.py` | 1–50 | 50 | `7c23146f2ce68199d0b092d44881c24c8030a6f0` | OCI/image provenance facts and `.tnt` constraints |
| `tests/runtime_provenance.py` | 1–62 | 62 | `077a6af4ae4acccf626c9720485f687d4099dc1d` | Runtime/Git provenance facts and `.tnt` constraints |
| `tests/server-smoke.sh` | 1–6 | 6 | `fb8141ccaca69799e9ae13e901c5b3736b70c46e` | Managed app readiness/HTTP case |
| `tests/smtp_mock.py` | 1–72 | 72 | `da90d8c0cf1e779b1ee7183bc0d4b52dc7a5d031` | Built-in SMTP capture fixture |
| `tests/staging-browser-smoke.mjs` | 1–289 | 289 | `2eaa9db6c0e2fb6597d5a00e27ba25f74af5e276` | Sandboxed browser `.tnt` cases |
| `tests/staging_state_cases.py` | 1–220 | 220 | `81132144b791cca3b2a8927543c547989cef4410` | Slices 14C–14D state/allocation conformance plus `.tnt` cases |

## SQL-only test replacement inventory

These are application/schema verification inputs, not production migration-runner fixtures. Task 16 deletes them after same-revision positive, negative, race, cleanup, and mutation parity; Slice 16M does not hold them.

| Path | Range | Lines | Git blob | Required destination |
|---|---:|---:|---|---|
| `tests/assertions.sql` | 1–96 | 96 | `a5959ce6a090b8c6f4f85f7f5add903a41cb98a0` | Task 16 typed PostgreSQL assertions |
| `tests/probe_run_state_fixture.sql` | 1–206 | 206 | `cfc691bbb28f9a7be3d3d8193caec1c3bce82b8e` | Task 16 typed committed seed/fixture API |
| `tests/security_definer_tenant_case.sql` | 1–98 | 98 | `a7934134fec0244143322c069291d23f4a56dda4` | Task 16 role/RLS/security-definer `.tnt` cases |

## Retained product assets

These files are in the extension inventory but are not verification/support programs and are not deletion targets.

| Path | Range | Lines | Git blob | Classification |
|---|---:|---:|---|---|
| `public/larrimon.js` | 1–733 | 733 | `f96088c41f6f2d84905c303456ea2621d42612ec` | project-owned production asset |
| `public/vendor/htmx-2.0.10.min.js` | 1–1 | 1 | `3b7ac1aceb211ca716c7a9c5774c649f74331ee1` | vendored production asset; immutable origin/digest classification required |

## Regeneration gate

Before any Larrimon migration/deletion branch:

1. resolve and record the exact candidate base commit;
2. regenerate the extension, SQL-only test, and Intent canonical inventories from committed bytes;
3. update every changed full-file range and Git blob ID;
4. regenerate the protected contract from the same base;
5. expand affected files into invariant-level rows before deletion;
6. fail closed if the worktree, contract base, inventory base, or report base differ.
