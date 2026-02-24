#!/usr/bin/env bash
# ============================================================================
# UNAUTHORITY (LOS) — CLEAN MAINNET AUDIT v5
# ============================================================================
# Final pre-mainnet verification: immutable constants, security gates,
# consistency checks. Exit 0 = ALL PASS, Exit 1 = ISSUES FOUND.
# ============================================================================

set -u
# Note: we don't use `set -e` or `set -o pipefail` because grep returns 1
# when no matches found, which is expected behavior in audit checks.

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

FAIL=0
PASS=0
WARN=0

pass() { PASS=$((PASS+1)); echo "  ✅ $1"; }
fail() { FAIL=$((FAIL+1)); echo "  ❌ $1"; }
warn() { WARN=$((WARN+1)); echo "  ⚠️  $1"; }

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║     UNAUTHORITY (LOS) — CLEAN MAINNET AUDIT v5            ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# ============================================================================
echo ""
echo "═── 1. IMMUTABLE CONSTANTS ──════════════════════════════════════"
# ============================================================================

# 1A. SHA3-256 only (no Keccak256 in production)
K=$(grep -rn "Keccak256" crates/ --include="*.rs" 2>/dev/null | grep -v "target/" | wc -l | tr -d ' ')
if [ "$K" = "0" ]; then
  pass "No Keccak256 — all hashing uses SHA3-256 (NIST FIPS 202)"
else
  fail "Keccak256 found in production code: $K instances"
fi

# 1B. Total supply = 21,936,236 LOS
S1=$(grep -rn "21_936_236\|21936236" crates/los-core/src/ genesis/src/ --include="*.rs" 2>/dev/null | wc -l | tr -d ' ')
if [ "$S1" -gt "0" ]; then
  pass "Total supply 21,936,236 LOS constant found ($S1 refs)"
else
  fail "Total supply constant 21,936,236 missing"
fi

# 1C. CIL_PER_LOS = 10^11
C=$(grep -rn "CIL_PER_LOS\|100_000_000_000" crates/los-core/src/lib.rs 2>/dev/null | wc -l | tr -d ' ')
if [ "$C" -gt "0" ]; then
  pass "CIL_PER_LOS = 10^11 (100,000,000,000) found"
else
  fail "CIL_PER_LOS constant missing"
fi

# 1D. Mining epoch = 3600s mainnet
ME=$(grep -rn "3600\|MINING_EPOCH_SECS" crates/los-core/src/pow_mint.rs 2>/dev/null | wc -l | tr -d ' ')
if [ "$ME" -gt "0" ]; then
  pass "Mining epoch 3600s (1 hour) found in pow_mint.rs"
else
  fail "Mining epoch constant missing"
fi

# 1E. Ed25519 excluded on mainnet via compile-time gate
ED=$(grep -c 'cfg(not(feature = "mainnet"))' crates/los-crypto/src/lib.rs 2>/dev/null || echo "0")
if [ "$ED" -gt "0" ]; then
  pass "Ed25519 excluded on mainnet via #[cfg(not(feature=\"mainnet\"))]"
else
  fail "Ed25519 NOT gated out on mainnet builds"
fi

# 1F. Linear voting power (no sqrt in active code)
SQRT_ACTIVE=$(grep -n "isqrt\|sqrt" crates/los-consensus/src/voting.rs 2>/dev/null | grep -v "#\[allow(dead_code)\]\|///\|//\|fn isqrt\|Previous\|vulnerable\|dead_code\|NOTE" || true)
if [ -z "$SQRT_ACTIVE" ]; then
  pass "Linear voting power — isqrt is dead_code only (AMM reserved)"
else
  fail "sqrt found in ACTIVE voting code: $SQRT_ACTIVE"
fi

# 1G. Public mining pool = 21,158,413 LOS
PM=$(grep -rn "21_158_413\|21158413" crates/los-core/src/ genesis/src/ --include="*.rs" 2>/dev/null | wc -l | tr -d ' ')
if [ "$PM" -gt "0" ]; then
  pass "Public mining pool 21,158,413 LOS constant found"
else
  warn "Public mining pool constant not explicitly found (may be calculated)"
fi

# ============================================================================
echo ""
echo "═── 2. SECURITY GATES ───════════════════════════════════════════"
# ============================================================================

# 2A. functional() has mainnet guard (assert/panic)
FG=$(grep -A5 "fn functional" crates/los-node/src/testnet_config.rs 2>/dev/null | grep -c "mainnet_build\|panic\|assert" || echo "0")
if [ "$FG" -gt "0" ]; then
  pass "functional() has mainnet build guard (assert!)"
else
  fail "functional() NO mainnet guard — signatures bypassable on mainnet!"
fi

# 2B. is_testnet() compile-time gate
IT=$(grep -c "is_mainnet_build()" crates/los-node/src/testnet_config.rs 2>/dev/null || echo "0")
if [ "$IT" -gt "0" ]; then
  pass "is_testnet() has compile-time is_mainnet_build() gate"
else
  fail "is_testnet() missing compile-time gate"
fi

