#!/bin/bash
# ══════════════════════════════════════════════════════════════════════════════
# LOS WALLET — ONE-CLICK RELEASE BUILD
# ══════════════════════════════════════════════════════════════════════════════
#
# Builds a complete, standalone installer for macOS (.dmg) or Linux (.tar.gz)
# that includes:
#   ✅ Flutter wallet desktop app (release build)
#   ✅ Dilithium5 native crypto library (compiled & bundled)
#   ✅ Tor auto-install/download (handled at runtime by the app)
#   ✅ All dependencies — friend just installs and runs
#
# Usage:
#   ./scripts/build_release.sh             # Build for current platform
#   ./scripts/build_release.sh macos       # Force macOS build
#   ./scripts/build_release.sh linux       # Force Linux build
#
# Output:
#   release/LOS-Wallet-v1.0.0-macos.dmg   (macOS)
#   release/LOS-Wallet-v1.0.0-linux.tar.gz (Linux)
#
# Prerequisites:
#   - Flutter SDK installed
#   - Rust toolchain installed (rustup.rs)
#   - macOS: Xcode command line tools
#   - Linux: clang, cmake, gtk3-dev, ninja-build
# ══════════════════════════════════════════════════════════════════════════════

set -e

# ── Configuration ────────────────────────────────────────────────────────────
NETWORK="${NETWORK:-testnet}"
if [ "$NETWORK" = "mainnet" ]; then
    VERSION="1.0.0"
else
    VERSION="1.0.0-testnet"
fi
APP_NAME="LOS Wallet"
BUNDLE_ID="com.unauthority.wallet"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WALLET_DIR="$(dirname "$SCRIPT_DIR")"
NATIVE_DIR="$WALLET_DIR/native/los_crypto_ffi"
RELEASE_DIR="$WALLET_DIR/release"

# ── Detect Platform ──────────────────────────────────────────────────────────
TARGET_PLATFORM="${1:-}"
if [ -z "$TARGET_PLATFORM" ]; then
    case "$(uname -s)" in
        Darwin) TARGET_PLATFORM="macos" ;;
        Linux)  TARGET_PLATFORM="linux" ;;
        *)      echo "❌ Unsupported platform: $(uname -s)"; exit 1 ;;
    esac
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  🚀 LOS Wallet Release Build"
echo "═══════════════════════════════════════════════════════════════"
echo "  Version:   $VERSION"
echo "  Platform:  $TARGET_PLATFORM"
echo "  Output:    $RELEASE_DIR/"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# ── Check Prerequisites ─────────────────────────────────────────────────────
echo "📋 Checking prerequisites..."

if ! command -v flutter &> /dev/null; then
    echo "❌ Flutter SDK not found. Install from https://flutter.dev"
    exit 1
fi
echo "   ✅ Flutter $(flutter --version 2>&1 | head -1 | awk '{print $2}')"

if ! command -v cargo &> /dev/null; then
    echo "❌ Rust/Cargo not found. Install from https://rustup.rs"
    exit 1
fi
echo "   ✅ Cargo $(cargo --version | awk '{print $2}')"

if [ "$TARGET_PLATFORM" = "macos" ]; then
    if ! command -v xcodebuild &> /dev/null; then
        echo "❌ Xcode command line tools not found"
        echo "   Run: xcode-select --install"
        exit 1
    fi
    echo "   ✅ Xcode CLT"
fi

echo ""

# ── Step 1: Build Native Dilithium5 Library ──────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 1/4: Compiling Dilithium5 native library..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$NATIVE_DIR"
cargo build --release 2>&1

# Run tests to verify
echo ""
echo "🧪 Running crypto tests..."
cargo test --release -- --nocapture 2>&1

echo "✅ Native library compiled and tested"
echo ""

# ── Step 2: Build Flutter Desktop App ────────────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 2/4: Building Flutter desktop app (release)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$WALLET_DIR"
flutter pub get
flutter build "$TARGET_PLATFORM" --release --dart-define=NETWORK="$NETWORK" 2>&1

