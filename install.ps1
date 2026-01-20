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
    $ntntDir = Join-Path (Get-Location) "ntnt-src"

    try {
        if (Test-Path $ntntDir) {
            Write-Host "Updating NTNT source in .\ntnt-src..."
            Push-Location $ntntDir
            git fetch --quiet origin 2>$null
            git reset --quiet --hard origin/main 2>$null
            git clean --quiet -fd 2>$null
        } else {
            Write-Host "Cloning NTNT source to .\ntnt-src..."
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

# Offer to download examples (only for binary installs)
$script:ExamplesDir = $null
if ($script:InstalledFrom -eq "binary") {
    Write-Host ""
    $response = Read-Host "Would you like to download the examples? (y/n)"
    if ($response -eq "y" -or $response -eq "Y") {
        $script:ExamplesDir = Join-Path (Get-Location) "ntnt-examples"
        Write-Host ""
        Write-Host "Downloading examples to .\ntnt-examples..."

        try {
            # Try sparse checkout first
            $null = git clone --depth 1 --filter=blob:none --sparse "https://github.com/$script:Repo.git" $script:ExamplesDir 2>&1
            if ($LASTEXITCODE -eq 0) {
                Push-Location $script:ExamplesDir
                $null = git sparse-checkout set examples 2>&1
                # Move examples to root and clean up
                if (Test-Path "examples") {
                    Get-ChildItem -Path "examples" | Move-Item -Destination . -Force
                    Remove-Item -Path "examples" -Recurse -Force -ErrorAction SilentlyContinue
                }
                Remove-Item -Path ".git" -Recurse -Force -ErrorAction SilentlyContinue
                Pop-Location
                Write-Host "[OK] Examples downloaded to .\ntnt-examples" -ForegroundColor Green
            } else {
                throw "Sparse checkout failed"
            }
        } catch {
            # Fallback: full shallow clone
            Remove-Item -Path $script:ExamplesDir -Recurse -Force -ErrorAction SilentlyContinue
            try {
                $null = git clone --depth 1 "https://github.com/$script:Repo.git" $script:ExamplesDir 2>&1
                if ($LASTEXITCODE -eq 0 -and (Test-Path "$script:ExamplesDir\examples")) {
                    # Keep only examples
                    Get-ChildItem -Path "$script:ExamplesDir\examples" | Move-Item -Destination $script:ExamplesDir -Force
                    Remove-Item -Path "$script:ExamplesDir\examples" -Recurse -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\src" -Recurse -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\tests" -Recurse -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\.git" -Recurse -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\.github" -Recurse -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\Cargo.toml" -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\Cargo.lock" -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\*.md" -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\*.sh" -Force -ErrorAction SilentlyContinue
                    Remove-Item -Path "$script:ExamplesDir\*.ps1" -Force -ErrorAction SilentlyContinue
                    Write-Host "[OK] Examples downloaded to .\ntnt-examples" -ForegroundColor Green
                } else {
                    throw "Clone failed"
                }
            } catch {
                $script:ExamplesDir = $null
                Write-Host "Could not download examples. You can browse them at:" -ForegroundColor Yellow
                Write-Host "  https://github.com/$script:Repo/tree/main/examples"
            }
        }
    }
}

Write-Host ""
Write-Host "Get started:"
Write-Host "  ntnt run hello.tnt     # Run a file" -ForegroundColor Cyan
Write-Host "  ntnt --help            # See all commands" -ForegroundColor Cyan
Write-Host ""
if ($script:InstalledFrom -eq "source") {
    Write-Host "Examples: .\ntnt-src\examples\"
} elseif ($script:ExamplesDir -and (Test-Path $script:ExamplesDir)) {
    Write-Host "Examples: .\ntnt-examples\"
} else {
    Write-Host "Examples: https://github.com/$script:Repo/tree/main/examples"
}
Write-Host "Docs: https://github.com/$script:Repo"
Write-Host ""

Wait-AndExit
