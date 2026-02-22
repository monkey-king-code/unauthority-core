#!/usr/bin/env bash
# start.sh — Start a 4-validator local testnet for Unauthority (LOS)
# Usage: ./start.sh [testnet_level]
#   testnet_level: functional | consensus (default) | production
#
# All inter-node communication runs over Tor hidden services (.onion).
# Requires the testnet Tor instance at ~/.los-testnet-tor to be running.

set -euo pipefail

LEVEL="${1:-consensus}"
BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$BASE_DIR/target/release/los-node"
TOR_DIR="$HOME/.los-testnet-tor"
TOR_SOCKS_PORT=${LOS_TOR_SOCKS_PORT:-9050}

if [[ ! -f "$BINARY" ]]; then
    echo "❌ Binary not found. Build first: cargo build --release"
    exit 1
fi

# ── Verify Tor is running ────────────────────────────────────────────
if ! curl -s --socks5 "127.0.0.1:$TOR_SOCKS_PORT" --max-time 3 "http://check.torproject.org" >/dev/null 2>&1; then
    # Try basic SOCKS5 handshake as fallback check
    if ! nc -z 127.0.0.1 "$TOR_SOCKS_PORT" 2>/dev/null; then
        echo "❌ Testnet Tor not running on SOCKS port $TOR_SOCKS_PORT"
        echo "   Start it: tor -f $TOR_DIR/torrc &"
        exit 1
    fi
fi
echo "🧅 Tor SOCKS5 proxy: 127.0.0.1:$TOR_SOCKS_PORT"

# ── Read .onion addresses ────────────────────────────────────────────
declare -a ONIONS
for i in 1 2 3 4; do
    HS_DIR="$TOR_DIR/hs-validator-$i"
    if [[ ! -f "$HS_DIR/hostname" ]]; then
        echo "❌ Hidden service $i not found: $HS_DIR/hostname"
        echo "   Run setup_tor_testnet.sh first."
        exit 1
    fi
    ONIONS[$i]="$(cat "$HS_DIR/hostname")"
done

# ── Build .onion bootstrap list (P2P port 4001 on each hidden service) ──
BOOTSTRAP=""
for i in 1 2 3 4; do
    [[ -n "$BOOTSTRAP" ]] && BOOTSTRAP+=","
    BOOTSTRAP+="${ONIONS[$i]}:4001"
done

echo "🚀 Starting 4-validator Tor testnet (level: $LEVEL)"
echo "──────────────────────────────────────────────────"

# ── Extract seed phrases from TESTNET genesis ────────────────────────
# Uses testnet-genesis/testnet_wallets.json (NOT mainnet genesis_config.json!)
# Wallets[0-1] = DevTreasury, Wallets[2-5] = BootstrapNode(1-4)
TESTNET_GENESIS="$BASE_DIR/testnet-genesis/testnet_wallets.json"
declare -a SEEDS
if [[ -f "$TESTNET_GENESIS" ]]; then
    for i in 0 1 2 3; do
        IDX=$((i + 2))  # offset: skip 2 DevTreasury wallets
        SEEDS[$((i+1))]="$(python3 -c "import json; print(json.load(open('$TESTNET_GENESIS'))['wallets'][$IDX].get('seed_phrase',''))" 2>/dev/null || true)"
    done
else
    echo "⚠️  Testnet genesis not found: $TESTNET_GENESIS"
    echo "   Validators will generate random keypairs (non-deterministic)"
fi

for i in 1 2 3 4; do
    PORT=$((3029 + i))
    # P2P local listen port (Tor maps external 4001 → local 400$i)
    P2P_LOCAL_PORT=$((4000 + i))
    NODE_DIR="node_data/v${i}"
    NODE_ID="validator-${i}"
    PID_FILE="$NODE_DIR/pid.txt"

    # Check if already running
    if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        echo "⏭️  Validator $i already running (PID $(cat "$PID_FILE"))"
        continue
    fi

    mkdir -p "$NODE_DIR"

    # Clean stale PID file if process is dead
    rm -f "$PID_FILE"

    # Clean stale database lock if the previous process died uncleanly.
    # sled uses flock() which is auto-released on process death, but
    # sometimes macOS holds it if the process entered UE (Uninterruptible Exit).
    # The DB directory itself is fine — only the flock is stale.
    DB_DIR="$NODE_DIR/los_database"
    if [[ -d "$DB_DIR" ]]; then
        # Verify no process actually holds the lock
        if ! fuser "$DB_DIR" >/dev/null 2>&1; then
            # No process has it open — safe to proceed
            :
        fi
    fi

    # Seed phrase from genesis for deterministic identity across restarts
    SEED="${SEEDS[$i]:-}"

    # Export env vars for the node process
    (
        export LOS_NODE_ID="$NODE_ID"
        export LOS_TESTNET_LEVEL="$LEVEL"
        export LOS_BOOTSTRAP_NODES="$BOOTSTRAP"
        export LOS_ONION_ADDRESS="${ONIONS[$i]}"
        export LOS_SOCKS5_PROXY="socks5h://127.0.0.1:$TOR_SOCKS_PORT"
        export LOS_P2P_PORT="$P2P_LOCAL_PORT"
        [[ -n "$SEED" ]] && export LOS_SEED_PHRASE="$SEED"

        nohup "$BINARY" --port "$PORT" --data-dir "$NODE_DIR" --node-id "$NODE_ID" \
            </dev/null > "$NODE_DIR/node.log" 2>&1 &
        echo $! > "$PID_FILE"
    )

    echo "✅ V$i — 🧅 ${ONIONS[$i]}"
    echo "        local: 127.0.0.1:$PORT (API) / $P2P_LOCAL_PORT (P2P)"
done

echo "──────────────────────────────────────────────────"
echo "🧅 Tor API endpoints (via SOCKS5 $TOR_SOCKS_PORT):"
for i in 1 2 3 4; do
    echo "   V$i: http://${ONIONS[$i]}"
done
echo ""
echo "🔧 Local API (dev only):"
for i in 1 2 3 4; do
    echo "   V$i: http://127.0.0.1:$((3029 + i))"
done
echo ""
echo "🛑 Stop with: ./stop.sh"
