"""install.ps1 tests: static checks always, and offline behaviour under pwsh when installed."""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "install.ps1"
PWSH = shutil.which("pwsh")
# Hang guard only: every run is offline and finishes in seconds.
PWSH_HANG_GUARD_SECONDS = 120

TAG = "v0.6.2"
ASSET = "ntnt-windows-x64.zip"
LATEST_PATH = "api.github.com/repos/ntntlang/ntnt/releases/latest"
RELEASE_PATH = f"github.com/ntntlang/ntnt/releases/download/{TAG}"
TAG_SOURCE_PATH = f"github.com/ntntlang/ntnt/archive/refs/tags/{TAG}.zip"
MAIN_SOURCE_PATH = "github.com/ntntlang/ntnt/archive/refs/heads/main.zip"


def statements(script):
    """Lines that run something: not blank, comments or Write-Host messages."""
    for line in script.splitlines():
        line = line.strip()
        if line and not line.startswith(("#", "Write-Host")):
            yield line


class WindowsInstallerStructureTests(unittest.TestCase):
    def test_never_builds_from_source(self):
        script = SCRIPT.read_text()
        self.assertFalse("Build-FromSource" in script, "install.ps1 still has a source build")
        offenders = [line for line in statements(script)
                     if re.search(r"(?i)\b(cargo|rustc|rustup|git)\b", line)]
        self.assertEqual(offenders, [])

    def test_never_downloads_from_a_branch(self):
        offenders = [line for line in statements(SCRIPT.read_text()) if "refs/heads/" in line]
        self.assertEqual(offenders, [])


def fake_ntnt(body):
    """A stand-in ntnt.exe for Linux. The comment pads its archive past the
    installer's 1000-byte truncated-download check."""
    return ("#!/bin/sh\n# " + "x" * 1200 + "\n" + body).encode()


RELEASE_BINARY = fake_ntnt('echo "ntnt 0.6.2"\n')
PREVIOUS_BINARY = b'#!/bin/sh\necho "ntnt 0.6.1"\n'


def zip_bytes(entries):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_STORED) as archive:
        for name, data in entries.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0)), data)
    return buffer.getvalue()


def release_archive(binary=RELEASE_BINARY):
    return zip_bytes({"ntnt.exe": binary})


def sidecar(archive):
    """The .sha256 file the release workflow publishes: lowercase hex, two spaces, LF."""
    return f"{hashlib.sha256(archive).hexdigest()}  {ASSET}\n"


WORKFLOW_SIDECAR = object()

# Offline stand-ins for the network. PowerShell resolves functions before
# cmdlets; Get-FileHash and the archive extraction itself are the real ones.
MOCKS = r"""
$ErrorActionPreference = "Continue"
# The offline release server: https://<host>/<path> is $env:MOCK_SERVER/<host>/<path>.
function Get-MockServerFile([string]$Url) {
    Join-Path $env:MOCK_SERVER ($Url -replace '^https://', '')
}
function Invoke-RestMethod {
    param([string]$Uri, [switch]$UseBasicParsing, [int]$TimeoutSec)
    $file = Get-MockServerFile $Uri
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "404 Not Found: $Uri (offline test)" }
    Get-Content -LiteralPath $file -Raw | ConvertFrom-Json
}
# Fake only System.Net.WebClient; other types (used by the Archive module) are real.
function New-Object {
    [CmdletBinding()]
    param([Parameter(Position = 0)][string]$TypeName,
          [Parameter(Position = 1)][object[]]$ArgumentList,
          [hashtable]$Property)
    if ($TypeName -ne "System.Net.WebClient") {
        Microsoft.PowerShell.Utility\New-Object @PSBoundParameters
        return
    }
    $client = [pscustomobject]@{}
    $client | Add-Member -MemberType ScriptMethod -Name DownloadFile -Value {
        param($url, $path)
        Add-Content -LiteralPath $env:MOCK_DOWNLOADS -Value $url
        $file = Get-MockServerFile $url
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "404 Not Found: $url (offline test)" }
        Copy-Item -LiteralPath $file -Destination $path
    }
    $client
}
# The real Expand-Archive, logged. Windows has no execute bit, so mark the
# extracted .exe files runnable for this Linux harness.
function Expand-Archive {
    [CmdletBinding()]
    param([string]$Path, [string]$LiteralPath, [string]$DestinationPath, [switch]$Force)
    Add-Content -LiteralPath $env:MOCK_EXTRACTIONS -Value "$Path$LiteralPath"
    Microsoft.PowerShell.Archive\Expand-Archive @PSBoundParameters
    Get-ChildItem -LiteralPath $DestinationPath -Recurse -File -Filter *.exe |
        ForEach-Object { & /bin/chmod 755 $_.FullName }
}
"""