echo "✅ Flutter build complete"
echo ""

# ── Step 3: Bundle Native Library Into App ───────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 3/4: Bundling native crypto library..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ "$TARGET_PLATFORM" = "macos" ]; then
    # ── macOS: Copy .dylib into .app/Contents/Frameworks/ ──
    APP_PATH="$WALLET_DIR/build/macos/Build/Products/Release/LOS Wallet.app"
    FRAMEWORKS_DIR="$APP_PATH/Contents/Frameworks"
    LIB_NAME="liblos_crypto_ffi.dylib"
    LIB_SRC="$NATIVE_DIR/target/release/$LIB_NAME"

    if [ ! -f "$LIB_SRC" ]; then
        echo "❌ Native library not found: $LIB_SRC"
        exit 1
    fi

    mkdir -p "$FRAMEWORKS_DIR"
    cp "$LIB_SRC" "$FRAMEWORKS_DIR/"

    # Fix dylib install name for macOS
    install_name_tool -id "@executable_path/../Frameworks/$LIB_NAME" \
        "$FRAMEWORKS_DIR/$LIB_NAME" 2>/dev/null || true

    echo "   ✅ $LIB_NAME → $APP_PATH/Contents/Frameworks/"

    # Re-sign the Frameworks (needed for macOS Gatekeeper)
    echo "   🔏 Re-signing app bundle..."
    codesign --force --deep --sign - "$APP_PATH" 2>/dev/null || true

elif [ "$TARGET_PLATFORM" = "linux" ]; then
    # ── Linux: Copy .so into bundle/lib/ ──
    BUNDLE_PATH="$WALLET_DIR/build/linux/x64/release/bundle"
    LIB_DIR="$BUNDLE_PATH/lib"
    LIB_NAME="liblos_crypto_ffi.so"
    LIB_SRC="$NATIVE_DIR/target/release/$LIB_NAME"

    if [ ! -f "$LIB_SRC" ]; then
        echo "❌ Native library not found: $LIB_SRC"
        exit 1
    fi

    mkdir -p "$LIB_DIR"
    cp "$LIB_SRC" "$LIB_DIR/"
    echo "   ✅ $LIB_NAME → $BUNDLE_PATH/lib/"
fi

echo ""

# ── Step 4: Package for Distribution ────────────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 4/4: Packaging for distribution..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

mkdir -p "$RELEASE_DIR"

if [ "$TARGET_PLATFORM" = "macos" ]; then
    # ── Create .dmg installer ──
    DMG_NAME="LOS-Wallet-v${VERSION}-macos.dmg"
    DMG_PATH="$RELEASE_DIR/$DMG_NAME"
    DMG_TEMP="$RELEASE_DIR/dmg_staging"

    rm -rf "$DMG_TEMP" "$DMG_PATH"
    mkdir -p "$DMG_TEMP"

    # Copy .app to staging
    cp -R "$APP_PATH" "$DMG_TEMP/"

    # Create Applications symlink (drag-to-install)
    ln -s /Applications "$DMG_TEMP/Applications"

    # Create README
    cat > "$DMG_TEMP/README.txt" << 'README'
╔══════════════════════════════════════════════╗
║         LOS WALLET - TESTNET RELEASE         ║
║         Unauthority Blockchain v1.0          ║
╚══════════════════════════════════════════════╝

INSTALLATION:
  Drag "LOS Wallet.app" to the Applications folder.

FIRST RUN:
  1. Open the app from Applications
  2. If blocked by macOS: System Settings → Privacy → Open Anyway
  3. The wallet will automatically:
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

SUPPORT:
  Contact the node operator for testnet access.
