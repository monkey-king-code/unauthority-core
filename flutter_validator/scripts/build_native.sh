#!/bin/bash
# ==============================================================================
# LOS Validator — Build Native Dilithium5 Library (Cross-Platform)
# ==============================================================================
# Compiles the Rust FFI crate and copies the dynamic library to the correct
# platform-specific locations for Flutter desktop apps.
#
# Prerequisites:
#   - Rust toolchain (rustup.rs)
#   - cargo-zigbuild: `cargo install cargo-zigbuild`
#   - zig: `brew install zig` (macOS) or system package
#
# Usage:
#   ./scripts/build_native.sh              # Build for current platform (release)
#   ./scripts/build_native.sh debug        # Build for current platform (debug)
#   ./scripts/build_native.sh all          # Cross-compile for macOS + Linux + Windows
#   ./scripts/build_native.sh linux        # Cross-compile for Linux only
#   ./scripts/build_native.sh windows      # Cross-compile for Windows only
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VALIDATOR_DIR="$(dirname "$SCRIPT_DIR")"
NATIVE_DIR="$VALIDATOR_DIR/native/los_crypto_ffi"

# ─── Preflight checks ─────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
    echo "❌ Rust/Cargo not found. Install from https://rustup.rs"
    exit 1
fi

BUILD_ARG="${1:-native}"
CARGO_FLAGS="--release"
TARGET_DIR="release"
if [ "$BUILD_ARG" = "debug" ]; then
    CARGO_FLAGS=""
    TARGET_DIR="debug"
    BUILD_ARG="native"
    echo "🔧 Building in DEBUG mode..."
else
    echo "🚀 Building in RELEASE mode..."
fi

cd "$NATIVE_DIR"

# ─── Helper: build + copy ─────────────────────────────────────────
build_macos() {
    echo ""
    echo "🍎 Building macOS (arm64 + x86_64 universal)..."

    # Ensure both targets are installed
    rustup target add aarch64-apple-darwin x86_64-apple-darwin 2>/dev/null || true

    cargo build $CARGO_FLAGS --target aarch64-apple-darwin 2>&1
    cargo build $CARGO_FLAGS --target x86_64-apple-darwin  2>&1

    # Create universal binary
    lipo -create \
        "target/aarch64-apple-darwin/$TARGET_DIR/liblos_crypto_ffi.dylib" \
        "target/x86_64-apple-darwin/$TARGET_DIR/liblos_crypto_ffi.dylib" \
        -output "target/$TARGET_DIR/liblos_crypto_ffi.dylib"

    # Copy to macOS Runner (for app bundle Frameworks)
    cp "target/$TARGET_DIR/liblos_crypto_ffi.dylib" "$VALIDATOR_DIR/macos/Runner/" 2>/dev/null || true

    local size
    size=$(du -h "target/$TARGET_DIR/liblos_crypto_ffi.dylib" | cut -f1)
    echo "  ✅ macOS universal: liblos_crypto_ffi.dylib ($size)"
    lipo -info "target/$TARGET_DIR/liblos_crypto_ffi.dylib"
}

build_linux() {
    echo ""
    echo "🐧 Building Linux (x86_64 + aarch64)..."

    if ! command -v cargo-zigbuild &>/dev/null; then
        echo "  ⚠️  cargo-zigbuild not found — installing..."
        cargo install cargo-zigbuild
    fi
    if ! command -v zig &>/dev/null; then
        echo "  ❌ zig not found. Install: brew install zig (macOS) or apt install zig"
        return 1
    fi

    rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu 2>/dev/null || true

    cargo zigbuild $CARGO_FLAGS --target x86_64-unknown-linux-gnu  2>&1
    cargo zigbuild $CARGO_FLAGS --target aarch64-unknown-linux-gnu 2>&1

    # Copy to Linux bundle directory
    mkdir -p "$VALIDATOR_DIR/linux"
    cp "target/x86_64-unknown-linux-gnu/$TARGET_DIR/liblos_crypto_ffi.so" \
       "$VALIDATOR_DIR/linux/liblos_crypto_ffi.so" 2>/dev/null || true

    local x86_size arm_size
    x86_size=$(du -h "target/x86_64-unknown-linux-gnu/$TARGET_DIR/liblos_crypto_ffi.so" | cut -f1)
    arm_size=$(du -h "target/aarch64-unknown-linux-gnu/$TARGET_DIR/liblos_crypto_ffi.so" | cut -f1)
    echo "  ✅ Linux x86_64:  liblos_crypto_ffi.so ($x86_size)"
    echo "  ✅ Linux aarch64: liblos_crypto_ffi.so ($arm_size)"
}

