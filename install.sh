#!/usr/bin/env bash
# install.sh — Build Unauthority (LOS) from source
# Usage: ./install.sh [--mainnet]

set -euo pipefail

echo "🔧 Unauthority (LOS) — Build from Source"
echo "──────────────────────────────────────────"

# Check Rust
if ! command -v cargo &>/dev/null; then
    echo "❌ Rust not found. Install from https://rustup.rs"
    exit 1
fi

echo "✅ Rust: $(rustc --version)"

# Check/Install Tor
if ! command -v tor &>/dev/null; then
    echo ""
    echo "⚠️  Tor is not installed. Tor is REQUIRED for network connectivity."
    echo ""
    if [[ "$(uname -s)" == "Linux" ]]; then
        echo "Install Tor:"
        echo "  sudo apt update && sudo apt install -y tor"
        echo "  sudo systemctl enable --now tor"
    elif [[ "$(uname -s)" == "Darwin" ]]; then
        echo "Install Tor:"
        echo "  brew install tor"
        echo "  brew services start tor"
    fi
    echo ""
    echo "After installing Tor, re-run this script."
    exit 1
fi
echo "✅ Tor:  $(tor --version | head -1)"

# Verify Tor SOCKS5 proxy is reachable
if nc -z 127.0.0.1 9050 2>/dev/null; then
    echo "✅ Tor SOCKS5 proxy: 127.0.0.1:9050 (auto-detected)"
else
    echo "⚠️  Tor SOCKS5 proxy not reachable at 127.0.0.1:9050"
    echo "   Make sure Tor is running: sudo systemctl start tor"
fi

if [[ "${1:-}" == "--mainnet" ]]; then
    echo ""
    echo "🏗️  Building MAINNET binary..."
    cargo build --release -p los-node -p los-cli --features los-core/mainnet
    echo ""
    echo "✅ Mainnet build complete!"
    echo "   Binary: target/release/los-node"
    echo "   CLI:    target/release/los-cli"
    echo ""
    echo "──────────────────────────────────────────"
    echo "🚀 Quick Start (just 2 env vars!):"
    echo ""
    echo "   export LOS_WALLET_PASSWORD='your-strong-password'"
    echo "   ./target/release/los-node --port 3030 --data-dir /opt/los-node"
    echo ""
    echo "   The node will automatically:"
    echo "   ✅ Discover bootstrap peers from genesis config"
    echo "   ✅ Detect Tor SOCKS5 proxy at 127.0.0.1:9050"
    echo "   ✅ Connect to the Unauthority network via Tor"
    echo "──────────────────────────────────────────"
else
    echo ""
    echo "🏗️  Building TESTNET binary..."
    cargo build --release
    echo ""
    echo "✅ Testnet build complete!"
    echo "   Binary: target/release/los-node"
    echo "   CLI:    target/release/los-cli"
    echo ""
    echo "🚀 Quick start: ./start.sh"
fi
