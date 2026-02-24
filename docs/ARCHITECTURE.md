# Architecture — Unauthority (LOS) v2.0.1

System design, crate structure, data flow, and technical decisions for the Unauthority blockchain.

---

## System Overview

Unauthority is a block-lattice (DAG) blockchain where each account maintains its own chain of blocks. A global ledger state is maintained via aBFT consensus across validators communicating exclusively over Tor hidden services.

```
┌──────────────────────────────────────────────────────────────┐
│              Flutter Wallet / Validator Dashboard             │
│           (Dart + Rust crypto via flutter_rust_bridge)        │
└─────────────────────────┬────────────────────────────────────┘
                          │ REST / gRPC over Tor (.onion)
┌─────────────────────────▼────────────────────────────────────┐
│                        los-node                              │
│  ┌──────────┐ ┌───────────┐ ┌──────────┐ ┌───────────┐      │
│  │ REST API │ │ gRPC API  │ │ P2P      │ │ CLI REPL  │      │
│  │ (Warp)   │ │ (Tonic)   │ │ Gossip   │ │ (stdin)   │      │
│  └────┬─────┘ └─────┬─────┘ └────┬─────┘ └─────┬─────┘      │
│       └──────────────┴────────────┴─────────────┘            │
│                          │                                    │
│  ┌───────────────────────▼──────────────────────────────┐    │
│  │           Shared State (Arc<RwLock<>>)                │    │
│  │   Ledger · Mempool · Oracle · Slashing · Rewards     │    │
│  └──────────────────────────────────────────────────────┘    │
│                          │                                    │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐      │
│  │ los-core │ │los-consen│ │los-crypto│ │  los-vm    │      │
│  │          │ │   sus    │ │          │ │            │      │
│  └──────────┘ └──────────┘ └──────────┘ └────────────┘      │
│                                          ┌────────────┐      │
│                                          │los-contracts│     │
│                                          │(USP-01/DEX)│      │
│                                          └────────────┘      │
└──────────────────────────────────────────────────────────────┘
                          │
              ┌───────────▼───────────┐
              │   Tor Hidden Service  │
              │   (.onion network)    │
              └───────────────────────┘
```

---

## Crate Dependency Graph

```
los-node (main binary, ~9000 lines)
├── los-core         (blockchain primitives, ~3000 lines)
├── los-consensus    (aBFT, slashing, checkpoints, ~2500 lines)
│   └── los-core
├── los-network      (P2P, Tor transport, fee scaling, ~1800 lines)
│   └── los-core
├── los-crypto       (Dilithium5, SHA-3, ~800 lines)
├── los-vm           (WASM smart contracts, ~1200 lines)
│   └── los-core
├── los-contracts    (USP-01 token, DEX AMM — WASM #![no_std], ~1500 lines)
│   └── los-sdk
├── los-cli          (CLI wallet, ~500 lines)
│   ├── los-core
│   └── los-crypto
└── los-sdk          (External integration SDK, ~300 lines)
```

---

## Crate Details

### los-core

Core blockchain primitives and state management. No I/O — pure logic.

| Module | Purpose |
|---|---|
| `lib.rs` | `Block`, `AccountState`, `Ledger`, `BlockType`, PoW, genesis loading |
| `distribution.rs` | Supply distribution tracking, burn accounting (u128 arithmetic) |
| `validator_config.rs` | Validator configuration structures |
| `validator_rewards.rs` | Reward pool distribution: `budget × stake / Σ(all_stakes)` (linear) |
| `pow_mint.rs` | PoW mining engine: SHA3-256, epoch management, proof verification |

**Key design decisions:**
- All monetary values stored as `u128` CIL (atomic units)
- All price values stored as `u128` micro-USD (1 USD = 1,000,000)
- Zero `f32`/`f64` in any consensus-critical path
- `Ledger` is the central state: `HashMap<String, AccountState>`

### los-consensus

aBFT consensus engine, validator coordination, and accountability.

| Module | Purpose |
|---|---|
| `abft.rs` | Asynchronous BFT consensus rounds, block finalization |
| `checkpoint.rs` | Periodic state checkpointing (RocksDB snapshots) |
| `slashing.rs` | Validator slashing: double-sign, fake burns, oracle manipulation |
| `voting.rs` | Linear voting: `vote_weight = stake` (1 LOS = 1 vote, Sybil-neutral) |

### los-network

Networking layer — all traffic over Tor.

