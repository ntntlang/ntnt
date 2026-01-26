# NTNT Language Installer for Windows
# Usage: irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex
#
# Requirements: PowerShell 5.0+ (Windows 10+ has this by default)

$ErrorActionPreference = "Continue"
$script:ExitCode = 0
$script:Repo = "ntntlang/ntnt"
$script:InstallDir = "$env:USERPROFILE\.local\bin"
$script:InstalledFrom = ""

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
        Write-Host "Will build from source instead." -ForegroundColor Yellow
        return $false
    }
}

function Build-FromSource {
    Write-Host ""
    Write-Host "Building from source..."
    Write-Host ""

    # Check for Visual Studio Build Tools by testing compilation
    $hasBuildTools = $false

    # Quick check: is link.exe in PATH?
    if (Get-Command link.exe -ErrorAction SilentlyContinue) {
        $hasBuildTools = $true
    }

    # Better check: try to compile something
    if (-not $hasBuildTools -and (Get-Command rustc -ErrorAction SilentlyContinue)) {
        $testFile = Join-Path $env:TEMP "ntnt_build_test_$(Get-Random).rs"
        $testExe = $testFile -replace '\.rs$', '.exe'
        try {
            "fn main() {}" | Out-File -Encoding ascii $testFile
            $null = & rustc $testFile -o $testExe 2>&1
            if ($LASTEXITCODE -eq 0 -and (Test-Path $testExe)) {
                $hasBuildTools = $true
            }
        } finally {
            Remove-Item $testFile -ErrorAction SilentlyContinue
            Remove-Item $testExe -ErrorAction SilentlyContinue
        }
    }

    if (-not $hasBuildTools) {
        Write-Host "X Visual Studio Build Tools not found" -ForegroundColor Red
        Write-Host ""
        Write-Host "NTNT requires the MSVC linker (link.exe) to compile on Windows."
        Write-Host ""
        Write-Host "Install Visual Studio Build Tools:" -ForegroundColor Yellow
        Write-Host ""
        Write-Host "  1. Download from:"
        Write-Host "     https://visualstudio.microsoft.com/visual-cpp-build-tools/"
        Write-Host ""
        Write-Host "  2. Run the installer and select:"
        Write-Host '     "Desktop development with C++"'
        Write-Host ""
        Write-Host "  3. IMPORTANT: After installing, open a NEW terminal"
        Write-Host "     (or use 'Developer Command Prompt for VS')"
        Write-Host ""
        $script:ExitCode = 1
        Wait-AndExit
    }
    Write-Host "[OK] Build tools available" -ForegroundColor Green

    # Check for Git
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        Write-Host "X Git not found" -ForegroundColor Red
        Write-Host ""
        Write-Host "Install Git from:" -ForegroundColor Yellow
        Write-Host "  https://git-scm.com/download/win"
        Write-Host ""
        Write-Host "After installing, restart your terminal and re-run this installer."
        $script:ExitCode = 1
        Wait-AndExit
    }
    Write-Host "[OK] Git available" -ForegroundColor Green

    # Check for Rust/Cargo
    $cargoPath = "$env:USERPROFILE\.cargo\bin\cargo.exe"
    $hasRust = (Get-Command cargo -ErrorAction SilentlyContinue) -or (Test-Path $cargoPath)

    if (-not $hasRust) {
        Write-Host ""
        Write-Host "Rust not found. Installing via rustup..." -ForegroundColor Yellow

        $rustupInit = Join-Path $env:TEMP "rustup-init-$(Get-Random).exe"
        try {
            $webClient = New-Object System.Net.WebClient
            $webClient.DownloadFile("https://win.rustup.rs/x86_64", $rustupInit)

            Start-Process -FilePath $rustupInit -ArgumentList "-y" -Wait -NoNewWindow
            Remove-Item $rustupInit -ErrorAction SilentlyContinue

            # Add to current session PATH
            $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

            Write-Host "[OK] Rust installed" -ForegroundColor Green
        } catch {
            Write-Host "X Failed to install Rust: $_" -ForegroundColor Red
            $script:ExitCode = 1
            Wait-AndExit
        }
    } else {
        $rustVersion = & rustc --version 2>$null
        Write-Host "[OK] Rust available: $rustVersion" -ForegroundColor Green
    }

    # Clone or update repo
    $ntntDir = Join-Path (Get-Location) "ntnt"

    try {
        if (Test-Path $ntntDir) {
            Write-Host "Updating NTNT source in .\ntnt..."
            Push-Location $ntntDir
            git fetch --quiet origin 2>$null
            git reset --quiet --hard origin/main 2>$null
            git clean --quiet -fd 2>$null
        } else {
            Write-Host "Cloning NTNT source to .\ntnt..."
            $cloneResult = git clone --quiet "https://github.com/$script:Repo.git" $ntntDir 2>&1
            if (-not (Test-Path $ntntDir)) {
                throw "Git clone failed: $cloneResult"
            }
            Push-Location $ntntDir
        }
    } catch {
        Write-Host "X Failed to download source: $_" -ForegroundColor Red
        $script:ExitCode = 1
        Wait-AndExit
    }

    # Build
    Write-Host ""
    Write-Host "Building NTNT (this takes a few minutes)..."
    Write-Host ""

    try {
        & cargo install --path . --locked 2>&1 | ForEach-Object { Write-Host $_ }
        if ($LASTEXITCODE -ne 0) {
            throw "Build failed with exit code $LASTEXITCODE"
        }
    } catch {
        Pop-Location
        Write-Host ""
        Write-Host "X Build failed: $_" -ForegroundColor Red
        Write-Host ""
        Write-Host "If this looks like a bug, please report it at:"
        Write-Host "  https://github.com/$script:Repo/issues" -ForegroundColor Cyan
        $script:ExitCode = 1
        Wait-AndExit
    }

    Pop-Location

    # cargo install puts it in ~/.cargo/bin
    $script:InstallDir = "$env:USERPROFILE\.cargo\bin"
    $script:InstalledFrom = "source"

    Write-Host ""
    Write-Host "[OK] Built and installed ntnt to $script:InstallDir" -ForegroundColor Green
}

