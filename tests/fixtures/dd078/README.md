# DD-078 reviewed section snapshots

These fixtures lock the exact reviewed text of DD-078's highest-risk scope and deletion-safety contracts:

- project-neutral core acceptance criteria;
- project-neutral core definition of done;
- Larrimon adoption Slice 16M migration compatibility and exclusive deletion authority;
- Larrimon consumer definition of done.

`tests/dd078_plan_tests.rs` compares the source sections byte-for-byte after trimming only outer whitespace. A source edit therefore fails until the matching fixture is deliberately updated in the same review. Do not regenerate these snapshots merely to make a test green: inspect the source/fixture diff and confirm that core scope remains project-neutral and consumer migration/deletion safeguards remain equivalent or stronger.
