# NTNT Language Installer for Windows
# Usage: irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex
# The release archive must match its .sha256 sidecar before extraction, and the
# new ntnt.exe must run and report the release version before it replaces an
# existing one. No source fallback: if the release binary cannot be installed,
# the installer stops. It never installs Rust, clones or updates a checkout, or
# compiles code.
#
# Requirements: PowerShell 5.0+ (Windows 10+ has this by default)

$ErrorActionPreference = "Continue"
$script:ExitCode = 0
$script:Repo = "ntntlang/ntnt"
$script:InstallDir = "$env:USERPROFILE\.local\bin"
$script:Archive = "ntnt-windows-x64.zip"
# The installed release tag. Not $script:Version: the main block assigns
# $version, and PowerShell variable names ignore case.
$script:ReleaseTag = $null

# Check PowerShell version
if ($PSVersionTable.PSVersion.Major -lt 5) {
    Write-Host ""
    Write-Host "X PowerShell 5.0 or higher required" -ForegroundColor Red
    Write-Host ""
    Write-Host "Your version: $($PSVersionTable.PSVersion)"
    Write-Host "Please update PowerShell: https://aka.ms/PSWindows"
    Write-Host ""
    Write-Host "Press Enter to close..." -ForegroundColor Gray
    try { [Console]::ReadLine() | Out-Null } catch { Start-Sleep 30 }
    exit 1
}

function Wait-AndExit {
    Write-Host ""
    Write-Host "Press Enter to close..." -ForegroundColor Gray
    try { [Console]::ReadLine() | Out-Null } catch { Start-Sleep 30 }
    exit $script:ExitCode
}

function Get-LatestVersion {
    try {
        $url = "https://api.github.com/repos/$script:Repo/releases/latest"
        $response = Invoke-RestMethod -Uri $url -UseBasicParsing -TimeoutSec 10
        return $response.tag_name
    } catch {
        return $null
    }
}

# Return the SHA-256 that a release .sha256 sidecar records for $archiveName.
# The release workflow writes "<hex>  <name>" with LF; releases up to v0.6.2 used
# CRLF. Accept either or no line end, a byte order mark, either hex case and a
# "*" binary-mode marker, but only a single entry that names this exact archive.
function Read-ReleaseChecksum($checksumFile, $archiveName) {
    # ReadAllText drops a byte order mark.
    $text = [System.IO.File]::ReadAllText($checksumFile).Replace("`r", "").Trim()
    $entry = [regex]::Match($text, '^([0-9A-Fa-f]{64})[ \t]+\*?(\S+)$')
    if (-not $entry.Success) {
        throw "Malformed release checksum file $archiveName.sha256; nothing installed"
    }
    if ($entry.Groups[2].Value -cne $archiveName) {
        throw "Release checksum names a different archive ($($entry.Groups[2].Value)); nothing installed"
    }
    return $entry.Groups[1].Value
}

# Run ntnt.exe --version and return what it printed; throw if it cannot run.
function Get-NtntVersionOutput($exe) {
    $output = & $exe --version
    if ($LASTEXITCODE -ne 0) {
        throw "exit code $LASTEXITCODE"
    }
    return ($output | Out-String).Trim()
}

