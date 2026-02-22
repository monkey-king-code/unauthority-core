#!/usr/bin/env bash
# stop.sh — Stop all running Unauthority validator nodes
# Reads PID files from node_data/v*/pid.txt
# Uses SIGTERM first, waits for clean exit, then SIGKILL as fallback.

set -euo pipefail

BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
STOPPED=0

for i in 1 2 3 4; do
    PID_FILE="$BASE_DIR/node_data/v${i}/pid.txt"
    DATA_DIR="$BASE_DIR/node_data/v${i}"

    if [[ -f "$PID_FILE" ]]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            # Phase 1: SIGTERM (graceful — flushes DB, removes PID)
            kill "$PID" 2>/dev/null || true
            # Wait up to 5s for clean exit
            for attempt in $(seq 1 10); do
                if ! kill -0 "$PID" 2>/dev/null; then
                    break
                fi
                sleep 0.5
            done
            # Phase 2: SIGKILL if still alive
            if kill -0 "$PID" 2>/dev/null; then
                kill -9 "$PID" 2>/dev/null || true
                sleep 0.5
                echo "🔪 Force-killed validator $i (PID $PID)"
            else
                echo "🛑 Stopped validator $i (PID $PID)"
            fi
            STOPPED=$((STOPPED + 1))
        else
            echo "⏭️  Validator $i not running (stale PID $PID)"
        fi
        rm -f "$PID_FILE"
    else
        echo "⏭️  Validator $i — no PID file found"
    fi
done

# Phase 3: Kill any orphaned los-node processes not tracked by PID files
ORPHANS=$(pgrep -f "los-node.*--data-dir" 2>/dev/null | grep -v "$$" || true)
if [[ -n "$ORPHANS" ]]; then
    echo "🧹 Killing orphaned los-node processes..."
    echo "$ORPHANS" | while read -r PID; do
        # Don't kill Flutter's embedded node
        if ! ps -p "$PID" -o command= 2>/dev/null | grep -q "flutter"; then
            kill -9 "$PID" 2>/dev/null || true
            echo "   Killed orphan PID $PID"
            STOPPED=$((STOPPED + 1))
        fi
    done
    sleep 1
fi

if [[ $STOPPED -eq 0 ]]; then
    echo "ℹ️  No running validators found"
else
    echo "✅ Stopped $STOPPED validator(s)"
fi
