# LOS Mining Guide — Public Supply Distribution

## What is LOS Mining?

LOS mining distributes **96.5% of the total supply** (21,158,413 LOS) to the public through Proof-of-Work. Anyone with a computer can mine. No special hardware required.

The mining algorithm uses SHA3-256, which is CPU-friendly — GPUs and ASICs have no significant advantage over standard CPUs. This ensures fair distribution.

---

## Quick Start (5 Minutes)

### 1. Download & Install

```bash
# Clone the repository
git clone https://github.com/monkey-king-code/unauthority-core.git
cd unauthority-core

# Build from source (requires Rust 1.75+)
cargo build --release --features mainnet -p los-node
```

### 2. Generate Your Wallet

Your wallet is automatically generated on first launch. Start the node and note your address:

```bash
export LOS_WALLET_PASSWORD='your-strong-password'
./target/release/los-node --port 3030
# Output:
#   🔑 New wallet created: LOS1a2b3c4d...
#   Keys saved to: node_data/node-3030/
```

Save your address — mining rewards go directly to this address.

### 3. Start Mining

```bash
./target/release/los-node --mine
```

That's it. Your node joins the network, syncs the blockchain, and starts mining automatically. When you find a valid proof, the reward is credited to your account within seconds.

---

## How Mining Works

### The Hash Puzzle

Every hour (1 epoch = 3,600 seconds), a new mining "round" begins. To mine:

1. Your node computes: `SHA3-256("LOS_MINE_V1" || chain_id || your_address || epoch || nonce)`
2. It increments `nonce` from a random starting point
3. When the hash has enough leading zero bits (≥ difficulty_bits), you found a valid proof
4. Your node broadcasts the proof to the network
5. Other validators verify the proof and credit your reward

The actual code that computes the hash:

```rust
// From: crates/los-core/src/pow_mint.rs

pub fn compute_mining_hash(address: &str, epoch: u64, nonce: u64) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(b"LOS_MINE_V1");           // Domain separator
    hasher.update(CHAIN_ID.to_le_bytes());    // Prevents cross-network replay
    hasher.update(address.as_bytes());        // Binds proof to your address
    hasher.update(epoch.to_le_bytes());       // Binds proof to current hour
    hasher.update(nonce.to_le_bytes());       // The value being ground
    
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}
```

### Why This is Fair

- **Address-bound:** The hash includes your address. Nobody can steal your proof.
- **Epoch-bound:** Each proof is only valid for the current hour. No stockpiling.
- **1 reward per address per epoch:** Mining multiple proofs in the same hour doesn't help. Sybil-neutral.
- **Random nonce start:** Each miner starts from a different random nonce, preventing wasted duplicate work.
- **No external oracle:** Everything is computed on-chain. No dependency on Bitcoin, Ethereum, or any external system.
- **CPU-friendly:** SHA3-256 runs efficiently on all CPUs. No GPU/ASIC advantage.

---

## Reward Schedule

### Initial Rate

| Parameter | Value |
|-----------|-------|
| Reward per epoch | **100 LOS** |
| Epoch duration | **1 hour** (3,600 seconds) |
| Maximum daily mining | ~2,400 LOS (across all miners) |
| Maximum yearly mining | ~876,000 LOS |

The 100 LOS per epoch is **split equally** among all miners who submit valid proofs in that epoch. If 5 miners succeed, each gets 20 LOS. If only 1 miner succeeds, they get the full 100 LOS.

### Halving Schedule

Mining rewards halve every **8,760 epochs** (approximately 1 year):

| Year | Reward/Epoch | Annual Mining Output | Cumulative |
|------|-------------|---------------------|------------|
| 1 | 100 LOS | ~876,000 LOS | 876,000 |
| 2 | 50 LOS | ~438,000 LOS | 1,314,000 |
| 3 | 25 LOS | ~219,000 LOS | 1,533,000 |
| 4 | 12.5 LOS | ~109,500 LOS | 1,642,500 |
| 5 | 6.25 LOS | ~54,750 LOS | 1,697,250 |
| ... | ... | ... | ... |
| 10 | ~0.195 LOS | ~1,710 LOS | ~1,748,000 |

The code that calculates the reward for any epoch:

```rust
// From: crates/los-core/src/pow_mint.rs

pub fn epoch_reward_cil(epoch: u64) -> u128 {
    let halving_interval = 8_760;  // ~1 year
    let halvings = epoch / halving_interval;
    if halvings >= 64 {
        return 0;  // Prevent overflow
    }
    MINING_REWARD_PER_EPOCH_CIL >> halvings  // Bitshift = divide by 2^halvings
}
```

### When Does Mining End?

Mining continues until the public supply of 21,158,413 LOS is exhausted. Due to halving, this takes many decades. The final rewards will be fractions of a CIL.

---

## Difficulty System

### How Difficulty Adjusts

Difficulty targets **10 successful miners per epoch**. After each epoch:

- **> 20 miners:** Difficulty increases by 1-4 bits (harder)
- **11-20 miners:** Difficulty increases by 1 bit
- **5-9 miners:** No change (target zone)
- **1-4 miners:** Difficulty decreases by 1 bit (easier)
- **0 miners:** Difficulty decreases by 2 bits (fast recovery)

Difficulty is bounded between **16 bits** (minimum) and **40 bits** (maximum).

```rust
// From: crates/los-core/src/pow_mint.rs — difficulty adjustment logic

if miners > TARGET_MINERS_PER_EPOCH * 2 {
    // Too many miners — increase difficulty
    let adjustment = ((miners / TARGET_MINERS_PER_EPOCH).ilog2() + 1)
        .min(MAX_DIFFICULTY_ADJUSTMENT_BITS);
    self.difficulty_bits = (self.difficulty_bits + adjustment)
        .min(MAX_MINING_DIFFICULTY_BITS);
} else if miners > TARGET_MINERS_PER_EPOCH {
    self.difficulty_bits = (self.difficulty_bits + 1)
        .min(MAX_MINING_DIFFICULTY_BITS);
} else if miners < TARGET_MINERS_PER_EPOCH / 2 && miners > 0 {
    self.difficulty_bits = self.difficulty_bits
        .saturating_sub(1)
        .max(MIN_MINING_DIFFICULTY_BITS);
} else if miners == 0 {
    self.difficulty_bits = self.difficulty_bits
        .saturating_sub(2)
        .max(MIN_MINING_DIFFICULTY_BITS);
}
```

### What Does Difficulty Mean?

| Difficulty (bits) | Average Hashes | Approx. Time (modern CPU) |
|----------------:|---------------:|-------------------------:|
| 16 | ~65,536 | < 0.1 second |
| 20 | ~1,048,576 | 0.5 - 2 seconds |
| 24 | ~16,777,216 | 8 - 30 seconds |
| 28 | ~268,435,456 | 2 - 8 minutes |
| 32 | ~4,294,967,296 | 30 - 120 minutes |
| 36 | ~68,719,476,736 | 8 - 32 hours |
| 40 | ~1,099,511,627,776 | 5 - 20 days |

Initial mainnet difficulty is **20 bits** (~1 million hashes, ~1 second on a modern CPU).

---

## Mining API

Your node exposes mining information at `GET /mining-info`:

```json
{
  "epoch": 1234,
  "difficulty_bits": 22,
  "reward_per_epoch_cil": 10000000000000,
  "remaining_supply_cil": 2115841200000000000000,
  "epoch_remaining_secs": 1847,
  "miners_this_epoch": 3,
  "chain_id": 1
}
```

| Field | Description |
|-------|-------------|
| `epoch` | Current mining epoch number |
| `difficulty_bits` | Required leading zero bits |
| `reward_per_epoch_cil` | Total reward this epoch in CIL |
| `remaining_supply_cil` | Public supply remaining |
| `epoch_remaining_secs` | Seconds until epoch ends |
| `miners_this_epoch` | Successful miners so far |
| `chain_id` | 1 = mainnet, 2 = testnet |

---

## Mining Requirements

### Minimum Setup
- A computer with any modern CPU (x86_64 or ARM64)
- Rust toolchain for building from source
- 1 LOS balance (for transaction fees to claim rewards)
- Internet connection (Tor recommended for privacy)

### Running as a Validator (Required)

Miners **must** run a full validator node. There is no separate mining client. This design ensures every miner contributes to network security:

```bash
# Mine + validate simultaneously
./target/release/los-node --mine
```

The mining thread runs in the background while your node validates blocks, participates in consensus, and serves the network.

### Mining with Tor (Recommended)

For maximum privacy, run your mining node behind Tor:

```bash
# Node auto-detects Tor SOCKS5 at 127.0.0.1:9050 and generates a .onion address
./target/release/los-node --mine
```

If Tor is installed and running, the node automatically detects the SOCKS5 proxy and routes all traffic through Tor. Your mining proofs are broadcast over the Tor network. Other participants cannot determine your IP address.

For manual `.onion` configuration, see the [Tor Setup Guide](TOR_SETUP.md).

---

## Verify Mining Proofs Yourself

Every mining proof is verifiable by anyone. Here's how to verify a proof manually:

```rust
use sha3::{Digest, Sha3_256};

fn verify(address: &str, epoch: u64, nonce: u64, difficulty_bits: u32) -> bool {
    let mut hasher = Sha3_256::new();
    hasher.update(b"LOS_MINE_V1");
    hasher.update(1u64.to_le_bytes());        // chain_id = 1 (mainnet)
    hasher.update(address.as_bytes());
    hasher.update(epoch.to_le_bytes());
    hasher.update(nonce.to_le_bytes());
    
    let hash = hasher.finalize();
    
    // Count leading zero bits
    let mut zeros = 0u32;
    for byte in hash.iter() {
        if *byte == 0 { zeros += 8; }
        else { zeros += byte.leading_zeros(); break; }
    }
    
    zeros >= difficulty_bits
}
```

---

## Supply Distribution Summary