README

    # Create DMG
    echo "   📦 Creating $DMG_NAME..."
    hdiutil create \
        -volname "LOS Wallet" \
        -srcfolder "$DMG_TEMP" \
        -ov \
        -format UDZO \
        "$DMG_PATH" 2>&1

    # Cleanup staging
    rm -rf "$DMG_TEMP"

    DMG_SIZE=$(du -h "$DMG_PATH" | cut -f1)
    echo "   ✅ DMG created: $DMG_PATH ($DMG_SIZE)"

elif [ "$TARGET_PLATFORM" = "linux" ]; then
    # ── Create .tar.gz archive ──
    ARCHIVE_NAME="LOS-Wallet-v${VERSION}-linux-x64.tar.gz"
    ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
    STAGING="$RELEASE_DIR/staging"

    rm -rf "$STAGING" "$ARCHIVE_PATH"
    mkdir -p "$STAGING/los-wallet"

    # Copy bundle
    cp -R "$BUNDLE_PATH"/* "$STAGING/los-wallet/"

    # Create launcher script
    cat > "$STAGING/los-wallet/run.sh" << 'LAUNCHER'
#!/bin/bash
# LOS Wallet Launcher
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export LD_LIBRARY_PATH="${SCRIPT_DIR}/lib:${LD_LIBRARY_PATH}"
exec "${SCRIPT_DIR}/flutter_wallet" "$@"
LAUNCHER
    chmod +x "$STAGING/los-wallet/run.sh"

    # Create README
    cat > "$STAGING/los-wallet/README.txt" << 'LREADME'
═══════════════════════════════════════════════
  LOS WALLET - TESTNET RELEASE (Linux)
  Unauthority Blockchain v1.0
═══════════════════════════════════════════════

INSTALLATION:
  1. Extract this archive: tar xzf LOS-Wallet-*.tar.gz
  2. Run: ./los-wallet/run.sh

The wallet automatically handles Tor and crypto setup.
LREADME

    # Create archive
    echo "   📦 Creating $ARCHIVE_NAME..."
    cd "$STAGING"
    tar czf "$ARCHIVE_PATH" los-wallet/
    cd "$WALLET_DIR"

    # Cleanup
    rm -rf "$STAGING"

    ARCHIVE_SIZE=$(du -h "$ARCHIVE_PATH" | cut -f1)
    echo "   ✅ Archive created: $ARCHIVE_PATH ($ARCHIVE_SIZE)"
fi

# ── Done ─────────────────────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  ✅ RELEASE BUILD COMPLETE"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "  Platform:     $TARGET_PLATFORM"
echo "  Version:      $VERSION"

if [ "$TARGET_PLATFORM" = "macos" ]; then
    echo "  Installer:    $RELEASE_DIR/$DMG_NAME"
    echo "  Size:         $DMG_SIZE"
elif [ "$TARGET_PLATFORM" = "linux" ]; then
    echo "  Installer:    $RELEASE_DIR/$ARCHIVE_NAME"
    echo "  Size:         $ARCHIVE_SIZE"
fi

echo ""
echo "  What's included:"
echo "    ✅ Flutter desktop wallet app (release build)"
echo "    ✅ Dilithium5 native crypto library (bundled)"
echo "    ✅ Tor auto-install/download (runtime)"
echo "    ✅ Pre-configured testnet .onion connection"
echo ""
echo "  Send this file to your friend. They just:"
if [ "$TARGET_PLATFORM" = "macos" ]; then
    echo "    1. Open the .dmg file"
    echo "    2. Drag 'LOS Wallet' to Applications"
    echo "    3. Open from Applications (right-click → Open if blocked)"
    echo "    4. Wallet auto-configures everything"
elif [ "$TARGET_PLATFORM" = "linux" ]; then
    echo "    1. Extract: tar xzf LOS-Wallet-*.tar.gz"
    echo "    2. Run: ./los-wallet/run.sh"
    echo "    3. Wallet auto-configures everything"
fi
echo ""
echo "═══════════════════════════════════════════════════════════════"
