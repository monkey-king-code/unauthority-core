#!/bin/bash
# ==============================================================================
# LOS Wallet — Build Native Dilithium5 Library
# ==============================================================================
# Compiles the Rust FFI crate and copies the dynamic library to the correct
# platform-specific location for Flutter desktop apps.
#
# Usage:
#   ./scripts/build_native.sh          # Release build
#   ./scripts/build_native.sh debug    # Debug build
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WALLET_DIR="$(dirname "$SCRIPT_DIR")"
NATIVE_DIR="$WALLET_DIR/native/los_crypto_ffi"

# Check if Rust is installed
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust/Cargo not found. Install from https://rustup.rs"
    exit 1
fi

# Determine build mode
BUILD_MODE="${1:-release}"
CARGO_FLAGS=""
TARGET_DIR="release"

if [ "$BUILD_MODE" = "debug" ]; then
    TARGET_DIR="debug"
    echo "🔧 Building in DEBUG mode..."
else
    CARGO_FLAGS="--release"
    echo "🚀 Building in RELEASE mode..."
fi

# Build the native library
echo ""
echo "📦 Compiling los-crypto-ffi (Dilithium5 FFI)..."
cd "$NATIVE_DIR"
cargo build $CARGO_FLAGS 2>&1

if [ $? -ne 0 ]; then
    echo "❌ Build failed!"
    exit 1
fi

echo "✅ Build successful!"
echo ""

# Determine platform and library name
OS="$(uname -s)"
case "$OS" in
    Darwin)
        LIB_NAME="liblos_crypto_ffi.dylib"
        PLATFORM="macOS"
        # Copy to macOS Frameworks directory (for release builds)
        MACOS_DIR="$WALLET_DIR/macos/Runner"
        if [ -d "$MACOS_DIR" ]; then
            echo "📋 Copying $LIB_NAME to macOS app bundle..."
            cp "$NATIVE_DIR/target/$TARGET_DIR/$LIB_NAME" "$MACOS_DIR/" 2>/dev/null || true
        fi
        ;;
    Linux)
        LIB_NAME="liblos_crypto_ffi.so"
        PLATFORM="Linux"
        # Copy to Linux bundle directory
        LINUX_DIR="$WALLET_DIR/linux"
        if [ -d "$LINUX_DIR" ]; then
            echo "📋 Copying $LIB_NAME to Linux bundle..."
            cp "$NATIVE_DIR/target/$TARGET_DIR/$LIB_NAME" "$LINUX_DIR/" 2>/dev/null || true
        fi
        ;;
    MINGW*|MSYS*|CYGWIN*)
        LIB_NAME="los_crypto_ffi.dll"
        PLATFORM="Windows"
        ;;
    *)
        echo "⚠️  Unknown platform: $OS"
        LIB_NAME="liblos_crypto_ffi.so"
        PLATFORM="Unknown"
        ;;
esac

LIB_PATH="$NATIVE_DIR/target/$TARGET_DIR/$LIB_NAME"
LIB_SIZE=$(du -h "$LIB_PATH" | cut -f1)

echo ""
echo "═══════════════════════════════════════════════════"
echo "  ✅ LOS Crypto FFI Build Complete"
echo "═══════════════════════════════════════════════════"
echo "  Platform:  $PLATFORM"
echo "  Library:   $LIB_NAME"
echo "  Size:      $LIB_SIZE"
echo "  Path:      $LIB_PATH"
echo "═══════════════════════════════════════════════════"
echo ""
echo "The Flutter wallet will automatically detect this library."
echo "Run 'flutter run -d macos' (or linux/windows) to test."
echo ""

# Run Rust tests to verify the library works
echo "🧪 Running native library tests..."
cargo test $CARGO_FLAGS -- --nocapture 2>&1

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ All native tests passed!"
else
    echo ""
    echo "⚠️  Some tests failed — check output above"
fi
