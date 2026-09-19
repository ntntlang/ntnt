#!/usr/bin/env python3
"""Package the already-built, QEMU-smoked ARMv7 executable (no build here)."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    root = Path(__file__).resolve().parents[2]
    os.chdir(root)
    env = os.environ
    target = env["ARMV7_TARGET"]
    binary = Path("target") / target / "release/ntnt"
    source_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    # A manifest identifies the exact checkout, not only a claimed CI SHA.
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], text=True):
        raise SystemExit("Refusing provenance for a dirty tracked checkout")
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    ref = env.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/") and ref != f"refs/tags/v{version}":
        raise SystemExit("Release tag disagrees with Cargo.toml version")
    dist = Path("dist")
    dist.mkdir(exist_ok=True)
    archive = dist / "ntnt-linux-armv7.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(binary, arcname="ntnt", recursive=False)
        tar.add("scripts/release/OPENSSL-LICENSE.txt", arcname="LICENSE.openssl", recursive=False)
    digest = sha256(archive)
    (dist / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n")
    elf = (dist / "armv7-elf.txt").read_text()
    manifest = {
        "schema_version": 1,
        "artifact": archive.name,
        "sha256": digest,
        "binary_sha256": sha256(binary),
        "version": version,
        "source_repository": "https://github.com/ntntlang/ntnt",
        "source_sha": source_sha,
        "cargo_lock_sha256": sha256(Path("Cargo.lock")),
        "target": target,
        "rust_toolchain": env["ARMV7_RUST_VERSION"],
        "rustc_verbose": env["ARMV7_RUSTC"],
        "cross_version": env["ARMV7_CROSS_VERSION"],
        "base_image": env["ARMV7_BASE_IMAGE"],
        "build_image_id": env["ARMV7_BUILD_IMAGE_ID"],
        "dockerfile_sha256": sha256(Path("scripts/release/Dockerfile.armv7")),
        "openssl": {
            "version": env["ARMV7_OPENSSL_VERSION"],
            "source_sha256": env["ARMV7_OPENSSL_SHA256"],
            "linkage": "static",
            "license": "Apache-2.0",
            "license_file": "LICENSE.openssl",
        },
        "build_command": f"cross +{env['ARMV7_RUST_VERSION']} build --release --locked --target {target}",
        "elf_interpreter": "/lib/ld-linux-armhf.so.3",
        "elf_needed": re.findall(r"\(NEEDED\).*?\[(.*?)\]", elf),
        "smoke": "direct qemu-arm in build image: --version, lint, interpreter/SQLite/SHA256 assertions",
    }
    (dist / "ntnt-linux-armv7.provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Packaged {archive}: {digest}")


if __name__ == "__main__":
    main()
