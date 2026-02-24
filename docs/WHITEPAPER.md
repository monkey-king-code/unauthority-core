# Unauthority (LOS) — Technical Whitepaper v2.1

**Lattice Of Sovereignty: Post-Quantum, Privacy-First Block-Lattice Blockchain**

*February 2026*

---

## Table of Contents

1. [Abstract](#1-abstract)
2. [Design Principles](#2-design-principles)
3. [Block-Lattice Architecture](#3-block-lattice-architecture)
4. [Transaction Lifecycle — Anatomy of a Transfer](#4-transaction-lifecycle)
5. [Consensus: Asynchronous Byzantine Fault Tolerance](#5-consensus-abft)
6. [Token Economics](#6-token-economics)
7. [Public Supply Distribution — PoW Mining](#7-public-supply-distribution)
8. [Validator Reward System](#8-validator-reward-system)
9. [Linear Voting — Sybil-Neutral Security](#9-linear-voting)
10. [Slashing — Validator Accountability](#10-slashing)
11. [Finality Checkpoints — Long-Range Attack Prevention](#11-finality-checkpoints)
12. [Post-Quantum Cryptography](#12-post-quantum-cryptography)
13. [Network Architecture — Tor Integration](#13-network-architecture)
14. [Smart Contracts — UVM](#14-smart-contracts)
15. [Security Analysis](#15-security-analysis)
16. [Performance Benchmarks](#16-performance-benchmarks)
17. [Supply Verification — Mathematical Proof](#17-supply-verification)

---

## 1. Abstract

Unauthority (ticker: **LOS** — Lattice Of Sovereignty) is a fully decentralized, permissionless blockchain designed for the post-quantum era. It uses a block-lattice (DAG) structure where each account maintains its own chain, enabling lock-free parallel transaction processing with sub-3-second finality.

Consensus is achieved through an asynchronous Byzantine Fault Tolerant (aBFT) protocol. All cryptographic operations use **CRYSTALS-Dilithium5** (NIST FIPS 204) for signatures and **SHA3-256** (NIST FIPS 202) for hashing — both quantum-resistant standards.

The native token has a fixed supply of **21,936,236 LOS** with **96.45% distributed to the public** through CPU-friendly Proof-of-Work mining. There is no ICO, no pre-sale, no venture capital allocation.

All consensus and financial arithmetic uses `u128` integer math — zero floating-point operations — ensuring deterministic results across all hardware platforms.

Network traffic is routed through **Tor hidden services** (recommended) for validator anonymity, though clearnet operation is also supported.

---

## 2. Design Principles

| Principle | Implementation |
|-----------|---------------|
| **Immutability** | No governance override, no admin keys, no emergency pause function |
| **Permissionless** | 1 LOS to register as validator. No approval needed. |
| **Privacy** | Tor hidden services recommended. No KYC. No IP exposure. |
| **Determinism** | `u128` integer-only consensus math. Zero `f32`/`f64`. |
| **Simplicity** | Single binary (`los-node`). Auto-bootstrap. Minimal config. |
| **Fair Distribution** | 96.45% of supply via open mining. No insider allocation. |

---

## 3. Block-Lattice Architecture

### 3.1 Structure

Unlike traditional blockchains that maintain a single sequential chain of blocks, Unauthority uses a block-lattice where **each account owns its own chain**:

```
Account A: [Genesis] → [Send 50] → [Send 20] → [Receive 10] → ...
                           │
Account B: [Genesis] → [Receive 50] → [Send 10] → ...
                                          │
Account C: [Mint 100] → [Receive 20] → [Receive 10] → ...
```

Each block references its `previous` block hash, forming per-account chains. Cross-account links (send → receive) create the lattice.

**Consequence:** Transactions on different accounts are processed in parallel. Account A sending to Account B does not block Account C sending to Account D. Throughput scales with the number of active accounts.

### 3.2 Block Types

| Type | Purpose | `link` field |
|------|---------|-------------|
| **Send** | Debit from sender | Recipient address |
| **Receive** | Credit to receiver | Hash of the Send block |
| **Mint** | Token creation (PoW or genesis) | Source reference (`MINE:epoch:nonce`) |
| **Slash** | Penalty deduction | Evidence hash |
| **ContractDeploy** | Deploy WASM contract | Contract hash |
| **ContractCall** | Execute contract function | Contract address |
| **Change** | Delegate representative | New representative address |

### 3.3 Block Fields

Every block contains these fields:

```rust
// From: crates/los-core/src/lib.rs

pub struct Block {
    pub account: String,      // Owner address (LOS...)
    pub previous: String,     // Previous block hash in this account's chain
    pub block_type: BlockType, // Send, Receive, Mint, etc.
    pub amount: u128,         // Amount in CIL (atomic units)
    pub link: String,         // Context-dependent reference
    pub signature: String,    // Dilithium5 hex signature
    pub public_key: String,   // Dilithium5 hex public key
    pub work: u64,            // Anti-spam PoW nonce
    pub timestamp: u64,       // Unix timestamp (seconds)
    pub fee: u128,            // Transaction fee in CIL
    pub hash: String,         // Block ID (SHA3-256 of all fields)
}
```

### 3.4 Block Hash Computation

Block identity is computed in two stages:

**Stage 1 — Signing Hash** (message to sign):

```rust
// From: crates/los-core/src/lib.rs — Block::signing_hash()

fn signing_hash(&self) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(CHAIN_ID.to_le_bytes());     // Prevents cross-chain replay
    hasher.update(self.account.as_bytes());
    hasher.update(self.previous.as_bytes());
    hasher.update([type_byte]);                 // 0=Send, 1=Receive, 3=Mint...
    hasher.update(self.amount.to_le_bytes());
    hasher.update(self.link.as_bytes());
    hasher.update(self.public_key.as_bytes());
    hasher.update(self.work.to_le_bytes());
    hasher.update(self.timestamp.to_le_bytes());
    hasher.update(self.fee.to_le_bytes());
    hex::encode(hasher.finalize())
}
```

**Stage 2 — Block Hash** (unique block ID):

```rust
fn calculate_hash(&self) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(self.signing_hash().as_bytes());
    hasher.update(self.signature.as_bytes());   // Includes signature in hash
    hex::encode(hasher.finalize())
}
```

Including the signature in the hash means two blocks with identical content but different signatures produce different hashes — preventing block ID collisions.

---

## 4. Transaction Lifecycle

A complete Send transaction involves 7 verified steps. Here is the exact sequence for sending 10 LOS from Alice to Bob:

### Step 1: Fetch Account State
```
GET /account/LOS_alice_address
→ { "balance_cil": 1500000000000, "head_block": "abc123...", "block_count": 5 }
```

### Step 2: Fetch Fee
```
GET /node-info
→ { "base_fee_cil": 100000 }
```

### Step 3: Construct Block
```
account:    "LOS_alice_address"
previous:   "abc123..."          (head_block from step 1)
block_type: Send
amount:     1000000000000        (10 LOS × 10^11 CIL/LOS)
link:       "LOS_bob_address"    (recipient)
fee:        100000               (0.000001 LOS)
timestamp:  1740000000           (current Unix time)
public_key: "dilithium5_hex..."
```

### Step 4: Compute Anti-Spam PoW
```
Find nonce where SHA3-256(signing_hash || nonce) has ≥16 leading zero bits
```
This takes < 0.1 seconds on any modern CPU. It prevents spam without requiring permission.

### Step 5: Sign with Dilithium5
```
message = signing_hash(block)
signature = dilithium5_sign(secret_key, message)
```

### Step 6: Submit
```
POST /send
{
  "from": "LOS_alice_address",
  "target": "LOS_bob_address",
  "amount": "10",
  "amount_cil": 1000000000000,
  "signature": "dilithium5_sig_hex...",
  "public_key": "dilithium5_pk_hex...",
  "previous": "abc123...",
  "work": 847291,
  "timestamp": 1740000000,
  "fee": 100000
}
```

### Step 7: Consensus & Finality (< 3 seconds)

1. Receiving validator verifies:
   - Signature is valid Dilithium5 over `signing_hash`
   - `previous` matches the actual head block (prevents double-spend)
   - Balance ≥ amount + fee
   - PoW meets 16-bit difficulty
   - Timestamp within ±5 minutes of current time
   - Public key hashes to the claimed address

2. Validator broadcasts `CONFIRM_REQ` to all peers

3. Peers vote with stake-weighted power:
   - Each validator with ≥ `MIN_VALIDATOR_STAKE_CIL` (1,000 LOS) adds `stake` to the vote tally
   - Requires ≥ `2f+1` distinct voting validators AND power threshold of 20,000 units

4. Once quorum reached: block is finalized. Alice's balance decreases by `amount + fee`. Bob has a pending Receive.

5. Auto-receive: Bob's node automatically creates a Receive block referencing Alice's Send block hash.

### Verification Code

Every step is verified in Rust with checked integer arithmetic:

```rust
// From: crates/los-core/src/lib.rs — process_block() for Send

// Verify balance sufficiency (integer math, no floating-point)
let total_debit = amount.checked_add(fee)
    .ok_or("Amount + fee overflow")?;
if account.balance < total_debit {
    return Err("Insufficient balance");
}

// Debit atomically
account.balance -= total_debit;
```

No `f64`. No rounding errors. No precision loss. The balance check uses `u128` (340 undecillion max value — far more than total supply).

---

## 5. Consensus: Asynchronous Byzantine Fault Tolerance

### 5.1 Protocol

Unauthority uses a 3-phase aBFT protocol:

```
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│ Pre-Prepare  │ ───→ │   Prepare    │ ───→ │   Commit     │
│  (Leader)    │      │  (All vote)  │      │  (Finalize)  │
└─────────────┘      └─────────────┘      └─────────────┘
```

1. **Pre-Prepare** — Leader proposes a block
2. **Prepare** — Validators verify the block and broadcast prepare votes
3. **Commit** — Once `2f+1` prepare votes collected, validators broadcast commit votes. After `2f+1` commit votes, the block is finalized.

### 5.2 Quorum

```
f = (n - 1) / 3          Maximum faulty validators
quorum = 2f + 1           Minimum votes to finalize
safety: n ≥ 3f + 1        Safety guarantee
```

| Validators (n) | Max Faulty (f) | Quorum (2f+1) |
|:-:|:-:|:-:|
| 4 | 1 | 3 |
| 7 | 2 | 5 |
| 13 | 4 | 9 |
| 100 | 33 | 67 |

### 5.3 Dual-Threshold Consensus

Send transactions use stake-weighted voting with TWO thresholds:

```rust
// From: crates/los-node/src/main.rs — CONFIRM_RES handler

// Threshold 1: Minimum distinct voters (BFT safety)
let min_voters = min_distinct_voters(total_validators);

// Threshold 2: Stake-weighted power (economic security)
const POWER_THRESHOLD: u128 = 20_000;

let finalized = distinct_voters >= min_voters
    && total_power >= POWER_THRESHOLD;
```

Both conditions must be met. This prevents scenarios where a small number of wealthy validators finalize transactions without broad consensus.

### 5.4 Leader Selection

Deterministic round-robin based on sorted validator set:

```rust
let mut registered: Vec<&String> = pool.validators.keys().collect();
registered.sort();
let leader_index = completed_epoch as usize % registered.len();
let is_leader = registered[leader_index] == &my_address;
```

All nodes compute the same leader deterministically — no randomness, no beacon, no external dependency.

### 5.5 View Change

If the leader fails to produce in 5,000ms:
1. View number increments
2. Next validator in sorted order becomes leader
3. Previous view's votes are cleared
4. Consensus restarts

This ensures liveness even if validators go offline.

### 5.6 Timing Parameters

| Parameter | Value |
|-----------|-------|
| Block proposal timeout | 3,000 ms |
| View change timeout | 5,000 ms (extended for Tor latency) |
| Finality target | < 3 seconds |
| Finalized block memory cap | 10,000 blocks |

---

## 6. Token Economics

### 6.1 Supply

| Parameter | Value |
|-----------|-------|
| **Total Supply** | 21,936,236 LOS |
| **Atomic Unit** | CIL (1 LOS = 10^11 CIL) |
| **Inflation** | Zero. No new tokens created beyond genesis + mining pool. |

### 6.2 Genesis Allocation

| Allocation | LOS | % of Supply | Purpose |
|-----------|----:|:----------:|---------|
| **Public Mining Pool** | **21,158,413** | **96.45%** | Distributed via PoW mining |
| Dev Treasury 1 | 428,113 | 1.95% | Core development |
| Dev Treasury 2 | 245,710 | 1.12% | Operations |
| Dev Treasury 3 | 50,000 | 0.23% | Community grants |
| Dev Treasury 4 | 50,000 | 0.23% | Contingency |
| Bootstrap Validators (4 × 1,000) | 4,000 | 0.02% | Stake only |
| **Total** | **21,936,236** | **100%** | |

### 6.3 Bootstrap Validator Restrictions

The 4 genesis bootstrap validators exist solely to start the network. They are **code-level blocked** from receiving ANY rewards:

```rust
// From: crates/los-core/src/validator_rewards.rs

pub fn is_eligible(&self, current_epoch: u64) -> bool {
    if self.is_genesis {
        return false;  // Genesis validators get ZERO rewards
    }
    // ... uptime, stake, probation checks
}
```

The mining thread also refuses to start for bootstrap addresses:

```rust
// Genesis addresses cannot mine
if bootstrap_validators.contains(&my_address) {
    println!("Bootstrap validator — mining disabled");
    return;
}
```

This guarantees that 100% of all rewards (mining + validator) go to public participants.

### 6.4 Fee Structure

| Parameter | Value |
|-----------|-------|
| Base transaction fee | 100,000 CIL (0.000001 LOS) |
| Fee model | Flat per-transaction |
| Anti-spam rate limiting | x2 multiplier for >10 tx/sec per address |
| Fee destination | Burned (removed from circulation) |

The fee is deliberately low — 0.000001 LOS per transaction — to enable microtransactions while preventing spam.

### 6.5 Unit System

```
1 LOS = 100,000,000,000 CIL = 10^11 CIL

Examples:
  0.000001 LOS = 100,000 CIL       (minimum fee)
  1 LOS        = 100,000,000,000 CIL
  1,000 LOS    = 100,000,000,000,000 CIL  (min reward stake)
```

CIL provides 11 decimal places of precision — higher than Bitcoin's 8 (satoshi) — enabling fine-grained smart contract and DeFi calculations.

---

## 7. Public Supply Distribution — PoW Mining

### 7.1 Design Philosophy

The mining system distributes **96.45% of the total supply** (21,158,413 LOS) to the public through computational work. Key design goals:

1. **No external dependency** — no oracle, no BTC/ETH bridge
2. **Front-run resistant** — proof is cryptographically bound to the miner's address
3. **CPU-friendly** — SHA3-256 has no meaningful GPU/ASIC advantage
4. **Sybil-neutral** — 1 reward per address per epoch, no benefit to splitting
5. **Deterministic emission** — halving schedule is hardcoded, not adjustable

### 7.2 Mining Algorithm

Miners compute:

```
hash = SHA3-256("LOS_MINE_V1" || chain_id || address || epoch || nonce)
```

The proof is valid when `hash` has >= `difficulty_bits` leading zero bits.

```rust
// From: crates/los-core/src/pow_mint.rs

pub fn compute_mining_hash(address: &str, epoch: u64, nonce: u64) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(b"LOS_MINE_V1");           // Domain separator
    hasher.update(CHAIN_ID.to_le_bytes());    // Chain binding
    hasher.update(address.as_bytes());        // Address binding
    hasher.update(epoch.to_le_bytes());       // Time binding
    hasher.update(nonce.to_le_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}
```

**Security properties:**

- `"LOS_MINE_V1"` prefix prevents hash collision with block hashes
- `chain_id` prevents testnet proofs from being replayed on mainnet
- `address` binding makes proofs non-transferable — a proof found by Alice cannot be submitted by Bob
- `epoch` binding limits proof validity to a 1-hour window

### 7.3 Epoch System

| Parameter | Mainnet | Testnet |
|-----------|---------|---------|
| Epoch duration | 3,600 sec (1 hour) | 120 sec (2 min) |
| Reward per epoch | 100 LOS | 100 LOS |
| Halving interval | 8,760 epochs (~1 year) | 10 epochs (~20 min) |
| Initial difficulty | 20 bits | 16 bits |
| Deduplication | 1 reward per (address, epoch) | Same |

### 7.4 Reward Schedule and Halving

```rust
// From: crates/los-core/src/pow_mint.rs

pub fn epoch_reward_cil(epoch: u64) -> u128 {
    let halving_interval = 8_760;
    let halvings = epoch / halving_interval;
    if halvings >= 64 { return 0; }
    MINING_REWARD_PER_EPOCH_CIL >> halvings
}
```

| Year | Epoch Range | Reward/Epoch | Annual Output | Cumulative |
|:----:|:-----------:|:------------:|:-------------:|:----------:|
| 1 | 0 – 8,759 | 100 LOS | 876,000 LOS | 876,000 |
| 2 | 8,760 – 17,519 | 50 LOS | 438,000 LOS | 1,314,000 |
| 3 | 17,520 – 26,279 | 25 LOS | 219,000 LOS | 1,533,000 |
| 4 | 26,280 – 35,039 | 12.5 LOS | 109,500 LOS | 1,642,500 |
| 5 | 35,040 – 43,799 | 6.25 LOS | 54,750 LOS | 1,697,250 |
| 10 | 78,840+ | ~0.195 LOS | ~1,710 LOS | ~1,748,000 |

Mining continues until the 21,158,413 LOS public pool is exhausted. Due to halving, full exhaustion takes many decades.

### 7.5 Difficulty Adjustment

Difficulty targets **10 successful miners per epoch**:

```rust
// From: crates/los-core/src/pow_mint.rs — advance_epoch()

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
        .saturating_sub(1).max(MIN_MINING_DIFFICULTY_BITS);
} else if miners == 0 {
    self.difficulty_bits = self.difficulty_bits
        .saturating_sub(2).max(MIN_MINING_DIFFICULTY_BITS);
}
```

| Difficulty (bits) | Average Hashes | Approx. Time (3 GHz CPU) |
|------------------:|---------------:|-------------------------:|
| 16 | ~65,536 | < 0.1 second |
| 20 | ~1,048,576 | ~0.5 second |
| 24 | ~16,777,216 | ~8 seconds |
| 28 | ~268,435,456 | ~2 minutes |
| 32 | ~4,294,967,296 | ~30 minutes |

Bounds: minimum 16 bits, maximum 40 bits. Maximum adjustment: +/-4 bits per epoch.

### 7.6 Anti-Double-Mining

Each address can only mine once per epoch. The dedup set is rebuilt from the ledger on node restart:

```rust
// On restart: scan Mint blocks to rebuild current epoch's miner set
for block in ledger.blocks.values() {
    if block.block_type == BlockType::Mint
        && block.link.starts_with(&epoch_prefix)
    {
        mining_state.current_epoch_miners.insert(block.account.clone());
    }
}
```

This prevents a restarted node from allowing double-mining within the same epoch.

### 7.7 Gossip Broadcast

Valid mining proofs are broadcast as:
```
MINE_BLOCK:{"account":"LOS...","block_type":"Mint","amount":10000000000000,...}
```

All receiving nodes independently verify the PoW before accepting the Mint block. Invalid proofs are silently dropped.

---

## 8. Validator Reward System

### 8.1 Pool

| Parameter | Value |
|-----------|-------|
| Total pool | 500,000 LOS (from genesis, non-inflationary) |
| Initial rate | 5,000 LOS per epoch |
| Epoch duration | 30 days (mainnet) |
| Halving interval | 48 epochs (~4 years) |
| Distribution | Linear stake-weighted, proportional |

### 8.2 Halving Schedule

```rust
// From: crates/los-core/src/validator_rewards.rs

pub fn epoch_reward_rate(&self) -> u128 {
    let halvings = self.current_epoch / REWARD_HALVING_INTERVAL_EPOCHS;
    if halvings >= 128 { return 0; }
    let rate = REWARD_RATE_INITIAL_CIL >> halvings;
    rate.min(self.remaining_pool_cil)
}
```

| Period | Epochs | Rate/Epoch | Total Distributed |
|--------|--------|-----------|------------------|
| Years 0–4 | 0 – 47 | 5,000 LOS | 240,000 LOS |
| Years 4–8 | 48 – 95 | 2,500 LOS | 120,000 LOS |
| Years 8–12 | 96 – 143 | 1,250 LOS | 60,000 LOS |
| Years 12–16 | 144 – 191 | 625 LOS | 30,000 LOS |
| Years 16–20 | 192 – 239 | 312.5 LOS | 15,000 LOS |

Asymptotic total: ~480,000 LOS distributed (pool never fully exhausted due to halving).

### 8.3 Distribution Formula

```
budget = min(epoch_reward_rate, remaining_pool)
reward_i = budget * stake_i / sum(stake_all_eligible)
```

```rust
// From: crates/los-core/src/validator_rewards.rs

pub fn linear_stake_weight(stake_cil: u128) -> u128 {
    if stake_cil >= MIN_VALIDATOR_STAKE_CIL {
        stake_cil
    } else {
        0
    }
}
```

Pure linear weighting. 10,000 LOS staked = 10x the reward of 1,000 LOS staked. No square root, no quadratic — Sybil-neutral by design.

### 8.4 Eligibility

| Requirement | Threshold |
|-------------|-----------|
| Minimum stake | 1,000 LOS |
| Minimum uptime | 95% of epoch heartbeats |
| Probation | 1 epoch (30 days) |
| Genesis bootstrap | Permanently excluded |

### 8.5 Uptime Tracking

```rust
// Integer math — no floating-point percentages
pub fn display_uptime_pct(&self) -> u64 {
    let expected = self.expected_heartbeats.max(1);
    let hb = self.heartbeats_current_epoch
        .max(self.heartbeats_last_epoch);
    ((hb as u64) * 100 / expected as u64).min(100)
}
```

---

## 9. Linear Voting — Sybil-Neutral Security

### 9.1 Voting Power

```
voting_power(stake) = stake    (if stake >= MIN_VALIDATOR_STAKE_CIL)
                    = 0        (otherwise)
```

This is the only Sybil-neutral design. Consider:

| Strategy | Stake | Voting Power |
|----------|------:|:-------------|
| 1 validator x 10,000 LOS | 10,000 | 10,000 |
| 10 validators x 1,000 LOS each | 10,000 | 10 x 1,000 = 10,000 |
| 100 validators x 100 LOS each | 10,000 | 0 (below minimum) |

Splitting stake across identities yields the same or less power. There is no advantage to Sybil splitting.

**Why not square root (quadratic)?** Square root voting gives sqrt(10000) = 100 to a single validator but 10 x sqrt(1000) = 316 to ten split validators — a 3.16x Sybil advantage.

### 9.2 Transaction Fee Model

Flat `BASE_FEE_CIL` (100,000 CIL = 0.000001 LOS) per transaction. No dynamic fee scaling. No fee auctions. No MEV extraction through fee manipulation.

Anti-spam rate limiting applies a x2 multiplier for addresses exceeding 10 transactions per second. This is a security mechanism, not fee scaling.

---

## 10. Slashing — Validator Accountability

### 10.1 Violation Types

| Violation | Penalty | Consequence |
|-----------|---------|-------------|
| Double Signing | 100% of stake | Permanent ban |
| Fraudulent Transaction | 100% of stake | Permanent ban |
| Extended Downtime | 1% of stake | Status -> Slashed |

### 10.2 Detection

**Double Signing:** The slashing manager maintains a rolling window of recent block signatures per validator. Two different blocks at the same height trigger immediate detection:

```rust
// From: crates/los-consensus/src/slashing.rs

pub fn check_double_sign(
    &mut self, address: &str, block_hash: &str,
    block_height: u64, timestamp: u64
) -> Option<SlashingViolation> {
    // Same height, different hash = double signing
    if existing_hash != block_hash {
        return Some(SlashingViolation {
            violation_type: ViolationType::DoubleSigning,
            slash_amount_bps: 10_000,  // 100%
            // ...
        });
    }
    None
}
```

**Downtime:** Participation tracked over 50,000 block observation window:

```
uptime_bps = (blocks_participated * 10,000) / total_blocks_observed
if window >= 50,000 blocks AND uptime_bps < 9,500:
    slash 1% of stake
```

### 10.3 Slash Consensus

Slashing requires multi-validator agreement:

```
slash_threshold = (total_validators * 2 / 3) + 1
```

Evidence hash prevents duplicate proposals. Stake amount is read from the ledger at confirmation time, preventing front-running (withdrawing stake before slash executes).

### 10.4 Slashed State Persistence

Total slashed CIL is persisted to disk across node restarts:

```rust
// From: crates/los-node/src/db.rs

pub fn save_total_slashed(&self, total_slashed_cil: u128) -> Result<()> {
    self.db.insert("total_slashed_cil", &total_slashed_cil.to_le_bytes())?;
    self.db.flush()?;
    Ok(())
}
```

---

## 11. Finality Checkpoints — Long-Range Attack Prevention

### 11.1 Design

Every `CHECKPOINT_INTERVAL` blocks (~30 minutes), the network creates a cryptographic checkpoint:

```rust
// From: crates/los-consensus/src/checkpoint.rs

pub struct Checkpoint {
    pub height: u64,                  // Block height
    pub state_hash: String,           // SHA3-256 of ledger state
    pub timestamp: u64,
    pub signatures: Vec<CheckpointSignature>,
}
```

### 11.2 Checkpoint Consensus

1. Leader proposes checkpoint with state hash
2. Validators verify: their local state hash matches the proposal
3. Each validator signs and broadcasts confirmation
4. Checkpoint is finalized when `2f+1` signatures collected

### 11.3 Purpose

Checkpoints prevent long-range attacks where an adversary constructs an alternative chain history from a point in the past. Nodes refuse to reorganize past a finalized checkpoint.

---

## 12. Post-Quantum Cryptography

### 12.1 Dilithium5 (NIST FIPS 204)

| Property | Value |
|----------|-------|
| Security (classical) | 256-bit |
| Security (quantum) | 128-bit (Grover/Shor resistant) |
| Public key size | 2,592 bytes |
| Signature size | 4,627 bytes |
| Key generation | ~1 ms |
| Sign | ~1 ms |
| Verify | ~0.5 ms |
| Standard | NIST FIPS 204 (finalized 2024) |

### 12.2 SHA3-256 (NIST FIPS 202)

Used for all hashing: block hashes, address derivation, mining, checkpoints.

### 12.3 Key Derivation

Keys are derived deterministically from BIP39 mnemonic seeds:

```
seed = BIP39_to_bytes(24_word_mnemonic)
expanded_seed = SHA3-256(seed)
(public_key, secret_key) = dilithium5_keygen(expanded_seed)
address = "LOS" + Base58(SHA3-256(public_key))
```

### 12.4 Key Storage

Private keys are encrypted at rest using the `age` encryption standard with scrypt key derivation (N=2^20):

```rust
// From: crates/los-crypto/src/lib.rs

pub fn encrypt_private_key(secret_key: &[u8], password: &str) -> Result<Vec<u8>> {
    let encryptor = age::scrypt::Recipient::new(
        password.into()
    );
    // age encrypted binary (portable, audited standard)
    // ...
}
```

### 12.5 Why Post-Quantum from Day One?

ECDSA and Ed25519 — used by Bitcoin, Ethereum, and most blockchains — are vulnerable to Shor's algorithm on quantum computers. Migration after deployment is disruptive and incomplete (old signed transactions remain vulnerable).

By using lattice-based cryptography from genesis, Unauthority:
- Protects all addresses and signatures from future quantum attacks
- Avoids a disruptive migration
- Ensures long-term value preservation

---

## 13. Network Architecture — Tor Integration

### 13.1 Transport

Network traffic routes through Tor hidden services (recommended):

```
Node A (.onion) <-> Tor Network <-> Node B (.onion)
```

Clearnet (IP/domain) is also supported but Tor is recommended for validator anonymity.

### 13.2 Transport Modes

| Configuration | Behavior |
|--------------|----------|
| `LOS_TOR_ENABLED=true` | Auto-generate `.onion` via Tor control port |
| `LOS_ONION_ADDRESS=x.onion` | Use manual `.onion` address |
| `LOS_HOST_ADDRESS=ip:port` | Clearnet mode |
| (none) | Auto-detect Tor; fall back to clearnet |

### 13.3 Security Properties

- **Tor enabled:** mDNS disabled (prevents LAN presence leak), binds `127.0.0.1`
- **Tor disabled:** mDNS enabled (local development), binds `0.0.0.0`
- **SOCKS5 proxy** used for all `.onion` peer connections
- **Noise Protocol** (XX handshake) for end-to-end encryption on top of Tor transport

### 13.4 Peer Discovery

1. Node reads bootstrap validator addresses from `genesis_config.json` (embedded at compile time)
2. Connects to bootstrap peers via Tor SOCKS5 (500ms timeout for proxy detection)
3. Fetches dynamic peer table from connected peers (`GET /peers`)
4. Maintains peer health table sorted by latency and uptime
5. Periodic re-discovery every 10 minutes

### 13.5 Why Tor?

- **No IP exposure:** Validator operators cannot be identified or targeted
- **NAT traversal:** `.onion` addresses work behind any firewall/router
- **Censorship resistance:** Tor traffic is difficult to block or inspect
- **No KYC dependency:** Running a validator requires no personal information

---

## 14. Smart Contracts — UVM

### 14.1 Architecture

The Unauthority Virtual Machine (UVM) executes WASM smart contracts:

```
Rust source -> cargo build --target wasm32-unknown-unknown -> .wasm
    -> Deploy via POST /deploy-contract
    -> Execute via POST /call-contract
    -> State stored in contract-specific key-value store
```

### 14.2 Token Standard: USP-01

Native fungible token standard for:
- Custom token creation
- Wrapped assets (wBTC, wETH, wUSDT)
- Standard interface: `transfer`, `approve`, `balance_of`, `total_supply`
- Token registry with metadata (name, symbol, decimals)

### 14.3 DEX Architecture

Decentralized exchange runs as Layer 2 smart contracts:
- AMM (Automated Market Maker) with xy=k pricing
- MEV resistant — block-lattice finality ordering, not miner-extractable
- Permissionless — anyone can deploy a DEX contract
- Multiple DEXs coexist independently

### 14.4 Gas Pricing

```
GAS_PRICE_CIL = 1          (1 gas = 1 CIL)
MIN_DEPLOY_FEE = 0.01 LOS  (1,000,000,000 CIL)
MIN_CALL_FEE = BASE_FEE    (100,000 CIL)
DEFAULT_GAS_LIMIT = 1,000,000 gas units
```

---

## 15. Security Analysis

### 15.1 Threat Model

| Threat | Mitigation |
|--------|-----------|
| Quantum computers | Dilithium5 (128-bit quantum security) |
| DDoS | Tor hidden services — no IP to target (recommended mode) |
| 51% attack | aBFT requires `2f+1` honest; linear voting is Sybil-neutral |
| Double spending | Per-account chain with `previous` hash; aBFT finality < 3s |
| Sybil attack | Linear voting (no benefit to splitting stake) |
| Floating-point non-determinism | Zero `f32`/`f64` in consensus; all `u128` integer |
| Inflation bug | Fixed supply with checked arithmetic (`checked_add`, `checked_mul`) |
| Replay attack | `chain_id` in signing hash; `timestamp` drift limit +/-5 min |
| Long-range attack | Finality checkpoints with multi-validator signatures |
| Fee manipulation | Flat fee (no auction); fee included in signing hash |
| Front-running | Mining proofs bound to address; block ordering by finality |
| Network partition | aBFT liveness requires `2f+1`; view change on timeout |

### 15.2 Determinism Guarantee

All consensus-critical computation uses:

- `u128` integer arithmetic (no floating-point)
- `checked_mul`, `checked_add` (overflow detection)
- `saturating_sub` (underflow prevention)
- `isqrt` via Newton's method (deterministic across platforms, used for DEX AMM math)
- Sorted arrays for deterministic ordering
- Basis points (10,000 = 100%) instead of percentages

This ensures all validators compute identical results regardless of CPU architecture, operating system, or compiler version.

### 15.3 Supply Integrity

The total supply is verified at multiple levels:

```rust
// Compile-time constant
pub const TOTAL_SUPPLY_CIL: u128 = 21_936_236 * CIL_PER_LOS;

// Mint block validation cap
const MAX_MINT_PER_BLOCK: u128 = 1_000 * CIL_PER_LOS;

// Supply exhaustion check in mining
let final_reward = miner_reward.min(remaining_supply_cil);
if final_reward == 0 {
    return Err("Public supply exhausted");
}
```

---

## 16. Performance Benchmarks

### 16.1 Target Metrics

| Metric | Target | Measured |
|--------|--------|---------|
| Transaction finality | < 3 seconds | ~2s over Tor |
| API response (Tor) | < 2 seconds | 500ms – 2s |
| API response (local) | < 5ms | < 5ms |
| Gossip round-trip (Tor) | < 3 seconds | 1s – 3s |
| Mining proof (20 bits) | < 2 seconds | ~0.5s |
| Dilithium5 sign | < 2 ms | ~1 ms |
| Dilithium5 verify | < 1 ms | ~0.5 ms |
| Concurrent accounts | Unlimited | Per-account chains |

### 16.2 Bottleneck

Tor adds ~500ms–2s per network hop. This is an explicit trade-off for privacy.

The block-lattice structure mitigates this — since each account has its own chain, transactions on different accounts process in parallel. A sends to B while C sends to D, without waiting for each other.

### 16.3 Why This Architecture is Fast

1. **No global block queue:** Transactions don't wait in a mempool for N minutes
2. **Per-account chains:** Parallel processing without locks
3. **Instant sender finality:** The sender's block is created immediately; only confirmation votes take ~2s
4. **Auto-receive:** Receiving nodes automatically create Receive blocks
5. **Lightweight verification:** Each validator only processes the specific accounts involved

---

## 17. Supply Verification — Mathematical Proof

### 17.1 Total Supply Invariant

At any point in time:

```
sum(all_account_balances) + sum(pending_sends) + remaining_mining_pool
    + remaining_validator_pool + total_fees_burned + total_slashed
    = TOTAL_SUPPLY_CIL
```

This invariant is enforced by:
- No mechanism to create CIL outside of genesis allocation + mining mint
- `checked_add` on all credit operations (overflow -> error, not silent wraparound)
- Balance checks before every debit (`balance >= amount + fee`)
- Mint cap per block (1,000 LOS maximum)

### 17.2 Mining Supply Exhaustion Guarantee

```
Year 1:  100 x 8,760 = 876,000 LOS
Year 2:   50 x 8,760 = 438,000 LOS
Year 3:   25 x 8,760 = 219,000 LOS
...
Total = 100 x 8,760 x sum(1/2^k, k=0..inf) = 876,000 x 2 = 1,752,000 LOS (geometric series)
```

This proves that even with continuous maximum mining, total mined supply converges to ~1,752,000 LOS — well below the 21,158,413 LOS mining pool. The pool is designed to last for many decades.

### 17.3 Validator Reward Pool Exhaustion

```
Epoch 0-47:   5,000 x 48 = 240,000 LOS
Epoch 48-95:  2,500 x 48 = 120,000 LOS
...
Total = 5,000 x 48 x sum(1/2^k, k=0..inf) = 240,000 x 2 = 480,000 LOS
```

Asymptotically approaches 480,000 LOS — within the 500,000 LOS pool. The remaining ~20,000 LOS stays in the pool indefinitely (halving never fully reaches zero due to integer truncation).

---

*Unauthority (LOS) — Lattice Of Sovereignty*

*100% Immutable. 100% Permissionless. 100% Decentralized.*
