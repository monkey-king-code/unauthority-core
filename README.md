# Unauthority (LOS) — Lattice Of Sovereignty

**A 100% Immutable, Permissionless, and Decentralized Blockchain.**

[![CI](https://github.com/monkey-king-code/unauthority-core/actions/workflows/ci.yml/badge.svg)](https://github.com/monkey-king-code/unauthority-core/actions)
[![Rust](https://img.shields.io/badge/rust-2021--edition-orange)]()
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)
[![Version](https://img.shields.io/badge/version-2.2.0-blue)]()

---

## What is Unauthority?

Unauthority is a post-quantum secure, block-lattice (DAG) blockchain with aBFT consensus. Tor hidden services (`.onion`) are **strongly recommended** for validator anonymity, but validators can also run on clearnet (IP/domain). No DNS required, no central point of failure.

| Property | Value |
|---|---|
| **Ticker** | LOS |
| **Atomic Unit** | CIL (1 LOS = 10¹¹ CIL) |
| **Total Supply** | 21,936,236 LOS (Fixed, non-inflationary) |
| **Consensus** | aBFT (Asynchronous Byzantine Fault Tolerance) |
| **Structure** | Block-Lattice (DAG) + Global State |
| **Cryptography** | Dilithium5 (Post-Quantum) + SHA-3 |
| **Network** | Tor Hidden Services (.onion) recommended; clearnet supported |
| **Smart Contracts** | WASM via UVM (Unauthority Virtual Machine) |
| **Token Standard** | USP-01 (Native Fungible + Wrapped Assets) |
| **DEX** | Constant-Product AMM (x·y=k), MEV Resistant |

---

## Why Unauthority?

- **Post-Quantum Secure** — Dilithium5 (NIST standard) resists both classical and quantum attacks
- **Tor-Recommended** — `.onion` strongly recommended for privacy; clearnet (IP/domain) also supported
- **DAG Architecture** — Parallel account processing, no global block contention
- **Fair Distribution** — 96.5% public via PoW Mining, only 3.5% dev allocation
- **Linear Voting** — 1 LOS = 1 Vote, Sybil-neutral stake-weighted consensus
- **Integer Math Only** — Zero floating-point in consensus. Fully deterministic across all nodes
- **USP-01 Token Standard** — Native fungible tokens + wrapped assets (wBTC, wETH) via WASM contracts
- **DEX AMM** — Constant-product decentralized exchange with MEV protection and slippage checks
- **Full CLI** — `los-cli` for wallet, transactions, validator ops, token management, and DEX trading

---

## Quick Start

### Run a Validator (3 steps)

```bash
# 1. Install Tor
sudo apt install -y tor && sudo systemctl enable --now tor   # Linux
brew install tor && brew services start tor                    # macOS

# 2. Build from source
git clone https://github.com/monkey-king-code/unauthority-core.git
cd unauthority-core && ./install.sh --mainnet

# 3. Run
export LOS_WALLET_PASSWORD='your-strong-password'
./target/release/los-node --port 3030 --data-dir /opt/los-node
```

**That's it.** The node automatically:
- Discovers bootstrap peers from genesis config (4 genesis validators)
- Detects Tor SOCKS5 proxy at `127.0.0.1:9050`
- Generates a Dilithium5 post-quantum wallet on first run
- Connects to the network and begins syncing

For full setup with Tor hidden service, systemd service, and monitoring, see the [Validator Guide](docs/VALIDATOR_GUIDE.md).


### Download Wallet & Validator Apps

> **macOS users:** After moving the app to `/Applications`, run:
> ```bash
> xattr -cr /Applications/LOS\ Wallet.app
> xattr -cr /Applications/LOS\ Validator\ \&\ Miner.app
> ```
> Or: System Settings → Privacy & Security → Open Anyway

| App | macOS | Windows | Linux |
|-----|-------|---------|-------|
| **LOS Wallet** | [LOS-Wallet-2.1.0-mainnet-macos.dmg](https://github.com/monkey-king-code/unauthority-core/releases/download/wallet-v2.1.0-mainnet/LOS-Wallet-2.1.0-mainnet-macos.dmg) | [LOS-Wallet-2.1.0-mainnet-windows-x64.zip](https://github.com/monkey-king-code/unauthority-core/releases/download/wallet-v2.1.0-mainnet/LOS-Wallet-2.1.0-mainnet-windows-x64.zip) | [LOS-Wallet-2.1.0-mainnet-linux-x64.tar.gz](https://github.com/monkey-king-code/unauthority-core/releases/download/wallet-v2.1.0-mainnet/LOS-Wallet-2.1.0-mainnet-linux-x64.tar.gz) |
| **LOS Validator Node** | [LOS-Validator-2.1.0-mainnet-macos.dmg](https://github.com/monkey-king-code/unauthority-core/releases/download/validator-v2.1.0-mainnet/LOS-Validator-2.1.0-mainnet-macos.dmg) | [LOS-Validator-2.1.0-mainnet-windows-x64.zip](https://github.com/monkey-king-code/unauthority-core/releases/download/validator-v2.1.0-mainnet/LOS-Validator-2.1.0-mainnet-windows-x64.zip) | [LOS-Validator-2.1.0-mainnet-linux-x64.tar.gz](https://github.com/monkey-king-code/unauthority-core/releases/download/validator-v2.1.0-mainnet/LOS-Validator-2.1.0-mainnet-linux-x64.tar.gz) |

**Windows:** Right-click `.exe` → Properties → Unblock, then launch. If SmartScreen appears: click "More info" → "Run anyway"  
**Linux:** `chmod +x run.sh flutter_wallet` (or `flutter_validator los-node`), then `./run.sh`.

The wallet and validator bundle Tor internally — no separate Tor installation required.

---

## Architecture

```
unauthority-core/
├── crates/
│   ├── los-node/         # Validator binary (REST + gRPC + P2P + consensus)
│   ├── los-core/         # Blockchain primitives (Block, Tx, Ledger)
│   ├── los-consensus/    # aBFT consensus, checkpointing, slashing
│   ├── los-network/      # Tor transport, P2P encryption, fee scaling
│   ├── los-crypto/       # Dilithium5 keygen, signing, verification
│   ├── los-vm/           # WASM smart contract engine (UVM)
│   ├── los-contracts/    # USP-01 token + DEX AMM (WASM, #![no_std])
│   ├── los-cli/          # Command-line wallet & node management
│   └── los-sdk/          # SDK for external integrations
├── flutter_wallet/       # Mobile/Desktop user wallet (Flutter + Rust via FRB)
├── flutter_validator/    # Validator dashboard app (Flutter + Rust via FRB)
├── genesis/              # Genesis block generator & configuration
├── examples/contracts/   # Sample WASM smart contracts (DEX, Token)
└── tests/                # Integration & E2E test suites
```

See [Architecture Deep Dive](docs/ARCHITECTURE.md) for detailed crate documentation and data flow.

---

## Token Economics

| Allocation | Amount (LOS) | Percentage |
|---|---|---|
| **Public (PoW Mining)** | 21,158,413 | ~96.5% |
| **Dev Treasury** | 773,823 | ~3.5% |
| **Bootstrap Validators (4×1,000)** | 4,000 | ~0.02% |
| **Total** | **21,936,236** | **100%** |


### How to Acquire LOS (PoW Mining)

LOS tokens are distributed through **Proof-of-Work Mining**. Miners run a full validator node (`los-node --mine`) and solve SHA3-256 hash puzzles to earn block rewards.

**Mining Mechanics:**
- **Algorithm:** `SHA3-256(LOS_MINE_V1 ‖ chain_id ‖ address ‖ epoch ‖ nonce)`
- **Reward:** 100 LOS per epoch (1 hour), halving every 8,760 epochs (~1 year)
- **Difficulty:** Starts at 20 leading zero bits, auto-adjusts based on miner count
- **Deduplication:** 1 reward per address per epoch — no double-mining
- **Requirement:** Must run a full validator node (no external mining API)

> **🏆 100% Fair Distribution — Zero Dev Rewards:**
>
> There is no pre-mine, no ICO, no token sale. Mining is the **only** way to acquire LOS from the public pool. Anyone can mine by running a validator node.
>
> **The 4 genesis bootstrap validators do NOT receive any mining rewards or validator epoch rewards.** All rewards are reserved exclusively for public participants. The dev treasury (3.5%) is pre-allocated in genesis for development costs only — it does not grow. Bootstrap validator nodes secure the network from day one but earn zero rewards, ensuring maximum fairness for all public miners and validators.

### Validator Rewards

- **Pool:** 500,000 LOS (non-inflationary, from total supply)
- **Per Epoch:** 5,000 LOS, halving every 48 epochs (~4 years)
- **Formula:** `reward_i = budget × stake_i / Σ(all_stakes)` (pure linear, Sybil-neutral)
- **Eligibility:** Min 1,000 LOS stake, ≥95% uptime, **non-genesis validator**
- **Genesis Exclusion:** The 4 bootstrap validators are explicitly excluded from epoch rewards

---

## API Overview

The validator node exposes a REST API (35+ endpoints) and a gRPC API.

| Method | Endpoint | Description |
|---|---|---|
| GET | `/health` | Health check |
| GET | `/node-info` | Node version, peers, block count |
| GET | `/supply` | Total supply and remaining supply |
| GET | `/bal/{address}` | Account balance |
| GET | `/account/{address}` | Full account details + history |
| GET | `/history/{address}` | Transaction history |
| GET | `/validators` | Active validator list with stake info |
| GET | `/consensus` | aBFT consensus status and safety |
| GET | `/peers` | Connected peers + validator endpoints |
| GET | `/block` | Latest block |
| GET | `/blocks/recent` | Recent blocks |
| GET | `/reward-info` | Reward pool & epoch info |
| GET | `/metrics` | Prometheus-compatible metrics |
| POST | `/send` | Send LOS transaction |
| POST | `/register-validator` | Register as network validator |
| POST | `/deploy-contract` | Deploy WASM smart contract |
| POST | `/call-contract` | Execute smart contract function |

Full documentation with request/response examples: [API Reference](docs/API_REFERENCE.md)

---

## Node Configuration

### CLI Flags

```bash
./target/release/los-node [OPTIONS]
```

| Flag | Description | Default |
|---|---|---|
| `--port <PORT>` | REST API listen port | `3030` |
| `--data-dir <DIR>` | Data storage directory | `node_data/node-{port}/` |
| `--node-id <ID>` | Node identifier for logs | `node-{port}` |
| `--json-log` | JSON log output (for Flutter dashboard) | off |
| `--config <FILE>` | Load config from TOML file | none |

### Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `LOS_WALLET_PASSWORD` | **Mainnet** | — | Wallet encryption password |
| `LOS_ONION_ADDRESS` | No | Auto-read from Tor | Your `.onion` address |
| `LOS_SOCKS5_PROXY` | No | Auto-detect `127.0.0.1:9050` | Tor SOCKS5 proxy address |
| `LOS_BOOTSTRAP_NODES` | No | Auto from genesis config | Comma-separated `host:port` peers |
| `LOS_NODE_ID` | No | `node-{port}` | Node identifier |
| `LOS_BIND_ALL` | No | `0` | Set `1` to bind to `0.0.0.0` |
| `LOS_P2P_PORT` | No | REST+1000 | P2P gossip listen port |
| `LOS_TESTNET_LEVEL` | No | `consensus` | Testnet mode: `functional`/`consensus`/`production` |

### Port Scheme

| Service | Port | Derivation |
|---|---|---|
| REST API | 3030 | `--port` value |
| P2P Gossip | 4030 | REST + 1000 |
| gRPC | 23030 | REST + 20000 |

---

## Documentation

### For Users & Node Operators
| Document | Description |
|---|---|
| [Validator Guide (CLI)](docs/VALIDATOR_GUIDE.md) | Complete setup: build, Tor, systemd, monitoring, rewards |
| [Validator Guide (Flutter)](docs/FLUTTER_VALIDATOR_GUIDE.md) | Step-by-step tutorial for the desktop validator app |
| [API Reference](docs/API_REFERENCE.md) | All 35+ REST & gRPC endpoints with examples |
| [Tor Setup](docs/TOR_SETUP.md) | Tor hidden service configuration & troubleshooting |
| [Whitepaper](docs/WHITEPAPER.md) | Technical whitepaper: design, consensus, economics |
| [Genesis Transparency](docs/GENESIS_TRANSPARENCY.md) | All genesis wallet addresses & public balance verification API |
| [Architecture](docs/ARCHITECTURE.md) | System design, crate map, data flow diagrams |
| [Exchange Integration](docs/EXCHANGE_INTEGRATION.md) | RPC documentation for exchanges & integrators |
| [Smart Contracts](docs/SMART_CONTRACTS.md) | Write, compile, and deploy WASM contracts on UVM |

### For Developers
| Document | Description |
|---|---|
| [Contributing](CONTRIBUTING.md) | Contribution guidelines, code standards, PR process |
| [Code of Conduct](CODE_OF_CONDUCT.md) | Community standards (Contributor Covenant) |
| [Security Policy](SECURITY.md) | Responsible disclosure and security contacts |
| [Changelog](CHANGELOG.md) | Version history and release notes |

---

## Build & Test

```bash
# Build (testnet, default)
cargo build --release

# Build (mainnet — strict mode: no faucet, enforced signatures)
cargo build --release -p los-node --features mainnet

# Run all tests (309 tests across 10 crates + E2E)
cargo test --release --workspace --all-features

# Run clippy (zero warnings enforced)
cargo clippy --workspace --all-features -- -D warnings

# Run specific crate tests
cargo test --release -p los-core
cargo test --release -p los-consensus
cargo test --release -p los-crypto
```

---

## License

AGPL-3.0 — See [LICENSE](LICENSE)
