# NTNT Language Installer for Windows
# Usage: irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex
# No source fallback: if the release binary cannot be installed, the installer
# stops. It never installs Rust, clones or updates a checkout, or compiles code.
#
# Requirements: PowerShell 5.0+ (Windows 10+ has this by default)

$ErrorActionPreference = "Continue"
$script:ExitCode = 0
$script:Repo = "ntntlang/ntnt"
$script:InstallDir = "$env:USERPROFILE\.local\bin"

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

function Try-DownloadBinary {
    Write-Host "Checking for pre-built binary..."
    Write-Host ""

    $version = Get-LatestVersion
    if (-not $version) {
        Write-Host "Could not determine latest version (no releases yet or API unavailable)." -ForegroundColor Yellow
        return $false
    }

    Write-Host "Latest version: $version"

    $url = "https://github.com/$script:Repo/releases/download/$version/ntnt-windows-x64.zip"
    $tmpDir = Join-Path $env:TEMP "ntnt-install-$(Get-Random)"
    $tmpFile = Join-Path $tmpDir "ntnt.zip"

    Write-Host "Downloading: $url"
    Write-Host ""

    try {
        # Create temp directory
        New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

        # Download with timeout
        $webClient = New-Object System.Net.WebClient
        $webClient.DownloadFile($url, $tmpFile)

        # Verify file was downloaded and has content
        if (-not (Test-Path $tmpFile)) {
            throw "Download failed - file not created"
        }
        $fileSize = (Get-Item $tmpFile).Length
        if ($fileSize -lt 1000) {
            throw "Download failed - file too small ($fileSize bytes)"
        }

        Write-Host "Downloaded $([math]::Round($fileSize / 1MB, 2)) MB"

        # Extract
        try {
            Expand-Archive -Path $tmpFile -DestinationPath $tmpDir -Force
        } catch {
            throw "Failed to extract archive: $_"
        }

        # Find the binary (might be in root or subdirectory)
        $ntntExe = Get-ChildItem -Path $tmpDir -Filter "ntnt.exe" -Recurse | Select-Object -First 1
        if (-not $ntntExe) {
            throw "Binary not found in archive"
        }

        # Install to ~/.local/bin
        New-Item -ItemType Directory -Force -Path $script:InstallDir | Out-Null
        $destPath = Join-Path $script:InstallDir "ntnt.exe"
        Move-Item -Path $ntntExe.FullName -Destination $destPath -Force

        # Cleanup temp
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue

        # Verify it runs
        try {
            $testOutput = & $destPath --version 2>&1
            if ($LASTEXITCODE -ne 0) {
                throw "Binary returned error code $LASTEXITCODE"
            }
        } catch {
            Remove-Item -Force $destPath -ErrorAction SilentlyContinue
            throw "Binary downloaded but won't run: $_"
        }

        Write-Host "[OK] Downloaded and installed ntnt to $script:InstallDir" -ForegroundColor Green
        return $true

    } catch {
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        Write-Host "Download failed: $_" -ForegroundColor Yellow
        return $false
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

# Download docs, examples, and agent helper files into a new .\ntnt directory.
# An existing .\ntnt (a project or clone) is never modified.
function Download-StarterKit {
    $ntntHome = Join-Path (Get-Location) "ntnt"
    Write-Host ""
    if (Test-Path -LiteralPath $ntntHome) {
        Write-Host "Skipping starter kit: .\ntnt already exists." -ForegroundColor Yellow
        return $false
    }
    Write-Host "Downloading NTNT starter kit (docs, examples, agent guides)..."

    # Download zip from GitHub (no git required)
    $tmpDir = Join-Path $env:TEMP "ntnt-starter-$(Get-Random)"
    $zipFile = Join-Path $tmpDir "ntnt.zip"
    $zipUrl = "https://github.com/$script:Repo/archive/refs/heads/main.zip"

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
