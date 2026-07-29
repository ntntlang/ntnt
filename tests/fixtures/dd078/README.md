# DD-078 reviewed section snapshots

These fixtures apply two review locks:

1. Exact section snapshots preserve DD-078's highest-risk scope and deletion-safety contracts:
   - project-neutral core acceptance criteria;
   - project-neutral core definition of done;
   - Larrimon adoption Slice 16M migration compatibility and exclusive deletion authority;
   - Larrimon consumer definition of done.
2. Canonical SHA-256 fixtures lock the complete normative envelopes of the core design, core implementation plan, and standalone Larrimon adoption plan. This prevents equivalent requirements or authority exceptions from being inserted immediately outside the named sections.

`tests/dd078_plan_tests.rs` compares section source byte-for-byte after normalizing line endings and verifies each complete document's SHA-256 over canonical LF bytes. A source edit therefore fails until the matching fixture is deliberately updated in the same review. Do not regenerate snapshots or digests merely to make a test green: inspect the source and fixture diffs and confirm that core scope remains project-neutral and consumer migration/deletion safeguards remain equivalent or stronger.