# ============================================
# Main Installation
# ============================================

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  NTNT Language Installer for Windows" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

if (Try-DownloadBinary) {
    $script:InstalledFrom = "binary"
} else {
    Build-FromSource
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

# Download docs, examples, and agent helper files
function Download-StarterKit {
    $script:NtntHome = Join-Path (Get-Location) "ntnt"
    Write-Host ""
    Write-Host "Downloading NTNT starter kit (docs, examples, agent guides)..."

    New-Item -ItemType Directory -Force -Path $script:NtntHome | Out-Null

    $tmpClone = Join-Path $env:TEMP "ntnt-clone-$(Get-Random)"

    try {
        # Try sparse checkout for efficiency (use --no-cone for mixed file/directory patterns)
        $null = git clone --depth 1 --filter=blob:none --sparse "https://github.com/$script:Repo.git" $tmpClone 2>&1
        if ($LASTEXITCODE -eq 0) {
            Push-Location $tmpClone
            # Use --no-cone mode to allow mixed directory and file patterns
            $null = git sparse-checkout set --no-cone "docs/*" "examples/*" ".github/*" ".claude/skills/*" "CLAUDE.md" 2>&1

            # Copy the files we want
            if (Test-Path "docs") { Copy-Item -Recurse "docs" $script:NtntHome -Force }
            if (Test-Path "examples") { Copy-Item -Recurse "examples" $script:NtntHome -Force }
            if (Test-Path ".github") { Copy-Item -Recurse ".github" $script:NtntHome -Force }
            if (Test-Path ".claude\skills") {
                New-Item -ItemType Directory -Force -Path "$script:NtntHome\.claude" | Out-Null
                Copy-Item -Recurse ".claude\skills" "$script:NtntHome\.claude\" -Force
            }
            if (Test-Path "CLAUDE.md") { Copy-Item "CLAUDE.md" $script:NtntHome -Force }

            Pop-Location
            Remove-Item -Recurse -Force $tmpClone -ErrorAction SilentlyContinue
        } else {
            throw "Sparse checkout failed"
        }
    } catch {
        # Fallback: full shallow clone
        Remove-Item -Recurse -Force $tmpClone -ErrorAction SilentlyContinue
        $tmpClone = Join-Path $env:TEMP "ntnt-clone-$(Get-Random)"

        try {
            $null = git clone --depth 1 "https://github.com/$script:Repo.git" $tmpClone 2>&1
            if ($LASTEXITCODE -eq 0) {
                if (Test-Path "$tmpClone\docs") { Copy-Item -Recurse "$tmpClone\docs" $script:NtntHome -Force }
                if (Test-Path "$tmpClone\examples") { Copy-Item -Recurse "$tmpClone\examples" $script:NtntHome -Force }
                if (Test-Path "$tmpClone\.github") { Copy-Item -Recurse "$tmpClone\.github" $script:NtntHome -Force }
                if (Test-Path "$tmpClone\.claude\skills") {
                    New-Item -ItemType Directory -Force -Path "$script:NtntHome\.claude" | Out-Null
                    Copy-Item -Recurse "$tmpClone\.claude\skills" "$script:NtntHome\.claude\" -Force
                }
                if (Test-Path "$tmpClone\CLAUDE.md") { Copy-Item "$tmpClone\CLAUDE.md" $script:NtntHome -Force }
                Remove-Item -Recurse -Force $tmpClone -ErrorAction SilentlyContinue
            } else {
                throw "Clone failed"
            }
        } catch {
            Remove-Item -Recurse -Force $tmpClone -ErrorAction SilentlyContinue
            Write-Host "Could not download starter kit. You can browse docs at:" -ForegroundColor Yellow
            Write-Host "  https://github.com/$script:Repo"
            return $false
        }
    }

    Write-Host "[OK] Starter kit downloaded to .\ntnt\" -ForegroundColor Green
    Write-Host ""
    Write-Host "  .\ntnt\docs\              - Documentation"
    Write-Host "  .\ntnt\examples\          - Example projects"
    Write-Host "  .\ntnt\CLAUDE.md          - Claude Code instructions"
    Write-Host "  .\ntnt\.claude\skills\    - Claude Code skills (IDD workflow)"
    Write-Host "  .\ntnt\.github\copilot-instructions.md  - GitHub Copilot instructions"
    return $true
}

# For binary installs, always download the starter kit
# For source installs, everything is already in ntnt\
$script:NtntHome = $null
if ($script:InstalledFrom -eq "binary") {
    Download-StarterKit | Out-Null
}

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
if ($script:InstalledFrom -eq "source") {
    Write-Host "Examples:    .\ntnt\examples\"
    Write-Host "Docs:        .\ntnt\docs\"
    Write-Host "Agent guide: .\ntnt\CLAUDE.md"
} elseif ($script:NtntHome -and (Test-Path $script:NtntHome)) {
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
