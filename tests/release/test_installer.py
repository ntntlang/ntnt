"""Installer integration tests: fake downloads/CPU, real tar/hash, temporary HOME only."""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ntnt-installer-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.work = self.root / "work"
        self.work.mkdir()
        self.fixtures = self.root / "fixtures"
        self.fixtures.mkdir()
        self.log = self.root / "requests"
        # Only these tools exist in the subprocess PATH. No real network, git,
        # cargo or rustup can be reached, even if fallback is accidentally added.
        for tool in ("bash", "mktemp", "rm", "sed", "tr", "tar", "gzip", "sort", "sha256sum", "shasum", "chmod", "mkdir", "cp", "mv"):
            target = shutil.which(tool)
            if target:
                (self.bin / tool).symlink_to(target)
        self.env = dict(os.environ, PATH=str(self.bin), HOME=str(self.home),
                        TMPDIR=str(self.root), FIXTURES=str(self.fixtures),
                        REQUEST_LOG=str(self.log), MOCK_OS="Linux", MOCK_ARCH="armv7l")
        self.env.pop("NTNT_VERSION", None)
        self.script("uname", '#!/bin/bash\ncase "$1" in -s) printf "%s\\n" "$MOCK_OS";; -m) printf "%s\\n" "$MOCK_ARCH";; esac\n')
        downloader = f'''#!{sys.executable}
import os, pathlib, shutil, sys
args = sys.argv[1:]
url = next(a for a in args if a.startswith("https://"))
with open(os.environ["REQUEST_LOG"], "a") as log: log.write(url + "\\n")
flag = "-o" if "-o" in args else "-O"
out = pathlib.Path(args[args.index(flag) + 1])
name = "latest.json" if url.endswith("/latest") else url.rsplit("/", 1)[1]
source = pathlib.Path(os.environ["FIXTURES"]) / name
if not source.exists(): sys.exit(22)
shutil.copyfile(source, out)
'''
        self.script("curl", downloader)
        self.script("wget", downloader)
        self.fixture()

    def script(self, name, content):
        path = self.bin / name
        path.write_text(content)
        path.chmod(0o755)

    def fixture(self, platform="linux-armv7", body=None, extra=None, license=False):
        self.archive = self.fixtures / f"ntnt-{platform}.tar.gz"
        body = body if body is not None else b'#!/bin/bash\nprintf "ntnt 0.5.4\\n"\n'
        entries = {"ntnt": body}
        if license:
            entries["LICENSE.openssl"] = b"test license fixture\n"
        entries.update(extra or {})
        with tarfile.open(self.archive, "w:gz") as tar:
            for name, data in entries.items():
                info = tarfile.TarInfo(name)
                info.mode = 0o755
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
        self.checksum = Path(str(self.archive) + ".sha256")
        self.checksum.write_text(f"{hashlib.sha256(self.archive.read_bytes()).hexdigest()}  {self.archive.name}\n")
        (self.fixtures / "latest.json").write_text(json.dumps({"tag_name": "v0.5.4"}))

    def install(self, *args, success=True):
        result = subprocess.run([str(self.bin / "bash"), str(ROOT / "install.sh"), *args],
                                cwd=self.work, env=self.env, text=True, capture_output=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def requests(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def existing_install(self):
        dest = self.home / ".local/bin/ntnt"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text("existing installation")
        return dest

    def test_explicit_arm_version_never_resolves_latest(self):
        self.fixture(license=True)
        self.install("--version", "v0.5.4", "--no-starter-kit")
        self.assertEqual(len(self.requests()), 2)
        self.assertTrue(all("/download/v0.5.4/" in url for url in self.requests()))
        self.assertTrue((self.home / ".local/bin/ntnt").is_file())
        self.assertEqual((self.home / ".local/share/licenses/ntnt/LICENSE.openssl").read_text(), "test license fixture\n")

    def test_version_env_without_v(self):
        self.env["NTNT_VERSION"] = "0.5.4"
        self.install("--no-starter-kit")
        self.assertEqual(len(self.requests()), 2)

    def test_argument_overrides_environment(self):
        self.env["NTNT_VERSION"] = "0.1.0"
        self.install("--version", "0.5.4", "--no-starter-kit")
        self.assertTrue(all("v0.5.4" in url for url in self.requests()))

    def test_latest_resolved_once(self):
        self.install("--no-starter-kit")
        self.assertEqual(sum(url.endswith("/latest") for url in self.requests()), 1)
        self.assertTrue(all("v0.5.4" in url for url in self.requests()[1:]))

    def test_existing_platforms(self):
        for os_name, arch, platform in [("Linux", "x86_64", "linux-x64"), ("Darwin", "arm64", "macos-arm64")]:
            with self.subTest(platform=platform):
                self.env.update(MOCK_OS=os_name, MOCK_ARCH=arch)
                self.fixture(platform)
                self.install("--version", "0.5.4", "--no-starter-kit")
                self.assertIn(f"ntnt-{platform}.tar.gz", self.requests()[-2])

    def test_wget_and_shasum_fallback(self):
        (self.bin / "curl").unlink()
        (self.bin / "sha256sum").unlink()
        self.install("--version", "0.5.4", "--no-starter-kit")

    def test_unsupported_target_has_no_download_or_source_fallback(self):
        self.env["MOCK_ARCH"] = "aarch64"
        result = self.install("--version", "0.5.4", success=False)
        self.assertIn("No release binary", result.stderr)
        self.assertEqual(self.requests(), [])
        self.assertFalse((self.work / "ntnt").exists())

    def test_directory_or_symlink_to_directory_destination_rejected(self):
        dest = self.home / ".local/bin/ntnt"
        dest.mkdir(parents=True)
        result = self.install("--version", "0.5.4", "--no-starter-kit", success=False)
        self.assertIn("destination is a directory", result.stderr)
        self.assertEqual(list(dest.iterdir()), [])
        dest.rmdir()
        target = self.home / "another-directory"
        target.mkdir()
        dest.symlink_to(target, target_is_directory=True)
        self.install("--version", "0.5.4", "--no-starter-kit", success=False)
        self.assertEqual(list(target.iterdir()), [])

    def test_post_install_checksum_detects_replacement_error(self):
        real_mv = shutil.which("mv")
        (self.bin / "mv").unlink()
        self.script("mv", f'#!/bin/bash\n"{real_mv}" "$@"\nprintf tampered > "$HOME/.local/bin/ntnt"\n')
        result = self.install("--version", "0.5.4", "--no-starter-kit", success=False)
        self.assertIn("Installed file checksum verification failed", result.stderr)
        self.assertNotIn("Installed ntnt", result.stdout)

    def test_invalid_versions_rejected(self):
        for version in ("../../main", "main", "v0.5.4/extra", "v0.5.4\nmain"):
            with self.subTest(version=version):
                self.install("--version", version, success=False)
        self.assertEqual(self.requests(), [])

    def test_unknown_option_and_missing_version(self):
        self.install("--unknown", success=False)
        self.install("--version", success=False)
        self.assertEqual(self.requests(), [])

    def test_checksum_mismatch_preserves_existing_install(self):
        dest = self.existing_install()
        self.archive.write_bytes(self.archive.read_bytes() + b"tamper")
        result = self.install("--version", "0.5.4", success=False)
        self.assertIn("SHA256 mismatch", result.stderr)
        self.assertEqual(dest.read_text(), "existing installation")

    def test_missing_asset_or_checksum_never_falls_back(self):
        for missing in (self.checksum, self.archive):
            with self.subTest(missing=missing.name):
                self.fixture()
                missing.unlink()
                self.install("--version", "0.5.4", success=False)
                self.assertFalse((self.home / ".local/bin/ntnt").exists())

    def test_checksum_filename_and_multiline_rejected(self):
        for content in ("0" * 64 + "  ../../ntnt\n", "bad\n", self.checksum.read_text() * 2):
            with self.subTest(content=content):
                self.checksum.write_text(content)
                self.install("--version", "0.5.4", success=False)

    def test_checksum_crlf_accepted(self):
        self.checksum.write_bytes(self.checksum.read_bytes().replace(b"\n", b"\r\n"))
        self.install("--version", "0.5.4", "--no-starter-kit")

    def test_extra_archive_entry_rejected(self):
        self.fixture(extra={"../escape": b"no"})
        self.install("--version", "0.5.4", success=False)
        self.assertFalse((self.root / "escape").exists())

    def test_broken_or_wrong_version_binary_preserves_existing(self):
        for body in (b"#!/bin/bash\nexit 1\n", b'#!/bin/bash\nprintf "ntnt 0.1.0\\n"\n'):
            with self.subTest(body=body):
                dest = self.existing_install()
                self.fixture(body=body)
                self.install("--version", "0.5.4", success=False)
                self.assertEqual(dest.read_text(), "existing installation")

    def test_starter_kit_same_tag(self):
        with tarfile.open(self.fixtures / "v0.5.4.tar.gz", "w:gz") as tar:
            data = b"tagged docs"
            info = tarfile.TarInfo("ntnt-0.5.4/docs/README.md")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        self.install("--version", "0.5.4")
        self.assertTrue(self.requests()[-1].endswith("/archive/refs/tags/v0.5.4.tar.gz"))
        self.assertEqual((self.work / "ntnt/docs/README.md").read_text(), "tagged docs")

    def test_starter_kit_skips_existing_directory(self):
        (self.work / "ntnt").mkdir()
        (self.work / "ntnt/keep").write_text("keep")
        self.install("--version", "0.5.4")
        self.assertEqual(len(self.requests()), 2)
        self.assertEqual((self.work / "ntnt/keep").read_text(), "keep")

    def test_starter_kit_download_failure_is_nonfatal(self):
        self.install("--version", "0.5.4")
        self.assertTrue((self.home / ".local/bin/ntnt").exists())

    def test_latest_failure_is_fatal(self):
        (self.fixtures / "latest.json").write_text('{"message":"rate limit"}')
        self.install(success=False)
        self.assertFalse((self.home / ".local/bin/ntnt").exists())

    def test_temporary_files_cleaned_on_success_and_failure(self):
        before = set(self.root.iterdir())
        self.install("--version", "0.5.4", "--no-starter-kit")
        self.assertEqual(set(self.root.iterdir()), before | {self.log})
        self.checksum.unlink()
        self.install("--version", "0.5.4", success=False)
        self.assertEqual(set(self.root.iterdir()), before | {self.log})


if __name__ == "__main__":
    unittest.main()
