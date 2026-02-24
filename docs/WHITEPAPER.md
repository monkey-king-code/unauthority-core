# Technical Whitepaper — Unauthority (LOS) v2.0.1

**Lattice Of Sovereignty: A Post-Quantum, Privacy-First Block-Lattice Blockchain**

*Last updated: February 2026*

---

## Table of Contents

1. [Abstract](#abstract)
2. [Design Principles](#design-principles)
3. [Block-Lattice Architecture](#block-lattice-architecture)
4. [Consensus: aBFT](#consensus-abft)
5. [Token Economics](#token-economics)
6. [PoW Mining Distribution](#pow-mining-distribution)
7. [Validator Rewards](#validator-rewards)
8. [Linear Voting & Security](#linear-voting--security)
9. [Slashing & Accountability](#slashing--accountability)
10. [Post-Quantum Cryptography](#post-quantum-cryptography)
11. [Network Layer](#network-layer)
12. [Smart Contracts (UVM)](#smart-contracts-uvm)
13. [Security Analysis](#security-analysis)
14. [Performance](#performance)

---

## Abstract

Unauthority (ticker: **LOS** — Lattice Of Sovereignty) is a fully decentralized, permissionless blockchain that operates exclusively over Tor hidden services. It uses a block-lattice (DAG) structure where each account maintains its own chain, enabling lock-free parallel transaction processing. Consensus is achieved via an asynchronous Byzantine Fault Tolerant (aBFT) protocol using post-quantum Dilithium5 signatures. The native token has a fixed supply of 21,936,236 LOS with no inflation.

Key differentiators:
- **Tor-recommended** — validators can run on .onion (recommended) or clearnet
- **Post-quantum** — Dilithium5 (NIST FIPS 204) with 256-bit classical security
- **Deterministic** — all consensus math uses u128 integer arithmetic, zero floating-point
- **Sybil-neutral** — linear voting (1 LOS = 1 vote), no concentration advantage
- **Interoperable** — USP-01 token standard for wrapped assets (wBTC, wETH) via WASM DEX

---

## Design Principles

1. **Immutability** — No governance override, no admin keys, no emergency pauses
2. **Permissionless** — Anyone can register as a validator with just 1 LOS (1,000 LOS for reward eligibility)
3. **Privacy** — Tor recommended for all traffic; no KYC required
4. **Determinism** — Integer-only math in all consensus-critical paths
5. **Simplicity** — Single binary (`los-node`), auto-bootstrap, minimal configuration

---

## Block-Lattice Architecture

### Structure

Unlike traditional blockchains (single global chain), Unauthority uses a block-lattice where each account has its own chain of blocks:

```
Account A: [Genesis] → [Send 50 to B] → [Send 20 to C] → ...
                            │
Account B: [Genesis] → [Receive 50]   → [Send 10 to C] → ...
                                              │
Account C: [Mint]   → [Receive 20]    → [Receive 10]   → ...
```

Each block references its `previous` block hash, forming per-account chains. Cross-account references (send→receive links) create the lattice structure.

### Block Types

| Type | Description | `link` field |
|---|---|---|
| **Send** | Debit from sender's balance | Recipient address |
| **Receive** | Credit to receiver's balance | Hash of the Send block |
| **Mint** | Token creation (genesis or PoW reward) | Source reference (MINE:epoch:nonce) |
| **Change** | Representative/validator delegation | New representative |

### Block Fields

| Field | Type | Description |
|---|---|---|
| `account` | String | Owner address (LOS...) |
| `previous` | String | Hash of previous block in this chain |
| `block_type` | Enum | Send, Receive, Mint, Change |
| `amount` | u128 | Amount in CIL (atomic units) |
| `link` | String | Context-dependent reference |
| `signature` | String | Dilithium5 hex signature |
| `public_key` | String | Dilithium5 hex public key |
| `work` | u64 | Proof-of-Work nonce (anti-spam) |
| `timestamp` | u64 | Unix timestamp |
| `fee` | u128 | Transaction fee in CIL (flat `BASE_FEE_CIL`) |

### Benefits

- **Lock-free parallelism** — transactions on different accounts process concurrently
- **Instant sender confirmation** — no global block wait
- **Scalable** — throughput scales with account count, not block size
- **Lightweight** — each account's chain is small; no need to process the full DAG

---

## Consensus: aBFT

Unauthority uses a 3-phase aBFT protocol (based on PBFT structure):

### Phases

1. **Pre-Prepare** — Leader proposes a block for consensus
2. **Prepare** — Validators verify and broadcast prepare votes
3. **Commit** — Once quorum reached, validators commit the block

### Byzantine Fault Tolerance

```
f = (n - 1) / 3        # Maximum faulty validators tolerated
quorum = 2f + 1         # Minimum votes to finalize
safety: 3f < n          # Safety guarantee (cannot finalize conflicting blocks)
```

| Validators (n) | Max Faulty (f) | Quorum (2f+1) |
|---|---|---|
| 4 | 1 | 3 |
| 7 | 2 | 5 |
| 13 | 4 | 9 |
| 100 | 33 | 67 |

### Leader Selection

Round-robin based on the current view number:

```
leader_index = view % total_validators
```

Validators are sorted deterministically by address, ensuring all nodes agree on the leader.

### View Change

If the leader fails (timeout after 5,000ms), a **view change** is triggered:
- View number increments
- Prepare/commit votes for the old view are cleared
- New leader is selected via round-robin
- Consensus restarts for the pending block

### Timing

| Parameter | Value |
|---|---|
| Block timeout | 3,000 ms |
| View change timeout | 5,000 ms |
| Finality target | < 3 seconds |
| Finalized block memory cap | 10,000 blocks |

---

## Token Economics

### Supply

| Parameter | Value |
|---|---|
| **Total Supply** | 21,936,236 LOS (Fixed, non-inflationary) |
| **Atomic Unit** | CIL (1 LOS = 10^11 CIL) |
| **Dev Treasury** | 773,823 LOS (~3.5%) |
| **Bootstrap Validators** | 4,000 LOS (4 × 1,000) |
| **Public Allocation** | 21,158,413 LOS (~96.5%) |
| **Reward Pool** | 500,000 LOS (from Dev Treasury, non-inflationary) |

### Dev Treasury Breakdown

| Allocation | Amount (LOS) | Purpose |
|---|---|---|
| Dev Treasury 1 | 428,113 | Core development |
| Dev Treasury 2 | 245,710 | Development operations |
| Dev Treasury 3 | 50,000 | Community grants |
| Dev Treasury 4 | 50,000 | Emergency fund |
| **Dev Subtotal** | **773,823** | |
| **Total Non-Public** | **777,823** | |

### Public Supply Distribution

The public allocation (21,158,413 LOS) is distributed exclusively through PoW mining:

```
Public Supply Cap = 21,158,413 × 10^11 CIL = 2,115,841,300,000,000,000 CIL
```

No other mechanism exists to create new tokens. Once the public supply is fully distributed, no more LOS can be minted.

---

## PoW Mining Distribution

### Mechanism

Miners run full validator nodes with `--mine` flag and compute SHA3-256 proofs:

1. Miner runs `los-node --mine` (full validator required)
2. Background thread grinds SHA3-256: `SHA3(LOS_MINE_V1 || chain_id || address || epoch || nonce)`
3. Hash must have N leading zero bits (difficulty auto-adjusts)
4. Successful proof creates a Mint block with `MINE:epoch:nonce` link format
5. Mint block broadcast via `MINE_BLOCK:{json}` gossip message
6. All nodes verify PoW proof before accepting

### Mining Parameters

| Parameter | Mainnet | Testnet |
|---|---|---|
| Epoch Duration | 3,600 sec (1 hour) | 120 sec (2 min) |
| Reward per Epoch | 100 LOS | 100 LOS |
| Halving Interval | 8,760 epochs (~1 year) | 8,760 epochs |
| Initial Difficulty | 20 leading zero bits | 20 bits |
| Deduplication | 1 reward per (address, epoch) | Same |

### Difficulty Adjustment

Difficulty auto-adjusts based on active miner count to maintain fair distribution.
More miners = higher difficulty = harder to find valid nonce.

### Fair Distribution

- No ICO, no pre-sale, no VC allocation
- ~96.5% of supply distributed via open PoW mining
- Anyone with a full node can mine
- No external mining API — must run a validator node

---

## Validator Rewards

### Pool

| Parameter | Value |
|---|---|
| Total Pool | 500,000 LOS (non-inflationary) |
| Initial Rate | 5,000 LOS per epoch |
| Epoch Duration | 30 days (mainnet), 2 minutes (testnet) |
| Halving Interval | Every 48 epochs (~4 years) |
| Pool Lifespan | ~16-20 years (asymptotic) |

### Halving Schedule

```
epoch_reward_rate = 5,000 LOS >> (current_epoch / 48)
```

| Epoch Range | Rate (LOS/epoch) | Period |
|---|---|---|
| 0 – 47 | 5,000 | Years 0–4 |
| 48 – 95 | 2,500 | Years 4–8 |
| 96 – 143 | 1,250 | Years 8–12 |
| 144 – 191 | 625 | Years 12–16 |
| 192 – 239 | 312 | Years 16–20 |
| ... | ... | Asymptotic → 0 |

After 128 halvings (bit shift to zero), no further rewards are distributed. Total distributed asymptotically approaches ~480,000 LOS.

### Distribution Formula

Per-validator reward within an epoch:

```
budget = min(epoch_reward_rate, remaining_pool)
reward_i = budget × stake_i / Σ stake_all
```

This is pure linear stake-weighted distribution (Sybil-neutral).

### Eligibility

Validator registration requires only **1 LOS** (permissionless). Reward distribution requires all conditions below:

| Requirement | Threshold |
|---|---|
| Minimum stake for rewards | 1,000 LOS |
| Minimum uptime | 95% of epoch heartbeats |
| Probation | 1 epoch after registration |
| Genesis bootstrap | Not eligible (mainnet only) |

Uptime is calculated as integer basis points:
```
uptime_pct = min((heartbeats × 100) / expected_heartbeats, 100)
eligible = uptime_pct >= 95
```

---

## Linear Voting & Security

### Linear Voting Power

Voting weight is directly proportional to stake (Sybil-neutral):

```
voting_power = staked_amount_cil  (if >= MIN_STAKE_CIL, else 0)
```

| Scenario | Stake | Voting Power |
|---|---|---|
| 1 large holder | 10,000 LOS | 10,000 |
| 10 small validators | 1,000 LOS each | 10 × 1,000 = 10,000 |

**Result:** Splitting stake across multiple identities yields the same total power. This is **Sybil-neutral** — no advantage to splitting or concentrating.

### Flat Fee Model

All transactions pay the same flat `BASE_FEE_CIL` (0.000001 LOS). No dynamic fee scaling.

### Governance Quorum

Proposal consensus uses stake-weighted votes:

```
votes_for_bps = (Σ voting_power_for × 10,000) / total_voting_power
consensus = votes_for_bps > 5,000  (strictly > 50%)
```

Concentration ratio tracked per validator: `validator_power × 10,000 / total_power`.

---

## Slashing & Accountability

### Violation Types

| Violation | Penalty | Result |
|---|---|---|
| **Double Signing** | 100% of stake | Permanent ban |
| **Fraudulent Transaction** | 100% of stake | Permanent ban |
| **Extended Downtime** | 1% of stake | Status → Slashed |

### Double Signing Detection

The node maintains the last 1,000 block signatures per validator. If two different blocks are signed at the same height by the same validator, it is flagged as double signing.

### Downtime Detection

```
observation_window = 50,000 blocks (~5 hours)
downtime_threshold = 10,000 blocks (~1 hour)

uptime_bps = (blocks_participated × 10,000) / total_blocks_observed

if total_blocks_observed >= 50,000 AND uptime_bps < 9,500:
    slash 1% of stake
```

### Multi-Validator Slash Proposal

Slashing requires consensus:

```
threshold = (total_validators × 2 / 3) + 1
```

- Proposer auto-confirms
- Evidence hash prevents duplicate proposals
- Stake amount read from ledger at confirmation time (prevents front-running)

### Validator State Machine

```
Active → Slashed (via downtime violation)
Active → Banned (via double-signing or fraud)
Active → Unstaking (voluntary exit)
```

---

## Post-Quantum Cryptography

### Algorithm: Dilithium5

Unauthority uses **CRYSTALS-Dilithium** at security level 5 (NIST FIPS 204):

| Property | Value |
|---|---|
| Security (classical) | 256-bit |
| Security (quantum) | 128-bit (against Grover/Shor) |
| Public key size | ~2,592 bytes |
| Signature size | ~4,627 bytes |
| Sign time | ~1ms |
| Verify time | ~0.5ms |

### Hash Function: SHA3-256 (NIST FIPS 202)

All block hashing uses SHA3-256:

```
block_hash = SHA3-256(account || previous || block_type || amount || ...)
address = Base58(SHA3-256(public_key))
```

### Key Derivation

Keys are derived from BIP39 mnemonic seeds using deterministic Dilithium5 key generation. The seed is expanded using SHA-3 before feeding into the key generation algorithm.

### Why Post-Quantum?

Quantum computers threaten ECDSA/Ed25519 via Shor's algorithm. By using lattice-based cryptography from day one, Unauthority:
- Protects against future quantum attacks
- Avoids a disruptive migration later
- Ensures long-term security of all addresses and signatures

---

## Network Layer

### Architecture

All network traffic is recommended to be routed through Tor hidden services:

```
Node A (.onion) ←→ Tor Network ←→ Node B (.onion)
```

- **Tor recommended** — validators are strongly recommended to use .onion addresses for anonymity
- **Clearnet supported** — validators can also run on IP addresses or domains
- **NAT traversal** — Tor-based nodes work behind any firewall/router

### Peer Discovery (v1.0.9+)

1. Node starts and reads genesis config (embedded at compile-time)
2. Extracts addresses of bootstrap validators (`.onion`, IP, or domain)
3. If Tor available, auto-detects SOCKS5 proxy at `127.0.0.1:9050` (500ms timeout)
4. Connects to bootstrap peers (via SOCKS5 for `.onion`, direct for clearnet)
5. Downloads dynamic peer table from connected peers
6. Maintains peer table sorted by latency/uptime

### P2P Communication

All gossip messages are HTTP POST requests routed through Tor:

| Message | Purpose |
|---|---|
| `ID` | Node identity announcement |
| `BLOCK` | New block broadcast |
| `CONFIRM_REQ` | Request confirmation votes |

### Encryption

P2P channels use **Noise Protocol** (XX handshake pattern) for end-to-end encryption, layered on top of Tor's transport encryption.

---

## Smart Contracts (UVM)

### Architecture

The Unauthority Virtual Machine (UVM) executes WASM smart contracts:

```
Rust source → cargo build --target wasm32-unknown-unknown → .wasm
    → Deploy via /deploy-contract
    → Execute via /call-contract
    → State stored in contract-specific key-value store
```

### Token Standard: USP-01

Native fungible token standard enables:
- Custom token creation
- Wrapped assets (wBTC, wETH)
- Standard `transfer`, `approve`, `balance_of` interface

### Decentralized Exchange (DEX)

The DEX runs as Layer 2 smart contracts:
- AMM (Automated Market Maker) model
- MEV resistant (ordered by block-lattice finality, not miner choice)
- Permissionless — anyone can deploy a DEX contract
- Multiple DEXs can coexist independently

---

## Security Analysis

### Threat Model

| Threat | Mitigation |
|---|---|
| Quantum computers | Dilithium5 (lattice-based, 128-bit quantum security) |
| DDoS | Tor hidden services — no IP to target (when using Tor) |
| 51% attack | aBFT requires 2/3 + 1 honest; linear voting is Sybil-neutral |
| Floating-point non-determinism | Zero f32/f64 in consensus; all u128 integer math |
| Double spending | Per-account chain with `previous` hash linking; aBFT finality |
| Sybil attack | 1 LOS registration minimum; 1,000 LOS for reward eligibility and quorum weight |
| Inflation bug | Fixed supply with checked arithmetic and u128 overflow protection |
| Network partition | aBFT liveness requires ≥2/3 validators; view change on leader failure |

### Determinism Guarantee

All consensus-critical computation uses:
- `u128` integer arithmetic (no floating-point)
- `checked_mul`, `checked_add` (overflow protection)
- `isqrt` via Newton's method for AMM/DEX math (deterministic across all platforms)
- Sorted arrays for median (deterministic ordering)
- Basis points (10,000 = 100%) instead of percentages

This ensures all validators compute identical results regardless of CPU architecture, OS, or compiler version.

---

## Performance

### Target Metrics

| Metric | Target | Achieved |
|---|---|---|
| Transaction finality | < 3 seconds | ~2s (over Tor) |
| API response (Tor) | < 2 seconds | 500ms–2s |
| API response (local) | < 5ms | <5ms |
| P2P gossip round-trip | < 3 seconds | 1s–3s (over Tor) |
| Validator uptime | > 95% | Monitored per epoch |
| Concurrent accounts | Unlimited | Per-account chains scale horizontally |

### Bottleneck: Tor Latency

Tor adds ~500ms–2s per hop. This is the primary performance constraint and is an explicit trade-off for privacy. The block-lattice structure mitigates this — since each account has its own chain, parallel transactions on different accounts don't wait for each other.

---

*Unauthority (LOS) — Lattice Of Sovereignty*
*100% Immutable. 100% Permissionless. 100% Decentralized.*
