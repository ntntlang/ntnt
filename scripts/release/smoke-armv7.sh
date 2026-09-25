#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
source scripts/release/armv7.env
IMAGE=${1:?Usage: smoke-armv7.sh BUILD_IMAGE}
IMAGE=$(docker image inspect "$IMAGE" --format '{{.Id}}')
BINARY="target/$ARMV7_TARGET/release/ntnt"
VERSION=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')
mkdir -p dist
# Capture actual ELF dependencies (not an assumed runtime-library list).
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --volume "$PWD:/work:ro" --workdir /work --entrypoint bash "$IMAGE" -eu -c '
    arm-linux-gnueabihf-readelf -h -l -d "$1"
' -- "$BINARY" > dist/armv7-elf.txt
grep -q 'Machine:.*ARM' dist/armv7-elf.txt
grep -q 'hard-float ABI' dist/armv7-elf.txt
grep -q '/lib/ld-linux-armhf.so.3' dist/armv7-elf.txt
# Vendored OpenSSL is linked statically; shipping an accidental libssl.so
# dependency would break on current Raspberry Pi OS.
if grep -E 'NEEDED.*lib(ssl|crypto)\.so' dist/armv7-elf.txt; then
    printf 'Unexpected dynamic OpenSSL dependency\n' >&2
    exit 1
fi
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --volume "$PWD:/work:ro" --tmpfs /tmp --workdir /work --entrypoint bash "$IMAGE" -eu -o pipefail -c '
    run() { timeout --kill-after=5 90 /usr/local/bin/qemu-arm -L /usr/arm-linux-gnueabihf "$1" "${@:2}"; }
    test "$(run "$1" --version)" = "ntnt $2"
    run "$1" lint scripts/release/armv7-smoke.tnt
    run "$1" run scripts/release/armv7-smoke.tnt | tee /tmp/smoke.txt
    grep -qx ARMV7_SMOKE_OK /tmp/smoke.txt
' -- "$BINARY" "$VERSION"
