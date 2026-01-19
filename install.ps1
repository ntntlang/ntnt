# NTNT Language Installer for Windows
# Usage: irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "🚀 Installing NTNT Language..." -ForegroundColor Cyan
Write-Host ""

# Check for Visual Studio Build Tools (C++ compiler)
$hasCompiler = (Get-Command cl -ErrorAction SilentlyContinue) -or
               (Test-Path "${env:ProgramFiles}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\*\cl.exe") -or
               (Test-Path "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\*\cl.exe")

if (-not $hasCompiler) {
    # Check if we can compile by trying rustc (it will fail fast if no linker)
    $testResult = & rustc --version 2>&1
    if ($LASTEXITCODE -ne 0 -or $testResult -match "linker") {
        Write-Host "❌ Visual Studio Build Tools not found" -ForegroundColor Red
        Write-Host ""
        Write-Host "NTNT requires C++ build tools to compile on Windows."
        Write-Host ""
        Write-Host "Install them by:"
        Write-Host ""
        Write-Host "  1. Download Visual Studio Build Tools:" -ForegroundColor Green
        Write-Host "     https://visualstudio.microsoft.com/visual-cpp-build-tools/"
        Write-Host ""
        Write-Host "  2. Run the installer and select:" -ForegroundColor Green
        Write-Host '     "Desktop development with C++"'
        Write-Host ""
        Write-Host "After installation completes, re-run this installer."
        Write-Host ""
        exit 1
    }
}
Write-Host "✓ Build tools found" -ForegroundColor Green

# Check for Git
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host ""
    Write-Host "❌ Git not found" -ForegroundColor Red
    Write-Host ""
    Write-Host "NTNT requires git to download the source code."
    Write-Host ""
    Write-Host "Install it from:" -ForegroundColor Green
    Write-Host "  https://git-scm.com/download/win"
    Write-Host ""
    Write-Host "After installation completes, re-run this installer."
    Write-Host ""
    exit 1
}

# Check for Rust/Cargo
$cargoPath = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$hasRust = (Get-Command cargo -ErrorAction SilentlyContinue) -or (Test-Path $cargoPath)

if (-not $hasRust) {
    Write-Host "Rust not found. Installing via rustup..." -ForegroundColor Yellow
    Write-Host ""

    # Download and run rustup-init
    $rustupInit = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit
    Start-Process -FilePath $rustupInit -ArgumentList "-y" -Wait -NoNewWindow
    Remove-Item $rustupInit -ErrorAction SilentlyContinue

    # Add cargo to current session PATH
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

    Write-Host "✓ Rust installed" -ForegroundColor Green
} else {
    $rustVersion = & rustc --version 2>$null
    Write-Host "✓ Rust found: $rustVersion" -ForegroundColor Green
}

# Clone or update repo in current directory
$ntntDir = Join-Path (Get-Location) "ntnt-src"

if (Test-Path $ntntDir) {
    Write-Host "Updating NTNT source in .\ntnt-src..."
    Push-Location $ntntDir
    # Always reset to match remote (handles force pushes, conflicts, etc.)
    git fetch --quiet origin
    git reset --quiet --hard origin/main
    git clean --quiet -fd
} else {
    Write-Host "Downloading NTNT source to .\ntnt-src..."
    git clone --quiet https://github.com/ntntlang/ntnt.git $ntntDir
    Push-Location $ntntDir
}

# Build and install
Write-Host "Building and installing to ~\.cargo\bin\ntnt.exe..."
& cargo install --path . --locked --quiet

Pop-Location

Write-Host ""
Write-Host "✓ NTNT installed successfully!" -ForegroundColor Green
Write-Host ""

# Show version
$ntntExe = "$env:USERPROFILE\.cargo\bin\ntnt.exe"
if (Test-Path $ntntExe) {
    $version = & $ntntExe --version 2>$null
    Write-Host "Version: $version"
}
Write-Host ""

# Check if ntnt is in PATH
if (-not (Get-Command ntnt -ErrorAction SilentlyContinue)) {
    Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Yellow
    Write-Host "  To use 'ntnt' command, add cargo's bin directory to your PATH." -ForegroundColor Yellow
    Write-Host "  Run this command in PowerShell:" -ForegroundColor Yellow
    Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Yellow
    Write-Host ""
    Write-Host '  $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"' -ForegroundColor Green
    Write-Host ""
    Write-Host "  To make it permanent:"
    Write-Host '  [Environment]::SetEnvironmentVariable("PATH", "$env:USERPROFILE\.cargo\bin;$([Environment]::GetEnvironmentVariable(''PATH'', ''User''))", "User")' -ForegroundColor Green
    Write-Host ""
    Write-Host "  Or just restart your terminal."
    Write-Host ""
}

Write-Host "Get started:"
Write-Host "  ntnt run hello.tnt     # Run a file" -ForegroundColor Green
Write-Host "  ntnt --help            # See all commands" -ForegroundColor Green
Write-Host ""
Write-Host "Examples are available in .\ntnt-src\examples\"
Write-Host "  dir ntnt-src\examples\" -ForegroundColor Green
Write-Host ""
Write-Host "Learn more: https://github.com/ntntlang/ntnt"
Write-Host ""