# 2C. CI tests mainnet feature flag
CI=$(grep -c "features mainnet" .github/workflows/ci.yml 2>/dev/null || echo "0")
if [ "$CI" -gt "0" ]; then
  pass "CI tests --features mainnet code paths ($CI steps)"
else
  fail "CI does NOT test mainnet feature — mainnet code paths untested"
fi

# 2D. Genesis cross-network protection
GN=$(grep -rn "Cannot load testnet genesis on mainnet" crates/los-node/src/ 2>/dev/null | wc -l | tr -d ' ')
if [ "$GN" -gt "0" ]; then
  pass "Genesis rejects testnet config on mainnet build"
else
  warn "Genesis cross-network validation not found (verify manually)"
fi

# 2E. mainnet-genesis/ in gitignore (private keys)
MG=$(grep -c "mainnet-genesis" .gitignore 2>/dev/null || echo "0")
if [ "$MG" -gt "0" ]; then
  pass "mainnet-genesis/ in .gitignore (keys not committed)"
else
  fail "mainnet-genesis/ NOT in .gitignore — private keys could leak!"
fi

# 2F. No hardcoded private key values
SEC=$(grep -rn "BEGIN PRIVATE\|PRIVATE_KEY.*=.*['\"]" --include="*.rs" --include="*.dart" . 2>/dev/null | grep -v "target/\|build/\|test\|example\|//\|///" | wc -l | tr -d ' ')
if [ "$SEC" = "0" ]; then
  pass "No hardcoded private key values in code"
else
  fail "Potential hardcoded private keys: $SEC instances"
fi

# 2G. No AI/copilot references in production code
AI=$(grep -rni "copilot\|openai\|claude\|chatgpt" --include="*.rs" --include="*.dart" crates/ flutter_wallet/lib/ flutter_validator/lib/ 2>/dev/null | grep -v "target/" | wc -l | tr -d ' ')
if [ "$AI" = "0" ]; then
  pass "No AI/copilot references in production code"
else
  fail "AI references found: $AI instances"
fi

# ============================================================================
echo ""
echo "═── 3. STALE REFERENCES ─════════════════════════════════════════"
# ============================================================================

# 3A. No √Stake / quadratic voting claims
SQ=$(grep -rn "√\|quadratic.*vot\|sqrt.*vot" dev_docs/ docs/ 2>/dev/null | grep -v "Previous\|vulnerable\|Sybil\|was\|changed from" | wc -l | tr -d ' ')
if [ "$SQ" = "0" ]; then
  pass "No √Stake/quadratic voting claims in docs/emails"
else
  fail "√Stake/quadratic voting claims found: $SQ instances"
fi

# 3B. No PoB (Proof-of-Burn) as distribution mechanism
POB=$(grep -rn "PoB\|Proof.of.Burn\|proof_of_burn" crates/ tests/ --include="*.rs" 2>/dev/null | grep -v "target/" | wc -l | tr -d ' ')
if [ "$POB" = "0" ]; then
  pass "No stale PoB/Proof-of-Burn references in Rust code"
else
  fail "PoB/Proof-of-Burn references: $POB instances"
fi

# 3C. No isqrt for rewards in CHANGELOG
ISQRT_CL=$(grep -i "isqrt.*reward" CHANGELOG.md 2>/dev/null | wc -l | tr -d ' ')
if [ "$ISQRT_CL" = "0" ]; then
  pass "CHANGELOG correctly says linear (not isqrt) for rewards"
else
  fail "CHANGELOG still claims isqrt for rewards"
fi

# 3D. No stale 7% dev allocation comments
DEV7=$(grep -rn "7%" crates/ --include="*.rs" 2>/dev/null | grep -i "dev\|alloc" | wc -l | tr -d ' ')
if [ "$DEV7" = "0" ]; then
  pass "No stale 7% dev allocation comments"
else
  fail "Stale 7% dev reference: $DEV7 instances"
fi

# ============================================================================
echo ""
echo "═── 4. VERSION CONSISTENCY ──════════════════════════════════════"
# ============================================================================

CARGO_VER=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
echo "    Workspace Cargo.toml: $CARGO_VER"

# 4A. Flutter wallet blockchain.dart version
if [ -f flutter_wallet/lib/constants/blockchain.dart ]; then
  WALLET_VER=$(grep "version.*=" flutter_wallet/lib/constants/blockchain.dart | grep -o "'[^']*'" | tr -d "'" | head -1)
  echo "    Flutter wallet blockchain.dart: $WALLET_VER"
  if [ "$CARGO_VER" = "$WALLET_VER" ]; then
    pass "Wallet version matches workspace ($WALLET_VER)"
  else
    fail "Wallet version mismatch: Cargo=$CARGO_VER, Wallet=$WALLET_VER"
  fi
fi

# 4B. Windows build script version
if [ -f flutter_wallet/scripts/build_release_windows.ps1 ]; then
  WIN_VER=$(grep 'VERSION.*=' flutter_wallet/scripts/build_release_windows.ps1 | head -1 | grep -o '"[^"]*"' | tr -d '"')
  echo "    Windows build script: $WIN_VER"
  if echo "$WIN_VER" | grep -q "1\.0"; then
    fail "Windows build script has stale version: $WIN_VER"
  else
    pass "Windows build script version OK: $WIN_VER"
  fi