| Allocation | LOS | Percentage |
|-----------|-----|-----------|
| **Public Mining Pool** | **21,158,413** | **96.45%** |
| Dev Treasury 1 | 428,113 | 1.95% |
| Dev Treasury 2 | 245,710 | 1.12% |
| Dev Treasury 3 | 50,000 | 0.23% |
| Dev Treasury 4 | 50,000 | 0.23% |
| Bootstrap Validators (4×1000, stake only) | 4,000 | 0.02% |
| **Total Supply** | **21,936,236** | **100%** |

**Bootstrap validators receive ZERO mining rewards and ZERO validator epoch rewards.** All rewards go exclusively to public participants.

---

## Bitcoin Mining vs LOS Mining

| | **Bitcoin (BTC)** | **LOS (Unauthority)** |
|---|---|---|
| **Algorithm** | SHA-256 (double hash) | SHA3-256 (single hash) |
| **Hardware** | ASIC-dominated (Antminer S21+) | CPU-only (SHA3 has no ASIC/GPU advantage) |
| **Entry Cost** | $2,000–$15,000+ per ASIC miner | $0 — any computer with a CPU |
| **Block Time** | ~10 minutes | 1 hour (epoch-based) |
| **Reward** | 3.125 BTC/block (2024) | 100 LOS/epoch, shared among miners |
| **Halving** | Every 210,000 blocks (~4 years) | Every 8,760 epochs (~1 year) |
| **Total Supply** | 21,000,000 BTC | 21,936,236 LOS |
| **Public Mining Pool** | ~100% (all mined) | 96.45% (21,158,413 LOS) |
| **Mining Pool** | Required (solo mining impractical) | Not possible — proofs are address-bound |
| **Deduplication** | None (unlimited blocks per miner) | 1 reward per address per epoch |
| **Difficulty Adjustment** | Every 2,016 blocks (~2 weeks) | Every epoch (1 hour), ±1-4 bits |
| **Finality** | ~60 minutes (6 confirmations) | ~2-3 seconds (aBFT consensus) |
| **Node Requirement** | Separate (mining ≠ full node) | Integrated (miners MUST run a full node) |
| **Energy** | ~150 TWh/year globally | Negligible (CPU-only, no ASIC farms) |
| **Quantum Resistance** | None (ECDSA) | Dilithium5 (Post-Quantum) |
| **Consensus** | Nakamoto PoW (longest chain) | aBFT + PoW mint (separate concerns) |

### Key Differences Explained

**1. CPU-Only Mining**
Bitcoin mining is dominated by ASIC hardware costing thousands of dollars. LOS uses SHA3-256 which has minimal GPU/ASIC advantage — a $200 laptop mines as efficiently per-watt as a data center. This was a deliberate design choice for fair distribution.

**2. No Mining Pools**
In Bitcoin, solo miners almost never find blocks. Pools aggregate hashpower and split rewards. In LOS, each mining proof is cryptographically bound to a specific address and epoch: `SHA3(LOS_MINE_V1 ‖ chain_id ‖ address ‖ epoch ‖ nonce)`. Pools cannot work because proofs are non-transferable.

**3. Separation of Concerns**
Bitcoin uses PoW for both consensus AND distribution. LOS separates these:
- **Consensus** → aBFT (fast finality, no energy waste)
- **Distribution** → PoW mint (fair supply release to the public)

This means LOS achieves 2-3 second finality while still using PoW for fair token distribution.

**4. Aggressive Halving**
Bitcoin halves every ~4 years. LOS halves every ~1 year (8,760 epochs). This means early miners are rewarded more aggressively, but the distribution completes faster — reaching >99% mined within ~15 years.

**5. No Electricity Arms Race**
Bitcoin's energy consumption rivals small countries because miners compete on hashrate. LOS mining has dynamic difficulty that targets ~10 miners per epoch. More miners = higher difficulty, but the CPU-only algorithm caps the energy ceiling naturally.

---

## FAQ

**Q: Can I mine on multiple machines?**
A: Each machine needs its own LOS address. Each address gets at most 1 reward per epoch. Running multiple machines with the same address provides no benefit.

**Q: What happens if I find a proof but my node is offline?**  
A: Proofs must be submitted during the epoch they were mined for. If your node goes offline before broadcasting, the proof is lost.

**Q: How quickly do I receive my mining reward?**  
A: Immediately. Valid mining proofs create Mint blocks that are finalized within ~2-3 seconds via aBFT consensus.

**Q: Can GPU miners dominate?**  
A: SHA3-256 has minimal GPU advantage over CPUs. The algorithm was chosen specifically for fair CPU mining.

**Q: Is there a minimum stake to mine?**  
A: You need to run a full validator node, which requires at minimum 1 LOS to register. The mining node process handles everything.

**Q: What if nobody mines for hours?**  
A: Difficulty drops by 2 bits per empty epoch, making it exponentially easier until someone mines. The difficulty floor is 16 bits (~65K hashes).

**Q: Can I use a mining pool?**  
A: No. Each proof is bound to a specific LOS address and epoch. Pool mining does not work with this design — this is intentional for fair distribution.
