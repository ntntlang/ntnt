"""Release wiring and packaging fixture checks; never cross-build the runtime."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]


def pins():
    return dict(line.split("=", 1) for line in (ROOT / "scripts/release/armv7.env").read_text().splitlines()
                if line and not line.startswith("#"))


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        # BaseLoader deliberately preserves the YAML key `on` as a string.
        self.workflow = yaml.load((ROOT / ".github/workflows/release.yml").read_text(), Loader=yaml.BaseLoader)
        self.jobs = self.workflow["jobs"]

    def test_publish_is_tag_only_and_waits_for_all_gates(self):
        release = self.jobs["release"]
        self.assertEqual(set(release["needs"]), {"build", "docs", "armv7", "packaging-tests"})
        self.assertEqual(release["if"], "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')")
        self.assertEqual(self.workflow["permissions"]["contents"], "read")
        for name in ("build", "armv7", "packaging-tests"):
            self.assertNotIn("permissions", self.jobs[name])

    def test_artifact_upload_and_publish_have_same_arm_files(self):
        upload = next(step for step in self.jobs["armv7"]["steps"] if step.get("uses", "").startswith("actions/upload-artifact"))
        publish = next(step for step in self.jobs["release"]["steps"] if step.get("uses", "").startswith("softprops/"))
        files = {Path(line).name for line in upload["with"]["path"].splitlines()}
        self.assertEqual(files, {"ntnt-linux-armv7.tar.gz", "ntnt-linux-armv7.tar.gz.sha256", "ntnt-linux-armv7.provenance.json"})
        release_steps = "\n".join(step.get("run", "") for step in self.jobs["release"]["steps"])
        for name in files:
            full = f"artifacts/ntnt-linux-armv7/{name}"
            self.assertIn(full, publish["with"]["files"].splitlines())
            self.assertIn(f"test -f {full}", release_steps)
        self.assertIn("sha256sum -c *.sha256", release_steps)
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertEqual(publish["with"]["fail_on_unmatched_files"], "true")

    def test_new_toolchain_action_is_immutable(self):
        action = next(s["uses"] for s in self.jobs["armv7"]["steps"]
                      if s.get("uses", "").startswith("dtolnay/rust-toolchain@"))
        self.assertRegex(action, r"^dtolnay/rust-toolchain@[0-9a-f]{40}$")

    def test_original_platform_matrix_preserved(self):
        matrix = self.jobs["build"]["strategy"]["matrix"]["include"]
        self.assertEqual({item["name"]: item["target"] for item in matrix}, {
            "macos-arm64": "aarch64-apple-darwin", "linux-x64": "x86_64-unknown-linux-gnu",
            "windows-x64": "x86_64-pc-windows-msvc"})
        tests = [step["run"] for step in self.jobs["build"]["steps"] if "cargo nextest run" in step.get("run", "")]
        self.assertEqual(len(tests), 2)
        self.assertTrue(all("--retries 0" in cmd for cmd in tests))

    def test_changed_paths_trigger_release_and_docs(self):
        required = {"scripts/release/**", "tests/release/**", "install.sh", "install.ps1", ".github/workflows/release.yml"}
        self.assertTrue(required.issubset(self.workflow["on"]["pull_request"]["paths"]))
        ci = yaml.load((ROOT / ".github/workflows/ci.yml").read_text(), Loader=yaml.BaseLoader)
        step = next(step for step in ci["jobs"]["changes"]["steps"] if step.get("id") == "filter")
        filters = yaml.safe_load(step["with"]["filters"])
        self.assertTrue(required.issubset(filters["docs"]))

    def test_recipe_pins_and_dockerfile_agree(self):
        env = pins()
        dockerfile = (ROOT / "scripts/release/Dockerfile.armv7").read_text()
        self.assertIn(f"FROM {env['ARMV7_BASE_IMAGE']}", dockerfile)
        self.assertIn(env["ARMV7_OPENSSL_SHA256"], dockerfile)
        self.assertIn(f"openssl-{env['ARMV7_OPENSSL_VERSION']}.tar.gz", dockerfile)
        self.assertIn("sha256sum -c -", dockerfile)
        self.assertIn("no-shared", dockerfile)
        self.assertIn("OPENSSL_STATIC=1", dockerfile)
        self.assertEqual(env["ARMV7_RUST_VERSION"], "1.94.0")
        self.assertEqual(env["ARMV7_CROSS_VERSION"], "0.2.5")

    def test_build_smoke_precedes_package_and_is_unprivileged(self):
        driver = (ROOT / "scripts/release/build-armv7.sh").read_text()
        self.assertIn('cross "+$ARMV7_RUST_VERSION" build --release --locked --target "$ARMV7_TARGET"', driver)
        self.assertLess(driver.index("bash scripts/release/smoke-armv7.sh"), driver.index("python3 scripts/release/package-armv7.py"))
        smoke = (ROOT / "scripts/release/smoke-armv7.sh").read_text()
        self.assertIn("/usr/local/bin/qemu-arm -L /usr/arm-linux-gnueabihf", smoke)
        self.assertIn("run \"$1\" lint scripts/release/armv7-smoke.tnt", smoke)
        self.assertIn("run \"$1\" run scripts/release/armv7-smoke.tnt", smoke)
        self.assertNotIn("--privileged", driver + smoke)
        self.assertIn("--network none", smoke)


class BuildDriverTests(unittest.TestCase):
    def test_output_directory_cannot_be_redirected_to_stale_artifact(self):
        with tempfile.TemporaryDirectory(prefix="ntnt-build-driver-") as directory:
            root = Path(directory)
            release = root / "scripts/release"
            release.mkdir(parents=True)
            for name in ("build-armv7.sh", "armv7.env"):
                shutil.copyfile(ROOT / "scripts/release" / name, release / name)
            (release / "smoke-armv7.sh").write_text("#!/bin/bash\nexit 0\n")
            tools = root / "mock-bin"
            tools.mkdir()
            commands = {
                "docker": '#!/bin/sh\nif [ "$1" = image ]; then printf "sha256:fixture\\n"; fi\n',
                "rustc": '#!/bin/sh\nprintf "rustc 1.94.0 fixture\\n"\n',
                "python3": '#!/bin/sh\nexit 0\n',
                "cross": '#!/bin/sh\nif [ "$1" = --version ]; then printf "cross 0.2.5 fixture\\n"; else printf "%s" "$CARGO_TARGET_DIR" > "$CAPTURE"; fi\n',
            }
            for name, content in commands.items():
                file = tools / name
                file.write_text(content)
                file.chmod(0o755)
            capture = root / "capture"
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                       CARGO_TARGET_DIR=str(root / "foreign-output"), CAPTURE=str(capture))
            result = subprocess.run(["bash", str(release / "build-armv7.sh")], env=env,
                                    text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(capture.read_text(), str(root / "target"))


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ntnt-package-fixture-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        shutil.copytree(ROOT / "scripts/release", self.root / "scripts/release")
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.5.4"\n')
        (self.root / "Cargo.lock").write_text("# fixture lockfile; not an NTNT build\n")
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.git("add", "scripts", "Cargo.toml", "Cargo.lock")
        self.git("-c", "user.email=fixture@example.invalid", "-c", "user.name=Fixture", "commit", "-qm", "fixture")
        self.binary = self.root / "target/armv7-unknown-linux-gnueabihf/release/ntnt"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b"explicit packaging fixture, not a runtime binary")
        self.binary.chmod(0o755)
        (self.root / "dist").mkdir()
        (self.root / "dist/armv7-elf.txt").write_text(" 0x (NEEDED) Shared library: [libc.so.6]\n")
        self.env = dict(os.environ, **pins(), ARMV7_BUILD_IMAGE_ID="sha256:" + "a" * 64,
                        ARMV7_RUSTC="fixture rustc 1.94.0", GITHUB_REF="refs/tags/v0.5.4")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True).strip()

    def package(self):
        return subprocess.run([sys.executable, str(self.root / "scripts/release/package-armv7.py")],
                              env=self.env, text=True, capture_output=True)

    def test_archive_checksum_license_and_provenance(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        dist = self.root / "dist"
        archive = dist / "ntnt-linux-armv7.tar.gz"
        with tarfile.open(archive) as tar:
            self.assertEqual(set(tar.getnames()), {"ntnt", "LICENSE.openssl"})
            binary_file = tar.extractfile("ntnt")
            license_file = tar.extractfile("LICENSE.openssl")
            assert binary_file is not None and license_file is not None
            self.assertEqual(binary_file.read(), self.binary.read_bytes())
            self.assertIn(b"Apache License", license_file.read())
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.assertEqual((dist / (archive.name + ".sha256")).read_text(), f"{digest}  {archive.name}\n")
        manifest = json.loads((dist / "ntnt-linux-armv7.provenance.json").read_text())
        self.assertEqual(manifest["sha256"], digest)
        self.assertEqual(manifest["source_sha"], self.git("rev-parse", "HEAD"))
        self.assertEqual(manifest["elf_needed"], ["libc.so.6"])
        self.assertEqual(manifest["build_image_id"], self.env["ARMV7_BUILD_IMAGE_ID"])
        self.assertEqual(manifest["openssl"]["source_sha256"], pins()["ARMV7_OPENSSL_SHA256"])
        self.assertEqual(manifest["cargo_lock_sha256"], hashlib.sha256((self.root / "Cargo.lock").read_bytes()).hexdigest())

    def test_tag_version_mismatch_rejected(self):
        self.env["GITHUB_REF"] = "refs/tags/v0.5.5"
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Release tag disagrees", result.stderr)

    def test_dirty_source_rejected(self):
        (self.root / "Cargo.lock").write_text("modified fixture")
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dirty tracked checkout", result.stderr)


if __name__ == "__main__":
    unittest.main()
