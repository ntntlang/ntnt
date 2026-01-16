# NTNT Language Installer for Windows
# Usage: irm https://raw.githubusercontent.com/ntntlang/ntnt/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "🚀 Installing NTNT Language..." -ForegroundColor Cyan
Write-Host ""

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

# Check for Git
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host ""
    Write-Host "❌ Git is required but not found." -ForegroundColor Red
    Write-Host ""
    Write-Host "Please install Git from: https://git-scm.com/download/win" -ForegroundColor Yellow
    Write-Host "Then run this installer again."
    Write-Host ""
    exit 1
}

# Clone or update repo
$ntntDir = "$env:USERPROFILE\.ntnt-src"

if (Test-Path $ntntDir) {
    Write-Host "Updating existing installation..."
    Push-Location $ntntDir
    git pull --quiet
} else {
    Write-Host "Downloading NTNT..."
    git clone --quiet https://github.com/ntntlang/ntnt.git $ntntDir
    Push-Location $ntntDir
}

# Build and install
Write-Host "Building NTNT (this may take a minute)..."
& cargo install --path . --locked --quiet

Pop-Location

Write-Host ""
Write-Host "✓ NTNT installed successfully!" -ForegroundColor Green
Write-Host ""

# Check if ntnt is accessible
$ntntExe = "$env:USERPROFILE\.cargo\bin\ntnt.exe"
if (Test-Path $ntntExe) {
    $version = & $ntntExe --version 2>$null
    Write-Host "Version: $version"
} else {
    Write-Host "Note: You may need to add cargo's bin directory to your PATH." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Run this in PowerShell to add it permanently:"
    Write-Host ""
    Write-Host '  [Environment]::SetEnvironmentVariable("PATH", "$env:USERPROFILE\.cargo\bin;$env:PATH", "User")' -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Then restart your terminal."
}

Write-Host ""
Write-Host "Get started:"
Write-Host '  Set-Content -Path hello.tnt -Value ''print("Hello, World!")'' -Encoding UTF8'
Write-Host "  ntnt run hello.tnt"
Write-Host ""
Write-Host "Learn more: https://github.com/ntntlang/ntnt"
Write-Host ""