| Module | Purpose |
|---|---|
| `tor_transport.rs` | SOCKS5 proxy connections, Tor auto-detection, onion address management |
| `p2p_integration.rs` | Peer management, connection tracking, peer table maintenance |
| `p2p_encryption.rs` | Noise Protocol encryption for P2P gossip channels |
| `fee_scaling.rs` | Anti-spam rate limiting and fee multiplier for high-frequency senders |
| `slashing_integration.rs` | Network-level slashing event propagation |
| `validator_rewards.rs` | Network-level reward distribution coordination |

**Key design decisions:**
- Auto-detects Tor SOCKS5 at `127.0.0.1:9050` with 500ms connection timeout
- Auto-discovers bootstrap peers from genesis config `.onion` addresses
- Gossip over HTTP POST through Tor — reliable at the cost of ~2s latency per hop

### los-crypto

Post-quantum cryptography via Dilithium5 (NIST FIPS 204).

| Function | Purpose |
|---|---|
| `generate_keypair()` | Dilithium5 key generation with deterministic BIP39 seed |
| `sign_message()` | Sign arbitrary bytes, returns hex signature |
| `verify_signature()` | Verify Dilithium5 signature against public key |
| `public_key_to_address()` | Derive LOS address from public key (SHA-3 hash, Base58) |

**Key specs:**
- Public key: ~2.5 KB
- Signature: ~4.6 KB
- Security level: 256-bit classical, 128-bit quantum

### los-node

Main validator binary — the heart of the system. Single binary, ~9000 lines.

| Module | Purpose |
|---|---|
| `main.rs` | REST API (Warp), P2P gossip, burn pipeline, epoch processing, CLI REPL |
| `grpc_server.rs` | gRPC API (Tonic) for structured client access |
| `genesis.rs` | Genesis config parsing, validation, account initialization |
| `db.rs` | RocksDB database layer for persistent ledger storage |
| `mempool.rs` | Transaction mempool management and prioritization |
| `metrics.rs` | Prometheus metrics (45+ gauges/counters/histograms) |
| `rate_limiter.rs` | API rate limiting per-IP and per-address |
| `testnet_config.rs` | Graduated testnet levels: functional / consensus / production |
| `validator_api.rs` | Validator-specific API handlers (register, unregister) |
| `validator_rewards.rs` | Epoch reward processing and distribution |

### los-vm

WASM Virtual Machine for smart contracts (Unauthority Virtual Machine — UVM).

| Module | Purpose |
|---|---|
| `lib.rs` | WASM runtime, contract deployment, execution, state management |
| `host.rs` | 16 host functions injected into WASM: state, events, transfers, crypto |
| `oracle_connector.rs` | Oracle price feed interface for smart contracts |

**Execution pipeline:**
1. **Hosted WASM** (Cranelift + deterministic gas metering via `wasmer-middlewares`)
2. Legacy WASM (backward compatibility, no host imports)
3. Mock dispatch (testnet only, `#[cfg(not(feature = "mainnet"))]`)

**Contract addressing:** Deterministic via `blake3(owner + ":" + nonce + ":" + block_number)` → `LOSCon` + first 32 hex chars.

### los-contracts

Production `#![no_std]` WASM smart contracts using `los-sdk`. Compiled to WebAssembly for deployment on the UVM.

| Module | Purpose |
|---|---|
| `usp01_token.rs` | USP-01 Native Fungible Token Standard (ERC-20 equivalent) |
| `dex_amm.rs` | Constant-product AMM (x·y=k) Decentralized Exchange |

**USP-01 Token Standard:**
- 11 entry points: `init`, `transfer`, `approve`, `transfer_from`, `burn`, `balance_of`, `allowance_of`, `total_supply`, `token_info`, `wrap_mint`, `wrap_burn`
- All values stored as decimal strings in contract state (`bal:{addr}`, `allow:{owner}:{spender}`)
- Supports native tokens AND wrapped assets (wBTC, wETH) via bridge operator model
- Events: `USP01:Init`, `USP01:Transfer`, `USP01:Approval`, `USP01:Burn`, `USP01:WrapMint`, `USP01:WrapBurn`

**DEX AMM:**
- 9 entry points: `init`, `create_pool`, `add_liquidity`, `remove_liquidity`, `swap`, `get_pool`, `quote`, `get_position`, `list_pools`
- Constant-product formula with 0.3% default fee (30 bps, configurable per pool)
- MEV protection via deadline parameter and slippage checks
- LP tokens minted proportionally: `isqrt(amount_a × amount_b) - MINIMUM_LIQUIDITY`
- 100% integer math (`u128`), zero floating-point

