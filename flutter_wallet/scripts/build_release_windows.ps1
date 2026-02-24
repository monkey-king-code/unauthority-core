# ══════════════════════════════════════════════════════════════════════════════
# LOS WALLET — WINDOWS RELEASE BUILD (PowerShell)
# ══════════════════════════════════════════════════════════════════════════════
#
# Builds a complete standalone installer for Windows that includes:
#   ✅ Flutter wallet desktop app (release build)
#   ✅ Dilithium5 native crypto library (.dll bundled)
#   ✅ Tor auto-install/download (handled at runtime)
#   ✅ All dependencies — friend just extracts and runs
#
# Usage:
#   .\scripts\build_release_windows.ps1
#
# Output:
#   release\LOS-Wallet-v2.0.0-windows-x64.zip
#
# Prerequisites:
#   - Flutter SDK installed
#   - Rust toolchain installed (rustup.rs)
#   - Visual Studio Build Tools (C++ workload)
# ══════════════════════════════════════════════════════════════════════════════

$ErrorActionPreference = "Stop"

# ── Configuration ────────────────────────────────────────────────────────────
$VERSION = "2.0.0"
$SCRIPT_DIR = Split-Path -Parent $MyInvocation.MyCommand.Definition
$WALLET_DIR = Split-Path -Parent $SCRIPT_DIR
$NATIVE_DIR = Join-Path $WALLET_DIR "native\los_crypto_ffi"
$RELEASE_DIR = Join-Path $WALLET_DIR "release"

Write-Host ""
Write-Host "═══════════════════════════════════════════════════════════════"
Write-Host "  🚀 LOS Wallet Release Build (Windows)"
Write-Host "═══════════════════════════════════════════════════════════════"
Write-Host "  Version:   $VERSION"
Write-Host "  Output:    $RELEASE_DIR\"
Write-Host "═══════════════════════════════════════════════════════════════"
Write-Host ""

# ── Check Prerequisites ─────────────────────────────────────────────────────
Write-Host "📋 Checking prerequisites..."

if (-not (Get-Command flutter -ErrorAction SilentlyContinue)) {
    Write-Host "❌ Flutter SDK not found. Install from https://flutter.dev"
    exit 1
}
$flutterVer = (flutter --version 2>&1 | Select-Object -First 1) -replace '.*Flutter\s+', '' -replace '\s.*', ''
Write-Host "   ✅ Flutter $flutterVer"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "❌ Rust/Cargo not found. Install from https://rustup.rs"
    exit 1
}
$cargoVer = (cargo --version) -replace 'cargo\s+', ''
Write-Host "   ✅ Cargo $cargoVer"
Write-Host ""

# ── Step 1: Build Native Dilithium5 Library ──────────────────────────────────
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "  Step 1/4: Compiling Dilithium5 native library..."
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

Push-Location $NATIVE_DIR
cargo build --release
Write-Host ""
Write-Host "🧪 Running crypto tests..."
cargo test --release -- --nocapture
Pop-Location

Write-Host "✅ Native library compiled and tested"
Write-Host ""

# ── Step 2: Build Flutter Desktop App ────────────────────────────────────────
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "  Step 2/4: Building Flutter desktop app (release)..."
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

Push-Location $WALLET_DIR
flutter pub get
flutter build windows --release
Pop-Location

Write-Host "✅ Flutter build complete"
Write-Host ""

# ── Step 3: Bundle Native Library ────────────────────────────────────────────
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "  Step 3/4: Bundling native crypto library..."
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$BUNDLE = Join-Path $WALLET_DIR "build\windows\x64\runner\Release"
$DLL_SRC = Join-Path $NATIVE_DIR "target\release\los_crypto_ffi.dll"

if (-not (Test-Path $DLL_SRC)) {
    Write-Host "❌ Native library not found: $DLL_SRC"
    exit 1
}

# Copy DLL next to the executable
Copy-Item $DLL_SRC $BUNDLE
Write-Host "   ✅ los_crypto_ffi.dll → $BUNDLE\"
Write-Host ""

# ── Step 4: Package for Distribution ────────────────────────────────────────
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "  Step 4/4: Packaging for distribution..."
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

New-Item -ItemType Directory -Force -Path $RELEASE_DIR | Out-Null

# Create README
@"
═══════════════════════════════════════════════════════════════
  LOS WALLET - TESTNET RELEASE (Windows)
  Unauthority Blockchain v1.0
═══════════════════════════════════════════════════════════════

INSTALLATION:
  1. Extract this archive to any folder
  2. Run: flutter_wallet.exe

FIRST RUN:
  The wallet will automatically:
  - Setup Tor connectivity (no manual install needed)
  - Generate your Dilithium5 quantum-secure wallet
  - Connect to the testnet node

FEATURES:
  ✅ Post-Quantum Cryptography (CRYSTALS-Dilithium5)
  ✅ Automatic Tor Connectivity (zero config)
  ✅ Send/Receive LOS tokens
  ✅ Burn LOS tokens (validator consensus)
  ✅ 24-word BIP39 seed phrase backup

NETWORK:
  The wallet connects to the testnet via Tor (.onion).
  All traffic is automatically routed through Tor.
"@ | Out-File -FilePath "$BUNDLE\README.txt" -Encoding utf8

$ZIP_NAME = "LOS-Wallet-v${VERSION}-windows-x64.zip"
$ZIP_PATH = Join-Path $RELEASE_DIR $ZIP_NAME

if (Test-Path $ZIP_PATH) { Remove-Item $ZIP_PATH }

Compress-Archive -Path "$BUNDLE\*" -DestinationPath $ZIP_PATH -Force

$zipSize = (Get-Item $ZIP_PATH).Length / 1MB
$zipSizeStr = "{0:N1} MB" -f $zipSize

Write-Host "   ✅ ZIP created: $ZIP_PATH ($zipSizeStr)"
Write-Host ""

# ── Done ─────────────────────────────────────────────────────────────────────
Write-Host "═══════════════════════════════════════════════════════════════"
Write-Host "  ✅ RELEASE BUILD COMPLETE"
Write-Host "═══════════════════════════════════════════════════════════════"
Write-Host ""
Write-Host "  Platform:     Windows x64"
Write-Host "  Version:      $VERSION"
Write-Host "  Installer:    $ZIP_PATH"
Write-Host "  Size:         $zipSizeStr"
Write-Host ""
Write-Host "  What's included:"
Write-Host "    ✅ Flutter desktop wallet app (release build)"
Write-Host "    ✅ Dilithium5 native crypto library (.dll bundled)"
Write-Host "    ✅ Tor auto-install/download (runtime)"
Write-Host "    ✅ Pre-configured testnet .onion connection"
Write-Host ""
Write-Host "  Send this file to your friend. They just:"
Write-Host "    1. Extract the .zip file"
Write-Host "    2. Run flutter_wallet.exe"
Write-Host "    3. Wallet auto-configures everything"
Write-Host ""
Write-Host "═══════════════════════════════════════════════════════════════"
