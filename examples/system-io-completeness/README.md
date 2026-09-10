# System I/O completion

Run `ntnt run examples/system-io-completeness/run.tnt` for binary crypto,
monotonic deadlines and an owned temporary file that is atomically replaced,
read and explicitly cleaned up. It uses `sync:false` for portable atomic visibility;
Unix callers can request file/parent durability by omitting that option.

Native checks:

```text
ntnt lint --strict examples/system-io-completeness
ntnt intent lint examples/system-io-completeness/system.intent
ntnt intent check examples/system-io-completeness/checks.tnt --intent examples/system-io-completeness/system.intent
```

`tcp.tnt` is an opt-in Normal-mode loopback fixture, exercised by
`normal_mode_tcp_example_exercises_coalesced_and_fragmented_binary_frames` in
`tests/system_io_completeness_tests.rs`. The Rust peer supplies coalesced and
fragmented non-UTF8 messages. Ordinary runs only print instructions. TCP authority
is intentionally denied in native Intent observers and non-Normal modes.

Temporary resources share identity, reject serialization/task transfer, and require
trusted paths. Explicit close is preferred; Drop/runtime shutdown are best-effort,
and crashes can leave paths behind. Unix creation is private under umask; other
platforms follow OS ACL rules. A failed consuming cleanup remains a terminal error.
TCP readers own the read side until close or last-reader Drop closes the socket;
errors preserve buffered bytes for a smaller exact read. The 16 MiB runtime buffer
budget excludes arrays already returned to callers. New Base64 decoders bound
payload bytes; interpreter arrays use more heap memory.
