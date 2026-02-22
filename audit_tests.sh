#!/bin/bash
cd "$(dirname "$0")"
FILES=(
  crates/los-core/src/lib.rs
  crates/los-core/src/validator_rewards.rs
  crates/los-core/src/pow_mint.rs
  crates/los-core/src/oracle_consensus.rs
  crates/los-core/src/distribution.rs
  crates/los-core/src/bonding_curve.rs
  crates/los-core/src/validator_config.rs
  crates/los-consensus/src/voting.rs
  crates/los-consensus/src/abft.rs
  crates/los-consensus/src/slashing.rs
  crates/los-consensus/src/checkpoint.rs
  crates/los-vm/src/lib.rs
  crates/los-vm/src/usp01.rs
  crates/los-vm/src/oracle_connector.rs
  crates/los-vm/src/token_registry.rs
  crates/los-vm/src/dex_registry.rs
  crates/los-sdk/src/lib.rs
  crates/los-cli/src/main.rs
  crates/los-node/src/grpc_server.rs
  crates/los-node/src/mempool.rs
  crates/los-node/src/metrics.rs
  crates/los-node/src/genesis.rs
  crates/los-node/src/db.rs
  crates/los-node/src/rate_limiter.rs
  crates/los-node/src/main.rs
  crates/los-network/src/fee_scaling.rs
  crates/los-network/src/p2p_encryption.rs
  crates/los-network/src/p2p_integration.rs
  crates/los-network/src/slashing_integration.rs
  crates/los-network/src/validator_rewards.rs
  crates/los-crypto/src/lib.rs
)

for f in "${FILES[@]}"; do
  if [ -f "$f" ]; then
    line=$(grep -n '#\[cfg(test)\]' "$f" 2>/dev/null | head -1 | cut -d: -f1)
    total=$(wc -l < "$f" | tr -d ' ')
    echo "$f: test_start=$line total=$total"
  fi
done
