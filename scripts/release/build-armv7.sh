#!/usr/bin/env bash
# Run from a clean checkout with Docker, cross and the pinned Rust toolchain.
set -euo pipefail
cd "$(dirname "$0")/../.."
source scripts/release/armv7.env
[[ "$(cross --version)" == "cross $ARMV7_CROSS_VERSION"* ]]
rustc "+$ARMV7_RUST_VERSION" --version
IMAGE=ntnt-release-armv7:local
docker build --file scripts/release/Dockerfile.armv7 --tag "$IMAGE" scripts/release
IMAGE=$(docker image inspect "$IMAGE" --format '{{.Id}}')
export CROSS_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_IMAGE="$IMAGE"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
# Build, smoke and packaging must consume the same output tree, even when the
# caller or global Cargo config selects another directory.
export CARGO_TARGET_DIR="$PWD/target"
cross "+$ARMV7_RUST_VERSION" build --release --locked --target "$ARMV7_TARGET"
# Direct QEMU execution: no binfmt registration and no privileged container.
bash scripts/release/smoke-armv7.sh "$IMAGE"
export ARMV7_BUILD_IMAGE_ID
ARMV7_BUILD_IMAGE_ID="$IMAGE"
export ARMV7_RUSTC
ARMV7_RUSTC=$(rustc "+$ARMV7_RUST_VERSION" --version --verbose)
set -a
source scripts/release/armv7.env
set +a
python3 scripts/release/package-armv7.py
