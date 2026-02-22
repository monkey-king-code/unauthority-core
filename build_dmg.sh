#!/bin/bash
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"

# ═══════════════════════════════════
# Build Validator DMG
# ═══════════════════════════════════
build_validator() {
  echo "=== Building Validator DMG ==="

  # Build los-node with mainnet feature flag (CHAIN_ID=1)
  echo "--- Building los-node (mainnet) ---"
  cd "$ROOT"
  cargo build --release -p los-node --features mainnet
  echo "✅ los-node mainnet binary ready"

  cd "$ROOT/flutter_validator"

  VERSION="1.0.12"
  APP="build/macos/Build/Products/Release/LOS Validator Node.app"
  FRAMEWORKS="$APP/Contents/Frameworks"

  # Ensure native lib + los-node are bundled
  mkdir -p "$FRAMEWORKS"
  cp native/los_crypto_ffi/target/release/liblos_crypto_ffi.dylib "$FRAMEWORKS/"
  cp "$ROOT/target/release/los-node" "$APP/Contents/MacOS/los-node"
  chmod +x "$APP/Contents/MacOS/los-node"

  # Create DMG
  rm -rf release/dmg_staging
  mkdir -p release/dmg_staging
  cp -R "$APP" release/dmg_staging/
  ln -s /Applications release/dmg_staging/Applications
  echo "LOS Validator Dashboard - Mainnet Release v${VERSION}" > release/dmg_staging/README.txt

  rm -f "release/LOS-Validator-${VERSION}-macos.dmg"
  hdiutil create \
    -volname "LOS Validator" \
    -srcfolder release/dmg_staging \
    -ov -format UDZO \
    "release/LOS-Validator-${VERSION}-macos.dmg"

  rm -rf release/dmg_staging
  echo "=== Validator DMG: release/LOS-Validator-${VERSION}-macos.dmg ==="
  ls -lh "release/LOS-Validator-${VERSION}-macos.dmg"
}

# ═══════════════════════════════════
# Build Wallet DMG
# ═══════════════════════════════════
build_wallet() {
  echo "=== Building Wallet DMG ==="
  cd "$ROOT/flutter_wallet"

  VERSION="1.0.12"
  APP="build/macos/Build/Products/Release/LOS Wallet.app"
  FRAMEWORKS="$APP/Contents/Frameworks"

  # Ensure native lib is bundled
  mkdir -p "$FRAMEWORKS"
  cp native/los_crypto_ffi/target/release/liblos_crypto_ffi.dylib "$FRAMEWORKS/"

  # Create DMG
  rm -rf release/dmg_staging
  mkdir -p release/dmg_staging
  cp -R "$APP" release/dmg_staging/
  ln -s /Applications release/dmg_staging/Applications
  echo "LOS Wallet - Mainnet Release v${VERSION}" > release/dmg_staging/README.txt

  rm -f "release/LOS-Wallet-${VERSION}-macos.dmg"
  hdiutil create \
    -volname "LOS Wallet" \
    -srcfolder release/dmg_staging \
    -ov -format UDZO \
    "release/LOS-Wallet-${VERSION}-macos.dmg"

  rm -rf release/dmg_staging
  echo "=== Wallet DMG: release/LOS-Wallet-${VERSION}-macos.dmg ==="
  ls -lh "release/LOS-Wallet-${VERSION}-macos.dmg"
}

case "${1:-all}" in
  validator) build_validator ;;
  wallet)    build_wallet ;;
  all)       build_validator; build_wallet ;;
  *) echo "Usage: $0 [validator|wallet|all]"; exit 1 ;;
esac

echo ""
echo "=== ALL DONE ==="
