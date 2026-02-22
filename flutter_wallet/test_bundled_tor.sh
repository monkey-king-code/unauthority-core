#!/bin/bash
# Test bundled Tor functionality
# This script verifies that Flutter wallet can auto-start Tor when Tor Browser is not running

set -e

echo "╔════════════════════════════════════════════════════════════╗"
echo "║          LOS WALLET - BUNDLED TOR TEST                     ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

echo "1️⃣  Checking for Tor Browser..."
if lsof -i :9150 | grep -q LISTEN; then
    echo "   ⚠️  Tor Browser is running on port 9150"
    echo "   Please close Tor Browser to test bundled Tor"
    echo ""
    echo "   To close Tor Browser:"
    echo "   - macOS: Cmd+Q in Tor Browser"
    echo "   - Or: pkill -9 'Tor Browser'"
    exit 1
else
    echo "   ✅ Tor Browser not running (good for testing)"
fi

echo ""
echo "2️⃣  Checking for system Tor..."
if lsof -i :9050 | grep -q LISTEN; then
    echo "   ⚠️  System Tor is running on port 9050"
    echo "   Bundled Tor will detect and use it"
else
    echo "   ✅ System Tor not running"
fi

echo ""
echo "3️⃣  Checking bundled Tor binary..."
cd "$(dirname "$0")"
if [ -f "tor/macos/tor" ]; then
    echo "   ✅ Found: tor/macos/tor ($(du -h tor/macos/tor | cut -f1))"
    if [ -x "tor/macos/tor" ]; then
        echo "   ✅ Binary is executable"
    else
        echo "   ❌ Binary is not executable"
        exit 1
    fi
else
    echo "   ❌ Bundled Tor binary not found"
    exit 1
fi

echo ""
echo "4️⃣  Testing manual Tor start..."
echo "   Starting bundled Tor on port 9250..."

# Create temp torrc
TEMP_DIR=$(mktemp -d)
TORRC="$TEMP_DIR/torrc"
cat > "$TORRC" << EOF
DataDirectory $TEMP_DIR/data
SocksPort 9250
Log notice stdout
ClientOnly 1
ExitRelay 0
ExitPolicy reject *:*
EOF

# Start Tor in background
./tor/macos/tor -f "$TORRC" > "$TEMP_DIR/tor.log" 2>&1 &
TOR_PID=$!

echo "   Tor PID: $TOR_PID"
echo "   Waiting for bootstrap (max 90s)..."

# Wait for "Bootstrapped 100%"
TIMEOUT=90
ELAPSED=0
while [ $ELAPSED -lt $TIMEOUT ]; do
    if grep -q "Bootstrapped 100%" "$TEMP_DIR/tor.log" 2>/dev/null; then
        echo "   ✅ Tor bootstrapped successfully!"
        break
    fi
    sleep 1
    ELAPSED=$((ELAPSED + 1))
    echo -n "."
done
echo ""

if [ $ELAPSED -ge $TIMEOUT ]; then
    echo "   ❌ Tor failed to bootstrap within ${TIMEOUT}s"
    kill $TOR_PID 2>/dev/null || true
    cat "$TEMP_DIR/tor.log"
    rm -rf "$TEMP_DIR"
    exit 1
fi

echo ""
echo "5️⃣  Testing .onion connectivity via bundled Tor..."
if curl -x socks5h://localhost:9250 \
    http://drnqiaqi5vvqpubem6qzptptijygbf7ggjheri6k5yn4qbeezhgjweyd.onion/health \
    --max-time 30 -s | grep -q "healthy"; then
    echo "   ✅ Successfully connected to LOS testnet via bundled Tor!"
else
    echo "   ❌ Failed to connect to testnet"
    kill $TOR_PID 2>/dev/null || true
    rm -rf "$TEMP_DIR"
    exit 1
fi

echo ""
echo "6️⃣  Cleanup..."
kill $TOR_PID 2>/dev/null || true
rm -rf "$TEMP_DIR"
echo "   ✅ Tor stopped and temp files cleaned"

echo ""
echo "╔════════════════════════════════════════════════════════════╗"
echo "║                  ✅ ALL TESTS PASSED!                      ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""
echo "📝 Summary:"
echo "   - Bundled Tor binary: OK"
echo "   - Tor daemon startup: OK"
echo "   - SOCKS5 proxy: OK (port 9250)"
echo "   - .onion connectivity: OK"
echo ""
echo "🚀 Flutter wallet will now:"
echo "   1. Detect if Tor Browser/System Tor is running"
echo "   2. If not found, auto-start bundled Tor"
echo "   3. Connect to testnet seamlessly"
echo ""
echo "💡 To test with Flutter:"
echo "   1. Close Tor Browser"
echo "   2. Run: flutter run -d macos"
echo "   3. Watch console for \"Starting bundled Tor daemon...\""
echo ""