build_windows() {
    echo ""
    echo "🪟 Building Windows (x86_64)..."

    if ! command -v cargo-zigbuild &>/dev/null; then
        echo "  ⚠️  cargo-zigbuild not found — installing..."
        cargo install cargo-zigbuild
    fi
    if ! command -v zig &>/dev/null; then
        echo "  ❌ zig not found. Install: brew install zig (macOS) or apt install zig"
        return 1
    fi

    rustup target add x86_64-pc-windows-gnu 2>/dev/null || true

    cargo zigbuild $CARGO_FLAGS --target x86_64-pc-windows-gnu 2>&1

    # Copy to Windows bundle directory
    mkdir -p "$VALIDATOR_DIR/windows"
    cp "target/x86_64-pc-windows-gnu/$TARGET_DIR/los_crypto_ffi.dll" \
       "$VALIDATOR_DIR/windows/los_crypto_ffi.dll" 2>/dev/null || true

    local size
    size=$(du -h "target/x86_64-pc-windows-gnu/$TARGET_DIR/los_crypto_ffi.dll" | cut -f1)
    echo "  ✅ Windows x86_64: los_crypto_ffi.dll ($size)"
}

build_native_only() {
    OS="$(uname -s)"
    case "$OS" in
        Darwin)
            build_macos
            ;;
        Linux)
            echo ""
            echo "🐧 Building Linux (native)..."
            cargo build $CARGO_FLAGS 2>&1
            mkdir -p "$VALIDATOR_DIR/linux"
            cp "target/$TARGET_DIR/liblos_crypto_ffi.so" "$VALIDATOR_DIR/linux/" 2>/dev/null || true
            local size
            size=$(du -h "target/$TARGET_DIR/liblos_crypto_ffi.so" | cut -f1)
            echo "  ✅ Linux native: liblos_crypto_ffi.so ($size)"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            echo ""
            echo "🪟 Building Windows (native)..."
            cargo build $CARGO_FLAGS 2>&1
            mkdir -p "$VALIDATOR_DIR/windows"
            cp "target/$TARGET_DIR/los_crypto_ffi.dll" "$VALIDATOR_DIR/windows/" 2>/dev/null || true
            local size
            size=$(du -h "target/$TARGET_DIR/los_crypto_ffi.dll" | cut -f1)
            echo "  ✅ Windows native: los_crypto_ffi.dll ($size)"
            ;;
        *)
            echo "⚠️  Unknown platform: $OS"
            exit 1
            ;;
    esac
}

# ─── Dispatch ──────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════"
echo "  LOS Validator — Dilithium5 Native Build"
echo "═══════════════════════════════════════════════════════"

case "$BUILD_ARG" in
    all)
        build_macos
        build_linux
        build_windows
        ;;
    linux)
        build_linux
        ;;
    windows)
        build_windows
        ;;
    macos|mac)
        build_macos
        ;;
    native)
        build_native_only
        ;;
    *)
        echo "Unknown build target: $BUILD_ARG"
        echo "Usage: $0 [all|native|macos|linux|windows|debug]"
        exit 1
        ;;
esac

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  ✅ Build Complete"
echo "═══════════════════════════════════════════════════════"
echo ""

# Run tests (native platform only)
if [ "$BUILD_ARG" = "native" ] || [ "$BUILD_ARG" = "macos" ] || [ "$BUILD_ARG" = "mac" ]; then
    echo "🧪 Running native library tests..."
    cargo test $CARGO_FLAGS -- --nocapture 2>&1
    if [ $? -eq 0 ]; then
        echo "✅ All native tests passed!"
    else
        echo "⚠️  Some tests failed — check output above"
    fi
fi