class Sandbox:
    """Temporary HOME/USERPROFILE, TEMP, working directory and release server.

    By default TAG is the latest release, its Windows archive matches the
    sidecar, and source archives exist for TAG and for main."""

    def __init__(self, testcase):
        tmp = tempfile.TemporaryDirectory(prefix="ntnt-ps1-installer-test-")
        testcase.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.work, self.home, self.temp, self.bin, self.server = (
            root / name for name in ("work", "home", "temp", "bin", "server"))
        for directory in (self.work, self.home, self.temp, self.bin, self.server):
            directory.mkdir()
        self.install_dir = self.home / ".local" / "bin"
        self.installed = self.install_dir / "ntnt.exe"
        self.tool_calls = root / "tool-calls"
        self.downloads = root / "downloads"
        self.extractions = root / "extractions"
        # The only commands on PATH: build and source tools that just record a
        # call, so a source fallback is visible instead of compiling anything.
        for tool in ("git", "cargo", "rustc", "rustup", "link.exe"):
            path = self.bin / tool
            path.write_text(f'#!/bin/sh\necho "$PWD: {tool} $*" >> "{self.tool_calls}"\n')
            path.chmod(0o755)
        self.wrapper = root / "run-installer.ps1"
        # `&`, not dot-sourcing: an `exit` in a dot-sourced script loses its code.
        self.wrapper.write_text(MOCKS + f"\n& '{SCRIPT}'\nexit $LASTEXITCODE\n")
        self.serve(LATEST_PATH, json.dumps({"tag_name": TAG}))
        self.publish(release_archive())
        self.serve(TAG_SOURCE_PATH, zip_bytes({"ntnt-0.6.2/docs/README.md": b"docs for v0.6.2\n"}))
        self.serve(MAIN_SOURCE_PATH, zip_bytes({"ntnt-main/docs/README.md": b"docs from main\n"}))

    def serve(self, path, content):
        """Serve bytes or text at path; None removes it (a 404)."""
        target = self.server / path
        if content is None:
            target.unlink(missing_ok=True)
            return
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content if isinstance(content, bytes) else content.encode())

    def publish(self, archive, checksum=WORKFLOW_SIDECAR):
        """Serve the Windows archive and its .sha256 sidecar (None: no sidecar)."""
        self.serve(f"{RELEASE_PATH}/{ASSET}", archive)
        self.serve(f"{RELEASE_PATH}/{ASSET}.sha256",
                   sidecar(archive) if checksum is WORKFLOW_SIDECAR else checksum)

    def install_previous(self):
        self.install_dir.mkdir(parents=True)
        self.installed.write_bytes(PREVIOUS_BINARY)
        self.installed.chmod(0o755)

    def run(self):
        assert PWSH is not None
        env = dict(os.environ, PATH=str(self.bin), HOME=str(self.home), USERPROFILE=str(self.home),
                   TEMP=str(self.temp), TMPDIR=str(self.temp), MOCK_SERVER=str(self.server),
                   MOCK_DOWNLOADS=str(self.downloads), MOCK_EXTRACTIONS=str(self.extractions),
                   POWERSHELL_TELEMETRY_OPTOUT="1", POWERSHELL_UPDATECHECK="Off")
        return subprocess.run([PWSH, "-NoProfile", "-NonInteractive", "-File", str(self.wrapper)],
                              cwd=self.work, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, timeout=PWSH_HANG_GUARD_SECONDS)

    @staticmethod
    def lines(path):
        return path.read_text().splitlines() if path.exists() else []

    def install_dir_entries(self):
        return sorted(p.name for p in self.install_dir.iterdir()) if self.install_dir.exists() else []


