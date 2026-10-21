# eMudhra TLS distrust fixtures

Public certificates from the October 2026 eMudhra intermediate-key incident:

- Chains: <https://emudhra.we-have-a-problem.com/chains.zip>
  - SHA-256: `8a2edc81b6c51e3ad27d45ca1e441ec84c961e2dee0fc547f2a02c0f74315f95`
- Original roots: <https://emudhra.we-have-a-problem.com/roots.json>
  - SHA-256: `119f32cd81a6b5a4e3c4c2df5fe07daa90dc43cafa123a62f0394f1895d92380`
- Advisory: <https://emudhra.we-have-a-problem.com/>
- Verifier fix: <https://github.com/rustls/webpki/pull/546>

The chain archive identifies its capture date as 20 October 2026. The twelve
`*-emudhra.we-have-a-problem.com.pem` files are its leaf-first chains, preserved
apart from adding a final newline. They cover all six affected TLS-issuing
intermediate keys (DV/OV/EV, G3B/G3C), each through the emSign chain and through
the AAA Certificate Services cross-sign. The other two compromised keys were
for code signing, not TLS issuance.

`emsign-ecc-g3-root.pem` and `aaa-root.pem` are the corresponding PEM fields
from `roots.json`. Tests deliberately trust these roots in a separate verifier
to ensure the upstream compromised-key block works even when root removal is
not the reason for rejection. They are never installed into production trust.

All fixtures contain **public certificates only**, not private keys. Tests run
entirely offline at `2026-10-20T12:00:00Z`, within the certificates' validity
periods, so future expiry cannot turn a vulnerable verifier into a false pass.
A separately generated, valid control chain must still verify successfully.

These tests cover NTNT's bundled Rust TLS verification. They do not assert
that the operating-system/native TLS store used by ordinary reqwest clients
has been updated; that remains a separate host/container deployment step.
