# Technical Whitepaper — Unauthority (LOS) v1.0.13

**Lattice Of Sovereignty: A Post-Quantum, Privacy-First Block-Lattice Blockchain**

*Last updated: February 2026*

---

## Table of Contents

1. [Abstract](#abstract)
2. [Design Principles](#design-principles)
3. [Block-Lattice Architecture](#block-lattice-architecture)
4. [Consensus: aBFT](#consensus-abft)
5. [Token Economics](#token-economics)
6. [Proof-of-Burn Bridge](#proof-of-burn-bridge)
7. [Oracle Consensus](#oracle-consensus)
8. [Validator Rewards](#validator-rewards)
9. [Linear Voting & Security](#linear-voting--security)
10. [Slashing & Accountability](#slashing--accountability)
11. [Post-Quantum Cryptography](#post-quantum-cryptography)
12. [Network Layer: Tor-Only](#network-layer-tor-only)
13. [Smart Contracts (UVM)](#smart-contracts-uvm)
14. [Security Analysis](#security-analysis)
15. [Performance](#performance)

---

## Abstract

Unauthority (ticker: **LOS** — Lattice Of Sovereignty) is a fully decentralized, permissionless blockchain that operates exclusively over Tor hidden services. It uses a block-lattice (DAG) structure where each account maintains its own chain, enabling lock-free parallel transaction processing. Consensus is achieved via an asynchronous Byzantine Fault Tolerant (aBFT) protocol using post-quantum Dilithium5 signatures. The native token has a fixed supply of 21,936,236 LOS with no inflation.

Key differentiators:
- **100% Tor-native** — no clearnet, no IP exposure, no DNS
- **Post-quantum** — Dilithium5 (NIST FIPS 204) with 256-bit classical security
- **Deterministic** — all consensus math uses u128 integer arithmetic, zero floating-point
- **Sybil-neutral** — linear voting (1 LOS = 1 vote), no concentration advantage
- **Interoperable** — Proof-of-Burn bridges for wrapped assets (wBTC, wETH)

---

## Design Principles

1. **Immutability** — No governance override, no admin keys, no emergency pauses
2. **Permissionless** — Anyone can run a validator with 1,000 LOS stake
3. **Privacy** — All traffic routed through Tor; no KYC, no clearnet dependency
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
| **Mint** | Token creation (genesis or burn reward) | Source reference |
| **Burn** | Proof-of-Burn event recording | External TX hash |
| **Change** | Representative/validator delegation | New representative |

### Block Fields

| Field | Type | Description |
|---|---|---|
| `account` | String | Owner address (LOS...) |
| `previous` | String | Hash of previous block in this chain |
| `block_type` | Enum | Send, Receive, Mint, Burn, Change |
| `amount` | u128 | Amount in CIL (atomic units) |
| `link` | String | Context-dependent reference |
| `signature` | String | Dilithium5 hex signature |
| `public_key` | String | Dilithium5 hex public key |
| `work` | u64 | Proof-of-Work nonce (anti-spam) |
| `timestamp` | u64 | Unix timestamp |
| `fee` | u128 | Dynamic fee in CIL |

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
| **Dev Treasury** | 777,823 LOS (~3.5%) |
| **Public Allocation** | 21,158,413 LOS (~96.5%) |
| **Reward Pool** | 500,000 LOS (from Dev Treasury, non-inflationary) |

### Dev Treasury Breakdown

| Allocation | Amount (LOS) | Purpose |
|---|---|---|
| Dev Treasury 1 | 428,113 | Core development |
| Dev Treasury 2 | 245,710 | Development operations |
| Dev Treasury 3 | 50,000 | Community grants |
| Dev Treasury 4 | 50,000 | Emergency fund |
| Bootstrap Validators | 4,000 | 4 validators × 1,000 LOS stake |
| **Total** | **777,823** | |

### Public Supply Distribution

The public allocation (21,158,413 LOS) is distributed exclusively through Proof-of-Burn:

```
Public Supply Cap = 21,158,413 × 10^11 CIL = 2,115,841,300,000,000,000 CIL
```

No other mechanism exists to create new tokens. Once the public supply is fully distributed, no more LOS can be minted.

---

## Proof-of-Burn Bridge

### Mechanism

Users burn external cryptocurrency (BTC, ETH) to receive LOS tokens:

1. User sends BTC/ETH to a provably unspendable dead address
2. User submits burn transaction hash to an Unauthority validator
3. Validators independently verify the burn on the source chain
4. Oracle consensus determines the USD value at burn time
5. LOS yield calculated based on remaining public supply
6. Once ≥2 validators confirm, a Mint block is created

### Yield Formula (Integer-Only)

```
yield_cil = (burn_amount_usd × remaining_supply) / PUBLIC_SUPPLY_CAP

where:
  burn_amount_usd  = USD value in $0.01 units (integer)
  remaining_supply = remaining public supply in CIL
  PUBLIC_SUPPLY_CAP = 21,158,413 × 10^11 CIL
```

All arithmetic uses `u128` with `checked_mul` to prevent overflow. Maximum values:
- `burn_amount_usd`: ~10^12 (practical max: burning $10B worth)
- `remaining_supply`: ~10^22 (at genesis)
- Product: ~10^34 (fits in u128, max ~10^38)

### Scarcity Curve

As more LOS is distributed, the yield per dollar burned decreases:

```
At 0% distributed:  yield_cil ≈ burn_usd (1:1 ratio)
At 50% distributed: yield_cil ≈ burn_usd × 0.5
At 90% distributed: yield_cil ≈ burn_usd × 0.1
At 99% distributed: yield_cil ≈ burn_usd × 0.01
```

This creates a fair, market-driven token distribution — early participants get better rates, but anyone can participate at any time.

---

## Oracle Consensus

### Purpose

The decentralized oracle provides real-time price feeds (ETH/USD, BTC/USD) needed for Proof-of-Burn yield calculations. All prices are determined by validator consensus — no external oracle service.

### Price Format

All prices stored as **micro-USD (u128)**:

```
1 USD = 1,000,000 micro-USD
$2,500.00 ETH = 2,500,000,000 micro-USD
$83,000.00 BTC = 83,000,000,000 micro-USD
```

### Aggregation: BFT Median

```
1. Each validator fetches ETH/BTC prices from public APIs
2. Submits signed price to peers (ORACLE_SUBMIT message)
3. Within 60-second window, collect all submissions
4. Sort prices, calculate median:
   - Odd count: exact middle element
   - Even count: integer average of two middle elements
5. Reject outliers: |price - median| / median > 20%
6. Minimum 2 valid submissions required (BFT: 2f+1 for n≥3)
```

### Security

- Zero-price submissions are rejected
- Outlier threshold: 20% (2,000 basis points) deviation from median
- Uses u128 integer — cannot be NaN, Infinity, or negative
- Signed with Dilithium5 — submissions are attributable
- Oracle manipulation triggers slashing

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

All conditions must be met:

| Requirement | Threshold |
|---|---|
| Minimum stake | 1,000 LOS |
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
| 1 whale | 10,000 LOS | 10,000 |
| 10 small validators | 1,000 LOS each | 10 × 1,000 = 10,000 |

**Result:** Splitting stake across multiple identities yields the same total power. This is **Sybil-neutral** — no advantage to splitting or concentrating.

### Flat Fee Model

All transactions pay the same flat `BASE_FEE_CIL` (0.000001 LOS). No dynamic fee scaling.

### Burn Cap

Maximum burn per block: 1,000 LOS. Prevents rapid supply draining.

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

### Hash Function: SHA-3 (Keccak)

All block hashing uses SHA-3 (FIPS 202):

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

## Network Layer: Tor-Only

### Architecture

All network traffic is routed through Tor hidden services:

```
Node A (.onion) ←→ Tor Network ←→ Node B (.onion)
```

- **No clearnet** — validators have no public IP exposure
- **No DNS** — `.onion` addresses are derived from Tor keys
- **NAT traversal** — works behind any firewall/router

### Peer Discovery (v1.0.9+)

1. Node starts and reads genesis config (embedded at compile-time)
2. Extracts `.onion` addresses of bootstrap validators
3. Auto-detects Tor SOCKS5 proxy at `127.0.0.1:9050` (500ms timeout)
4. Connects to bootstrap peers via SOCKS5
5. Downloads dynamic peer table from connected peers
6. Maintains peer table sorted by latency/uptime

### P2P Communication

All gossip messages are HTTP POST requests routed through Tor:

| Message | Purpose |
|---|---|
| `ID` | Node identity announcement |
| `BLOCK` | New block broadcast |
| `CONFIRM_REQ` | Request confirmation votes |
| `VOTE_REQ` | Request burn verification votes |
| `VOTE_RES` | Burn verification vote response |
| `ORACLE_SUBMIT` | Oracle price submission |

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

### Oracle Connector

Smart contracts can access oracle price feeds:

```rust
// Inside a WASM contract
let eth_price = oracle_get_price("ETH/USD");
let btc_price = oracle_get_price("BTC/USD");
```

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
| DDoS | Tor hidden services — no IP to target |
| 51% attack | aBFT requires 2/3 + 1 honest; linear voting is Sybil-neutral |
| Oracle manipulation | BFT median, 20% outlier rejection, slashing for fraud |
| Floating-point non-determinism | Zero f32/f64 in consensus; all u128 integer math |
| Double spending | Per-account chain with `previous` hash linking; aBFT finality |
| Front-running (MEV) | Block-lattice doesn't have a mempool ordering advantage |
| Sybil attack | 1,000 LOS minimum stake for validators |
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