function Try-DownloadBinary {
    Write-Host "Checking for pre-built binary..."
    Write-Host ""

    $version = Get-LatestVersion
    if (-not $version) {
        Write-Host "Could not determine latest version (no releases yet or API unavailable)." -ForegroundColor Yellow
        return $false
    }
    if ($version -cnotmatch '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$') {
        Write-Host "Latest release has an invalid version tag: $version" -ForegroundColor Yellow
        return $false
    }

    Write-Host "Latest version: $version"

    $url = "https://github.com/$script:Repo/releases/download/$version/$script:Archive"
    $tmpDir = Join-Path $env:TEMP "ntnt-install-$(Get-Random)"
    $tmpFile = Join-Path $tmpDir $script:Archive
    $checksumFile = "$tmpFile.sha256"
    $extractDir = Join-Path $tmpDir "extract"
    $stagedPath = $null

    Write-Host "Downloading: $url"
    Write-Host ""

    try {
        # Create temp directory
        New-Item -ItemType Directory -Force -Path $tmpDir -ErrorAction Stop | Out-Null

        # Download with timeout
        $webClient = New-Object System.Net.WebClient
        $webClient.DownloadFile($url, $tmpFile)

        # Verify file was downloaded and has content
        if (-not (Test-Path -LiteralPath $tmpFile)) {
            throw "Download failed - file not created"
        }
        $fileSize = (Get-Item -LiteralPath $tmpFile).Length
        if ($fileSize -lt 1000) {
            throw "Download failed - file too small ($fileSize bytes)"
        }

        Write-Host "Downloaded $([math]::Round($fileSize / 1MB, 2)) MB"

        # Verify the archive against the checksum published with it before
        # extracting anything.
        try {
            $webClient.DownloadFile("$url.sha256", $checksumFile)
        } catch {
            throw "Release checksum unavailable ($url.sha256); nothing installed: $_"
        }
        $expectedHash = Read-ReleaseChecksum $checksumFile $script:Archive
        $actualHash = (Get-FileHash -LiteralPath $tmpFile -Algorithm SHA256 -ErrorAction Stop).Hash
        if ($actualHash -ine $expectedHash) {
            throw "SHA256 mismatch for $script:Archive (expected $($expectedHash.ToLowerInvariant()), got $($actualHash.ToLowerInvariant())); nothing installed"
        }
        Write-Host "[OK] SHA256 verified"

        # Extract
        try {
            Expand-Archive -LiteralPath $tmpFile -DestinationPath $extractDir -Force -ErrorAction Stop
        } catch {
            throw "Failed to extract archive: $_"
        }

        # Find the binary (might be in root or subdirectory)
        $ntntExe = Get-ChildItem -LiteralPath $extractDir -Filter "ntnt.exe" -Recurse | Select-Object -First 1
        if (-not $ntntExe) {
            throw "Binary not found in archive"
        }

        # Stage the candidate next to ~/.local/bin/ntnt.exe and run it there; an
        # existing ntnt.exe is replaced only after it reports this release.
        New-Item -ItemType Directory -Force -Path $script:InstallDir -ErrorAction Stop | Out-Null
        $destPath = Join-Path $script:InstallDir "ntnt.exe"
        if (Test-Path -LiteralPath $destPath -PathType Container) {
            throw "Install destination is a directory: $destPath"
        }
        $stagedPath = Join-Path $script:InstallDir ".ntnt-$(Get-Random).exe"
        Copy-Item -LiteralPath $ntntExe.FullName -Destination $stagedPath -ErrorAction Stop

        $expectedOutput = "ntnt $($version.Substring(1))"
        try {
            $reported = Get-NtntVersionOutput $stagedPath
        } catch {
            throw "Binary cannot run on this system ($_); existing installation unchanged"
        }
        if ($reported -cne $expectedOutput) {
            throw "Binary reports '$reported', expected '$expectedOutput'; existing installation unchanged"
        }

        try {
            Move-Item -LiteralPath $stagedPath -Destination $destPath -Force -ErrorAction Stop
        } catch {
            throw "Could not replace $destPath (is ntnt running?): $_"
        }
        $stagedPath = $null
        $script:ReleaseTag = $version

        Write-Host "[OK] Installed $reported to $script:InstallDir" -ForegroundColor Green
        return $true

    } catch {
        Write-Host "Install failed: $_" -ForegroundColor Yellow
        return $false
    } finally {
        if ($stagedPath) {
            Remove-Item -LiteralPath $stagedPath -Force -ErrorAction SilentlyContinue
        }
        Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ============================================
# Main Installation
# ============================================

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  NTNT Language Installer for Windows" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

if (-not (Try-DownloadBinary)) {
    Write-Host ""
    Write-Host "X Could not install the NTNT release binary. No source fallback was attempted." -ForegroundColor Red
    Write-Host ""
    Write-Host "Re-run the installer once the release download works, or build the release"
    Write-Host "tag you want yourself; see Build from Source in https://github.com/$script:Repo#readme"
    $script:ExitCode = 1
    Wait-AndExit
}

Write-Host ""
Write-Host "============================================" -ForegroundColor Green
Write-Host "  NTNT installed successfully!" -ForegroundColor Green
Write-Host "============================================" -ForegroundColor Green
Write-Host ""

# Show version
$ntntExe = Join-Path $script:InstallDir "ntnt.exe"
if (Test-Path $ntntExe) {
    try {
        $version = & $ntntExe --version 2>$null
        Write-Host "Version: $version"
    } catch {
        Write-Host "Version: (unable to determine)"
    }
}
Write-Host ""

# Check if in PATH
$pathDirs = $env:PATH -split ';'
$inPath = $pathDirs | Where-Object { $_ -eq $script:InstallDir }
if (-not $inPath) {
    Write-Host "NOTE: Add ntnt to your PATH to use it from anywhere." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "  Option 1 - Add permanently (run this once):" -ForegroundColor Cyan
    Write-Host "  [Environment]::SetEnvironmentVariable('PATH', `"$script:InstallDir;`" + [Environment]::GetEnvironmentVariable('PATH', 'User'), 'User')"
    Write-Host ""
    Write-Host "  Option 2 - Add for this session only:"
    Write-Host "  `$env:PATH = `"$script:InstallDir;`$env:PATH`"" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "  Then restart your terminal."
    Write-Host ""
}

# Download docs, examples, and agent helper files from the installed release's
# tag (never main) into a new .\ntnt directory. An existing .\ntnt (a project or
# clone) is never modified.
function Download-StarterKit {
    $ntntHome = Join-Path (Get-Location) "ntnt"
    Write-Host ""
    if (Test-Path -LiteralPath $ntntHome) {
        Write-Host "Skipping starter kit: .\ntnt already exists." -ForegroundColor Yellow
        return $false
    }
    Write-Host "Downloading NTNT $script:ReleaseTag starter kit (docs, examples, agent guides)..."

    # Download zip from GitHub (no git required)
    $tmpDir = Join-Path $env:TEMP "ntnt-starter-$(Get-Random)"
    $zipFile = Join-Path $tmpDir "ntnt.zip"
    $zipUrl = "https://github.com/$script:Repo/archive/refs/tags/$($script:ReleaseTag).zip"

    try {
        New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

        # Download the zip file
        $webClient = New-Object System.Net.WebClient
        $webClient.DownloadFile($zipUrl, $zipFile)

        # Extract
        Expand-Archive -Path $zipFile -DestinationPath $tmpDir -Force

        # Find extracted directory (e.g., ntnt-main)
        $extractedDir = Get-ChildItem -Path $tmpDir -Directory -Filter "ntnt-*" | Select-Object -First 1
        if (-not $extractedDir) {
            throw "Starter kit archive has no ntnt-* directory"
        }

        # Without -Force this fails instead of merging if .\ntnt appeared meanwhile.
        New-Item -ItemType Directory -Path $ntntHome -ErrorAction Stop | Out-Null
        $src = $extractedDir.FullName
        if (Test-Path "$src\docs") { Copy-Item -Recurse "$src\docs" $ntntHome -Force }
        if (Test-Path "$src\examples") { Copy-Item -Recurse "$src\examples" $ntntHome -Force }
        if (Test-Path "$src\.github") { Copy-Item -Recurse "$src\.github" $ntntHome -Force }
        if (Test-Path "$src\.claude\skills") {
            New-Item -ItemType Directory -Force -Path "$ntntHome\.claude" | Out-Null
            Copy-Item -Recurse "$src\.claude\skills" "$ntntHome\.claude\" -Force
        }
        if (Test-Path "$src\CLAUDE.md") { Copy-Item "$src\CLAUDE.md" $ntntHome -Force }

        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue

        Write-Host "[OK] Starter kit downloaded to .\ntnt\" -ForegroundColor Green
        Write-Host ""
        Write-Host "  .\ntnt\docs\              - Documentation"
        Write-Host "  .\ntnt\examples\          - Example projects"
        Write-Host "  .\ntnt\CLAUDE.md          - Claude Code instructions"
        Write-Host "  .\ntnt\.claude\skills\    - Claude Code skills (IDD workflow)"
        Write-Host "  .\ntnt\.github\copilot-instructions.md  - GitHub Copilot instructions"
        return $true
    } catch {
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        Write-Host "Could not download starter kit. You can browse docs at:" -ForegroundColor Yellow
        Write-Host "  https://github.com/$script:Repo"
        return $false
    }
}

$hasStarterKit = Download-StarterKit

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  Quick Start" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "Try these commands:"
Write-Host "  ntnt run hello.tnt     Run a file" -ForegroundColor Green
Write-Host "  ntnt repl              Interactive REPL" -ForegroundColor Green
Write-Host "  ntnt --help            See all commands" -ForegroundColor Green
Write-Host ""
if ($hasStarterKit) {
    Write-Host "Examples:    .\ntnt\examples\"
    Write-Host "Docs:        .\ntnt\docs\"
    Write-Host "Agent guide: .\ntnt\CLAUDE.md"
} else {
    Write-Host "Examples: https://github.com/$script:Repo/tree/main/examples"
    Write-Host "Docs:     https://github.com/$script:Repo"
}
Write-Host ""

# Show tab completion instructions
Write-Host "Tab Completion (optional):" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Add to your PowerShell profile (`$PROFILE):"
Write-Host "    ntnt completions powershell | Out-String | Invoke-Expression" -ForegroundColor Green
Write-Host ""
Write-Host "  Or run once to add permanently:"
Write-Host "    ntnt completions powershell >> `$PROFILE" -ForegroundColor Green
Write-Host ""

Write-Host "Happy coding!"
Write-Host ""

Wait-AndExit