@unittest.skipUnless(PWSH and os.name == "posix", "needs pwsh on a POSIX host")
class WindowsInstallerBehaviourTests(unittest.TestCase):
    """Run install.ps1 under pwsh against an offline release server."""

    def assert_refused(self, box, result, previous_install):
        """Exit 1 without a source build or starter kit; the install dir is as before."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertIn("No source fallback was attempted", result.stdout)
        self.assertEqual(box.lines(box.tool_calls), [])
        self.assertFalse(any("/archive/" in url for url in box.lines(box.downloads)), output)
        if previous_install:
            self.assertEqual(box.install_dir_entries(), ["ntnt.exe"], output)
            self.assertEqual(box.installed.read_bytes(), PREVIOUS_BINARY, output)
        else:
            self.assertEqual(box.install_dir_entries(), [], output)

    def test_unavailable_release_stops_without_building_source(self):
        for case, path in (("no latest release", LATEST_PATH), ("no Windows archive", f"{RELEASE_PATH}/{ASSET}")):
            with self.subTest(case):
                box = Sandbox(self)
                box.serve(path, None)
                # A caller-controlled ./ntnt checkout must never be updated or compiled.
                planted = box.work / "ntnt"
                planted.mkdir()
                (planted / "build.rs").write_text("fn main() {}\n")
                result = box.run()
                self.assert_refused(box, result, previous_install=False)
                self.assertEqual(sorted(p.name for p in planted.iterdir()), ["build.rs"])

    def test_verified_release_replaces_previous_install(self):
        archive = release_archive()
        digest = hashlib.sha256(archive).hexdigest()
        checksums = {
            "release workflow format (LF)": sidecar(archive),
            "releases up to v0.6.2 (CRLF)": f"{digest}  {ASSET}\r\n",
            "uppercase hex, LF": f"{digest.upper()}  {ASSET}\n",
            "binary-mode marker, no newline": f"{digest} *{ASSET}",
            "UTF-8 byte order mark": f"\ufeff{digest}  {ASSET}\r\n",
        }
        for case, checksum in checksums.items():
            with self.subTest(case):
                box = Sandbox(self)
                box.install_previous()
                box.publish(archive, checksum)
                result = box.run()
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(box.install_dir_entries(), ["ntnt.exe"])
                self.assertEqual(box.installed.read_bytes(), RELEASE_BINARY)
                self.assertIn("Version: ntnt 0.6.2", result.stdout)
                self.assertEqual(box.lines(box.tool_calls), [])

    def test_archive_that_does_not_match_its_checksum_installs_nothing(self):
        published = release_archive()
        tampered = release_archive(fake_ntnt('echo "ntnt 0.6.2"\necho tampered\n'))
        for previous_install in (True, False):
            with self.subTest(previous_install=previous_install):
                box = Sandbox(self)
                if previous_install:
                    box.install_previous()
                box.publish(tampered, sidecar(published))
                result = box.run()
                self.assert_refused(box, result, previous_install)
                self.assertIn("SHA256 mismatch", result.stdout)
                self.assertEqual(box.lines(box.extractions), [], "extracted before verifying")

    def test_missing_or_malformed_checksum_installs_nothing(self):
        archive = release_archive()
        digest = hashlib.sha256(archive).hexdigest()
        checksums = {
            "missing": None,
            "empty": "",
            "not a checksum": "Not Found\r\n",
            "short digest": f"{digest[:-1]}  {ASSET}\r\n",
            "digest without a file name": f"{digest}\r\n",
            "names another archive": f"{digest}  ntnt-linux-x64.tar.gz\r\n",
            "two entries": f"{digest}  {ASSET}\r\n{digest}  {ASSET}\r\n",
        }
        for case, checksum in checksums.items():
            with self.subTest(case):
                box = Sandbox(self)
                box.install_previous()
                box.publish(archive, checksum)
                result = box.run()
                self.assert_refused(box, result, previous_install=True)
                self.assertIn("checksum", result.stdout)
                self.assertEqual(box.lines(box.extractions), [], "extracted before verifying")

    def test_release_binary_that_does_not_run_keeps_previous_install(self):
        binaries = {
            "exits with an error": fake_ntnt('echo "cannot start" >&2\nexit 3\n'),
            "not an executable": b"MZ" + bytes(2000),
            "reports another version": fake_ntnt('echo "ntnt 0.6.0"\n'),
        }
        for case, binary in binaries.items():
            with self.subTest(case):
                box = Sandbox(self)
                box.install_previous()
                box.publish(release_archive(binary))
                result = box.run()
                self.assert_refused(box, result, previous_install=True)

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0, "root can write a read-only directory")
    def test_unwritable_install_directory_fails_and_keeps_previous_install(self):
        box = Sandbox(self)
        box.install_previous()
        box.install_dir.chmod(0o555)
        self.addCleanup(box.install_dir.chmod, 0o755)
        result = box.run()
        self.assert_refused(box, result, previous_install=True)

    def test_starter_kit_comes_from_the_release_tag(self):
        box = Sandbox(self)
        result = box.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(box.installed.read_bytes(), RELEASE_BINARY)
        self.assertEqual((box.work / "ntnt/docs/README.md").read_text(), "docs for v0.6.2\n")
        downloads = box.lines(box.downloads)
        self.assertIn(f"https://{TAG_SOURCE_PATH}", downloads)
        self.assertFalse(any("/refs/heads/" in url for url in downloads), downloads)

    def test_starter_kit_never_overwrites_existing_directory(self):
        box = Sandbox(self)
        existing = box.work / "ntnt/docs"
        existing.mkdir(parents=True)
        (existing / "README.md").write_text("my notes\n")
        result = box.run()
        self.assertEqual((existing / "README.md").read_text(), "my notes\n")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Skipping starter kit", result.stdout)
        self.assertEqual(sorted(p.name for p in (box.work / "ntnt").iterdir()), ["docs"])
        self.assertFalse(any("/archive/" in url for url in box.lines(box.downloads)))


if __name__ == "__main__":
    unittest.main()
