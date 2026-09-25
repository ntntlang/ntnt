# Release builds and pinned installation

## Unix installer

`install.sh` downloads release binaries for Linux x86-64, Linux ARMv7 hard-float
(`uname -m` = `armv7l`), and macOS ARM64. Windows x64 remains available through
`install.ps1` and the existing Windows release job; the options below are for the
Bash installer only.

```sh
# Latest release (resolved once)
curl -fsSL https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash

# Explicit release: accepts vX.Y.Z or X.Y.Z
curl -fsSL https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh \
  | bash -s -- --version v0.5.4 --no-starter-kit

# Equivalent when running a downloaded installer
NTNT_VERSION=v0.5.4 bash install.sh --no-starter-kit
```

The explicit version example illustrates selection, not an assertion that older
releases have every architecture asset. ARMv7 installation requires a release
containing `ntnt-linux-armv7.tar.gz` and its `.sha256` sidecar. Do not move an
existing tag just to add this build recipe. Without `--version` or `NTNT_VERSION`,
the installer resolves GitHub's latest release; the command-line option takes
precedence over the environment variable.

Every binary archive must pass SHA256 verification before extraction. The
candidate must run and report the selected version before replacing an existing
`~/.local/bin/ntnt`. On unsupported architectures, missing assets/checksums,
corruption, or execution failure, installation stops. It does **not** install
Rust, clone/reset a working tree, or silently compile `main`. Build your chosen
tag manually if there is no suitable release binary. Checksums detect corruption;
they are fetched from the same release and are not an independent signature.

Unless `--no-starter-kit` is set, docs/examples/agent guides are fetched from the
**same version tag**. An existing `./ntnt` is never overwritten; starter-kit
failure is nonfatal after a successful binary install. This optional GitHub source
archive is not covered by the binary checksum. The ARM OpenSSL license is retained
at `~/.local/share/licenses/ntnt/LICENSE.openssl`.

## ARMv7 build recipe

The separate `armv7` release job uses these pins from
`scripts/release/armv7.env`:

- Target: `armv7-unknown-linux-gnueabihf` (32-bit ARMv7, hard-float GNU/Linux).
- Rust: **1.94.0**; cross: **0.2.5**; Cargo uses `--release --locked`.
- Base image:
  `ghcr.io/cross-rs/armv7-unknown-linux-gnueabihf@sha256:4a08bebb60f08d52c1885b643a768f163c7318c05f91cf119f6c02855ea52699`.
- OpenSSL **3.5.6**, official source archive SHA256
  `deae7c80cba99c4b4f940ecadb3c3338b13cb77418409238e57d7f31f2a3b736`.

The base image lacks target OpenSSL. `scripts/release/Dockerfile.armv7` builds the
checksum-pinned official source with `linux-armv4`, the ARM hard-float cross
compiler, and `no-shared no-tests`. `OPENSSL_DIR=/opt/openssl-armv7` and
`OPENSSL_STATIC=1` select it without changing NTNT dependencies or `Cargo.lock`.
OpenSSL is statically linked; its Apache-2.0 license, copied from the upstream
`openssl-3.5.6` tag, is included as `LICENSE.openssl` in the release archive.
**System OpenSSL updates do not update this embedded copy.** Security updates
require updating the version/checksum/Dockerfile/license as applicable, rebuilding,
and publishing a new NTNT release.

With Docker, Python 3.11+, the pinned toolchain/target, and cross installed, run:

```sh
bash scripts/release/build-armv7.sh
```

The driver rebuilds the pinned Dockerfile, resolves its local immutable image ID,
and uses that ID for cross-build, ELF inspection and QEMU execution. It uses two
Cargo jobs by default (`CARGO_BUILD_JOBS` can override). It does not publish an
image, push a tag, or publish a release. No privileged container or host binfmt
registration is needed.

The smoke gate directly runs the image's `/usr/local/bin/qemu-arm` with its
`/usr/arm-linux-gnueabihf` loader prefix. It checks `--version`, lints the smoke
fixture, then executes assertions covering interpretation, SQLite, and SHA256.
Each invocation has a 90-second timeout plus a five-second forced-kill grace
period. The container has no network and a read-only checkout. This is executable
smoke coverage, **not** a replacement for the native release tests or a claim that
all ARM functions (network, process creation, hardware) are covered.

ELF checks require ARM hard-float and `/lib/ld-linux-armhf.so.3`, and reject dynamic
`libssl`/`libcrypto` dependencies. The provenance manifest records the actual
`NEEDED` libraries; compatible glibc, loader and libgcc must be provided by the
destination. TLS use also requires system CA certificates. This is not an ARMv6,
musl/Alpine, or 64-bit ARM build. A physical Pi test remains separate from the
repeatable QEMU CI gate.

Artifacts in `dist/`:

- `ntnt-linux-armv7.tar.gz` — executable plus OpenSSL license.
- `ntnt-linux-armv7.tar.gz.sha256` — archive checksum.
- `ntnt-linux-armv7.provenance.json` — source commit, Cargo.lock hash, archive/binary
  hashes, toolchain, cross version, base image digest, actual local build image ID,
  Dockerfile hash, OpenSSL provenance, and ELF dependencies.

The manifest describes provenance; it is not a signed attestation or a claim of
bit-for-bit reproducibility. Packaging refuses dirty tracked source and a release
tag that disagrees with `Cargo.toml`. The existing Linux x64/macOS ARM64/Windows
x64 native jobs are unchanged. Publishing is tag-push-only and waits for all native
builds, docs, ARMv7 build/smoke, and packaging tests; it requires the ARM archive,
checksum and manifest and verifies archive checksums before publication. Pull
requests on packaging/installer paths exercise the jobs without publishing.

## Fast, offline fixture checks

Requires Python 3.11+ and PyYAML 6.0.3 (install in a virtualenv if needed):

```sh
python3 -m unittest discover -s tests/release -v
bash -n install.sh scripts/release/*.sh
```

These use mock download/uname commands and temporary homes, local fixture
archives, and an isolated fixture git repository. They never install NTNT into the
user's real home, perform a real download, build the runtime, or publish anything.
Passing them does not establish that a new cross-build or physical-Pi run passed;
those need their own execution evidence.
