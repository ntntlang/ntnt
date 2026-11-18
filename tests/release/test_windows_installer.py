"""install.ps1 tests: a static check always, and offline behaviour under pwsh when installed."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "install.ps1"
PWSH = shutil.which("pwsh")
# Hang guard only: every run is offline and finishes in seconds.
PWSH_HANG_GUARD_SECONDS = 120


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


# Offline stand-ins for the cmdlets install.ps1 uses to reach the network or
# unpack archives. PowerShell resolves functions before cmdlets.
MOCKS = r"""
$ErrorActionPreference = "Continue"
function Invoke-RestMethod {
    if ($env:MOCK_RELEASE -eq "missing") { throw "no release (offline test)" }
    [pscustomobject]@{ tag_name = "v0.6.2" }
}
function New-Object {
    $downloader = [pscustomobject]@{}
    $downloader | Add-Member -MemberType ScriptMethod -Name DownloadFile -Value {
        param($url, $path)
        Add-Content -Path $env:MOCK_DOWNLOADS -Value $url
        if ($env:MOCK_RELEASE -eq "broken") { throw "download failed (offline test)" }
        Set-Content -Path $path -Value ($url + (" " * 2000))
    }
    $downloader
}
function Expand-Archive {
    param($Path, $DestinationPath, [switch]$Force)
    $url = (Get-Content -Path $Path -Raw).Trim()
    if ($url -like "*/ntnt-windows-x64.zip") {
        $exe = Join-Path $DestinationPath "ntnt.exe"
        Set-Content -Path $exe -Value "#!/bin/sh`necho ntnt 0.6.2"
        & /bin/chmod 755 $exe
    } else {
        $docs = Join-Path $DestinationPath "ntnt-main/docs"
        New-Item -ItemType Directory -Force -Path $docs | Out-Null
        Set-Content -Path (Join-Path $docs "README.md") -Value "starter kit docs"
    }
}
"""


@unittest.skipUnless(PWSH, "pwsh is not installed")
class WindowsInstallerBehaviourTests(unittest.TestCase):
    """Run install.ps1 under pwsh with every download mocked."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ntnt-ps1-installer-test-")
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.work, self.home, self.temp, self.bin = (root / name for name in ("work", "home", "temp", "bin"))
        for directory in (self.work, self.home, self.temp, self.bin):
            directory.mkdir()
        self.tool_calls = root / "tool-calls"
        self.downloads = root / "downloads"
        # The only commands on PATH: build and source tools that just record a
        # call, so a source fallback is visible instead of compiling anything.
        for tool in ("git", "cargo", "rustc", "rustup", "link.exe"):
            path = self.bin / tool
            path.write_text(f'#!/bin/sh\necho "$PWD: {tool} $*" >> "{self.tool_calls}"\n')
            path.chmod(0o755)
        self.wrapper = root / "run-installer.ps1"
        # `&`, not dot-sourcing: an `exit` in a dot-sourced script loses its code.
        self.wrapper.write_text(MOCKS + f"\n& '{SCRIPT}'\nexit $LASTEXITCODE\n")

    def install(self, release):
        assert PWSH is not None
        env = dict(os.environ, PATH=str(self.bin), HOME=str(self.home), USERPROFILE=str(self.home),
                   TEMP=str(self.temp), TMPDIR=str(self.temp), MOCK_RELEASE=release,
                   MOCK_DOWNLOADS=str(self.downloads), POWERSHELL_TELEMETRY_OPTOUT="1",
                   POWERSHELL_UPDATECHECK="Off")
        return subprocess.run([PWSH, "-NoProfile", "-NonInteractive", "-File", str(self.wrapper)],
                              cwd=self.work, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, timeout=PWSH_HANG_GUARD_SECONDS)

    def tool_call_log(self):
        return self.tool_calls.read_text() if self.tool_calls.exists() else ""

    def test_missing_binary_stops_without_building_source(self):
        for release in ("missing", "broken"):
            with self.subTest(release=release):
                # A caller-controlled ./ntnt checkout must never be updated or compiled.
                planted = self.work / "ntnt"
                planted.mkdir(exist_ok=True)
                (planted / "build.rs").write_text("fn main() {}\n")
                result = self.install(release)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("No source fallback was attempted", result.stdout)
                self.assertEqual(self.tool_call_log(), "")
                self.assertEqual(sorted(p.name for p in planted.iterdir()), ["build.rs"])
                self.assertFalse((self.home / ".local/bin/ntnt.exe").exists())

    def test_binary_install_and_starter_kit(self):
        result = self.install("ok")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.tool_call_log(), "")
        self.assertEqual((self.work / "ntnt/docs/README.md").read_text().strip(), "starter kit docs")
        self.assertTrue((self.home / ".local/bin/ntnt.exe").is_file())

    def test_starter_kit_never_overwrites_existing_directory(self):
        existing = self.work / "ntnt/docs"
        existing.mkdir(parents=True)
        (existing / "README.md").write_text("my notes\n")
        result = self.install("ok")
        self.assertEqual((existing / "README.md").read_text(), "my notes\n")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Skipping starter kit", result.stdout)
        self.assertEqual(sorted(p.name for p in (self.work / "ntnt").iterdir()), ["docs"])
        self.assertFalse(any("/archive/" in url for url in self.downloads.read_text().splitlines()))


if __name__ == "__main__":
    unittest.main()