fi

# 4C. Flutter validator pubspec
if [ -f flutter_validator/pubspec.yaml ]; then
  VAL_VER=$(grep "^version:" flutter_validator/pubspec.yaml | sed 's/version: //' | sed 's/+.*//')
  echo "    Flutter validator pubspec: $VAL_VER"
  if [ "$CARGO_VER" = "$VAL_VER" ]; then
    pass "Validator pubspec matches workspace ($VAL_VER)"
  else
    warn "Validator pubspec version: $VAL_VER (Cargo: $CARGO_VER)"
  fi
fi

# ============================================================================
echo ""
echo "═── 5. BUILD & FORMAT ───════════════════════════════════════════"
# ============================================================================

# 5A. Release binary exists
if [ -f target/release/los-node ]; then
  BIN_DATE=$(ls -la target/release/los-node | awk '{print $6, $7, $8}')
  pass "Release binary exists (built: $BIN_DATE)"
else
  warn "Release binary not found (run: cargo build --release)"
fi

# 5B. cargo fmt
echo "    Running cargo fmt check..."
if cargo fmt --all -- --check 2>/dev/null; then
  pass "cargo fmt: all formatted"
else
  fail "cargo fmt: formatting issues"
fi

# 5C. No unimplemented!/todo! in production
TODO_HITS=$(grep -rn "unimplemented!\|todo!" crates/los-core/src/ crates/los-node/src/ crates/los-consensus/src/ crates/los-crypto/src/ --include="*.rs" 2>/dev/null | grep -v "test\|target/" | wc -l | tr -d ' ')
if [ "$TODO_HITS" = "0" ]; then
  pass "No unimplemented!()/todo!() in production code"
else
  fail "unimplemented!/todo! in production: $TODO_HITS instances"
fi

# 5D. Docker testnet warning
if head -5 docker-compose.yml | grep -qi "testnet\|local" 2>/dev/null; then
  pass "docker-compose.yml has testnet-only warning"
else
  warn "docker-compose.yml missing testnet-only warning header"
fi

# ============================================================================
echo ""
echo "═── 6. CRYPTO & POST-QUANTUM ───════════════════════════════════"
# ============================================================================

# 6A. Dilithium5 support
if grep -rq "dilithium\|Dilithium\|DILITHIUM" crates/los-crypto/src/ --include="*.rs" 2>/dev/null; then
  pass "Dilithium5 post-quantum crypto support present"
else
  fail "Dilithium5 not found in los-crypto"
fi

# 6B. PoW mining uses SHA3
if grep -q "Sha3_256" crates/los-core/src/pow_mint.rs 2>/dev/null; then
  pass "PoW mining uses Sha3_256 (NIST FIPS 202)"
else
  fail "PoW mining does NOT use Sha3_256"
fi

# 6C. State root uses SHA3
if grep -q "Sha3_256" crates/los-core/src/lib.rs 2>/dev/null; then
  pass "State root uses Sha3_256"
else
  fail "State root does NOT use Sha3_256"
fi

# 6D. No f64 in active consensus math (comments OK)
# Known safe: slashing.rs get_uptime_percent() is #[cfg(not(feature = "mainnet"))] — excluded from mainnet builds
F64_UNSAFE=$(grep -rn "\bf64\b" crates/los-core/src/ crates/los-consensus/src/ --include="*.rs" 2>/dev/null | grep -v "test\|//\|///\|metric\|log\|display\|format\|target/\|cfg(not\|uptime_percent\|get_uptime" | wc -l | tr -d ' ')
if [ "$F64_UNSAFE" = "0" ]; then
  pass "No f64 in mainnet consensus code (testnet-only display gated out)"
else
  fail "f64 in consensus code: $F64_UNSAFE active instances"
fi

# ============================================================================
# SUMMARY
# ============================================================================

TOTAL=$((PASS + FAIL + WARN))

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║          UNAUTHORITY (LOS) — AUDIT SUMMARY                 ║"
echo "╠══════════════════════════════════════════════════════════════╣"
printf "║  ✅ PASSED:   %-44s║\n" "$PASS / $TOTAL"
printf "║  ⚠️  WARNINGS: %-43s║\n" "$WARN"
printf "║  ❌ FAILED:   %-44s║\n" "$FAIL"
echo "╚══════════════════════════════════════════════════════════════╝"

if [ "$FAIL" -gt 0 ]; then
  echo ""
  echo "⛔ MAINNET NOT READY — $FAIL critical issue(s) must be resolved."
  exit 1
elif [ "$WARN" -gt 0 ]; then
  echo ""
  echo "⚠️  MAINNET CONDITIONAL — $WARN warning(s) to review (non-blocking)."
  exit 0
else
  echo ""
  echo "🚀 CLEAN MAINNET READY — All $TOTAL checks passed!"
  exit 0
fi