**Key design decisions:**
- Both contracts use `los-sdk` host functions exclusively — no `std` dependency
- State stored as `BTreeMap<String, String>` in the VM, persisted to sled DB
- Fully checked arithmetic with descriptive error messages (no panics)

### los-cli

Command-line interface for wallet and node management.

| Command | Purpose |
|---|---|
| `wallet` | Create/import wallet, check balance, export keys |
| `tx` | Send transactions, check status |
| `query` | Query blocks, accounts, supply, history |
| `validator` | Register/unregister as validator |

---

## Block-Lattice Structure

Unlike traditional blockchains (single chain of global blocks), Unauthority uses a block-lattice where each account has its own chain:

```
Account A:  [Genesis] ──→ [Send 50 to B] ──→ [Send 20 to C] ──→ ...
                              │
Account B:  [Genesis] ──→ [Receive 50 from A] ──→ [Send 10 to C] ──→ ...
                                                       │
Account C:  [Mint 100] ──→ [Receive 20 from A] ──→ [Receive 10 from B] ──→ ...
```

**Benefits:**
- Lock-free parallel processing — transactions on different accounts don't contend
- Instant sender-side confirmation — no waiting for global block
- Scalable throughput — adding accounts doesn't slow existing ones

### Block Types

| Type | Description | `link` field contains |
|---|---|---|
| `Send` | Debit from sender | Recipient address |
| `Receive` | Credit to receiver | Hash of the Send block |
| `Mint` | Token creation (genesis, PoW mining reward) | Source reference (`MINE:epoch:nonce`) |
| `Burn` | Token destruction (burning LOS) | Burn reference |
| `Change` | Representative/validator delegation | New representative |

### Block Fields

```rust
Block {
    account:    String,    // Owner address (LOS...)
    previous:   String,    // Hash of previous block in this account's chain
    block_type: BlockType, // Send | Receive | Mint | Burn | Change
    amount:     u128,      // Amount in CIL (atomic units)
    link:       String,    // Context-dependent reference
    signature:  String,    // Dilithium5 hex signature
    public_key: String,    // Dilithium5 hex public key
    work:       u64,       // Proof-of-Work nonce (anti-spam)
    timestamp:  u64,       // Unix timestamp
    fee:        u128,      // Transaction fee in CIL
}
```

---

## Data Flow: Send Transaction

```
1. Client creates Send block → signs with Dilithium5
2. POST /send → los-node REST API
3. los-node validates: signature, balance, PoW, fee, previous block hash
4. Block added to sender's account chain in Ledger
5. Gossip CONFIRM_REQ to all peers
6. Peers validate and return votes (weighted by stake)
7. Once ≥2/3 quorum reached → block finalized
8. Receive block auto-created on recipient's chain
9. Broadcast BLOCK message to all peers for ledger sync
```

## Data Flow: PoW Mining (Public Distribution)

```
1. Miner runs full validator node with --mine flag
2. Background thread grinds SHA3-256(LOS_MINE_V1 || chain_id || address || epoch || nonce)
3. When hash meets difficulty target (≥N leading zero bits):
   a. Creates Mint block with link = "MINE:epoch:nonce"
   b. Signs with Dilithium5
   c. Broadcasts via MINE_BLOCK:{json} gossip message
4. Receiving validators independently verify:
   a. PoW proof (SHA3-256 hash meets difficulty)
   b. Epoch deduplication (1 reward per address per epoch)
   c. Dilithium5 signature validity
   d. Link format ("MINE:epoch:nonce")
5. If valid → Mint block added to DAG, miner receives reward
```

---

## Security Model

| Layer | Mechanism |
|---|---|
| **Cryptography** | Dilithium5 (NIST PQC) — 256-bit classical, 128-bit quantum security |
| **Consensus** | aBFT — tolerates f < n/3 Byzantine validators |
| **Network** | Tor-only — no IP exposure, `.onion` hidden services |
| **Voting** | Linear voting: 1 LOS = 1 vote (Sybil-neutral) |
| **Determinism** | u128 integer math everywhere — zero `f32`/`f64` in consensus |
| **Anti-Spam** | Proof-of-Work nonce per block + flat BASE_FEE_CIL |
| **Accountability** | Slashing for double-signing, fake burns, oracle manipulation |
| **Privacy** | No KYC, no clearnet, Tor SOCKS5 for all traffic |
