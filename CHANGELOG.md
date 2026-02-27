# Changelog

All notable changes to the Unauthority (LOS) project are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/).

---

## [2.2.1] — 2026-02-27

> **Scope:** Documentation accuracy audit — align all `.md` files with actual codebase.

### Fixed (Documentation)

- **SECURITY.md** — Supported version updated from `2.0.1` to `2.2.x`.
- **EXCHANGE_INTEGRATION.md** — Corrected "Tor exclusively" to "Tor recommended; clearnet supported". Fixed nonexistent `GET /fee-info` endpoint → actual `GET /fee-estimate/{address}`. Fixed `BASE_FEE_CIL` from `1,000,000` to `100,000` (matching code). Updated `/bal` response format to match actual API (removed `status`, `pending_receives`; added `balance_cil_str`, `block_count`, `head`).
- **ARCHITECTURE.md** — Corrected "exclusively over Tor" to "over Tor (recommended) or clearnet".
- **TOR_SETUP.md** — Corrected "runs exclusively on Tor" to "Tor strongly recommended". Added clearnet alternative documentation.
- **WHITEPAPER.md** — Replaced nonexistent `LOS_TOR_ENABLED=true` env var with actual auto-detect behavior in Transport Modes table.
- **MINING_GUIDE.md** — Fixed wrong git clone URL (`unauthority/` → `monkey-king-code/`). Replaced nonexistent `--generate-key` flag with actual wallet auto-generation on first launch. Replaced nonexistent `LOS_TOR_ENABLED=true` with actual Tor auto-detection behavior.
- **VALIDATOR_GUIDE.md** — Added missing `LOS_HOST_ADDRESS` environment variable to config table.
- **README.md** — Added missing `LOS_HOST_ADDRESS` environment variable to env table.

---

## [2.2.0] — 2026-02-27

> **Scope:** State root integrity upgrade + orphan block cleanup. Protocol-level change requiring mainnet reset.

### Changed

- **`compute_state_root()`** — Now hashes `(address, balance, block_count, head)` per account (previously balance-only). Detects ghost blocks that corrupt chain structure, not just balance divergence. *(Protocol breaking — requires network-wide upgrade.)*
- **All block count APIs** (`/node-info`, `/block`, `/blocks/recent`, `/health`) — Use `total_chain_blocks()` (sum of account chain counts) instead of `l.blocks.len()` (HashMap size). Excludes orphaned blocks from reported counts.
- **`/account/:addr` transaction history** — Now walks the account chain from head→previous for deterministic newest→oldest order. Previously used arbitrary HashMap iteration.
- **`/blocks/recent`** — Filters blocks through chain-walk validation, showing only blocks in valid account chains.
- **Version bumped to 2.2.0** across all 10 Rust crates.

### Added

- **`Ledger::total_chain_blocks()`** — Returns sum of `block_count` across all accounts (orphan-safe).
- **`Ledger::remove_orphaned_blocks()`** — Walks all account chains via head→previous links, removes blocks not reachable from any chain. Called at startup and after every sync path.
- **Orphan cleanup at 4 points:** node startup, REST sync merge, SYNC_GZIP fast-path adoption, SYNC_GZIP slow-path merge.

### Fixed

- **Ghost block bug** — MINE_BLOCK gossip handler had a direct insertion fallback that bypassed `process_block()` validation, creating orphaned blocks in `l.blocks` HashMap with broken chain linkage. Root cause of block count mismatch between nodes (VPS2=39, VPS1=38). Removed direct insertion; blocks now recover via periodic SYNC_REQUEST.
- **Deadlock in slow-path sync** — `safe_lock(&ledger)` was called after slow-path loop while the outer scope still held the lock. Now reuses the existing `l` guard.
- **GossipSub "Duplicate" log spam** — Suppressed normal duplicate message warnings in small mesh networks.

### Security

- **State root now tamper-evident for chain structure** — Previous balance-only hash could not detect orphaned blocks or head divergence between nodes. The new hash covers the full account state tuple.

### Flutter Apps

- **flutter_validator v2.1.0** — No changes required. All API field types (`block_height`, `height`, `blocks`) are JSON numbers; Dart `json.decode` handles `u64`/`usize` identically.
- **flutter_wallet v2.1.0** — No changes required. Same JSON wire format compatibility.

---

## [2.1.0] — 2026-02-25

> **Scope:** Full mainnet audit release — all Rust crates + Flutter apps bumped to v2.1.0.

### Changed

- **Version bumped to 2.1.0** across all 10 Rust crates, both Flutter apps, all docs, README badges, build scripts, and download links.
- **Comprehensive code comment audit** — Removed 24 non-technical comments (AI language, Indonesian text, emoji prefixes, filler words) across 5 Rust source files. All comments now explain logic/function only.
- **Non-public scripts moved** — `_audit_scan.py`, `audit_clean_mainnet.sh`, `build_dmg.sh`, `test_bundled_tor.sh` moved to `dev_docs/` (gitignored).
- **New documentation:**
  - `docs/WHITEPAPER.md` — Comprehensive 17-section technical whitepaper (~680 lines) with code proofs and mathematical supply verification.
  - `docs/WHY_UNAUTHORITY.md` — 13-section proof document (~420 lines) demonstrating speed, security, determinism, and fair distribution with source code references.
  - `docs/MINING_GUIDE.md` — Miner acquisition guide (~280 lines) with halving tables, difficulty explanation, and quick start.

### Fixed

- **Flutter Validator `sendTransaction` broken** — Was constructing a local-only `Block` struct and never submitting to the API. Now correctly uses `BlockConstructionService` to build and submit via `POST /send`.
- **Flutter Validator missing `BlockConstructionService`** — Created new service for server-side block construction (signing_hash, PoW, Dilithium5 signing via FFI).
- **Flutter Validator `getHistory` always empty** — Was calling `GET /account/{address}` (returns balance, not history). Fixed to call `GET /history/{address}`.
- **Flutter Validator mainnet localhost priority** — `getApiBaseUrl()` tried localhost:3030 first, defeating the purpose of external peer discovery. Fixed to prioritize `/peers` discovery, use external peers only.
- **Flutter Validator local fallback POST bug** — `_tryLocalFallback()` used `POST /account/{address}` instead of `GET`. Fixed to GET.
- **Flutter Validator `isMainnet` was compile-time** — `const isMainnet` baked in at build. Changed to runtime network config detection.
- **Flutter Wallet/Validator missing `Account` fields** — Added `representative`, `modified_timestamp`, `block_count`, `frontier` to match API response.
- **Flutter Wallet missing `getFeeEstimate`** — Had `estimateFee()` returning hardcoded value. Added proper `GET /fee-estimate` API call.
- **Rust test `test_validator_rewards_epoch`** — Was using `is_genesis=true`, making test meaningless (genesis always ineligible). Fixed to `is_genesis=false`.
- **Rust `_balance` variable scoping** — `Ok(balance) => balance` in validator registration was shadowed. Fixed to `Ok(bal) => bal`.

### Audited (0 issues remaining)

- **0 `TODO` / `FIXME` / `HACK`** in production code
- **0 `unimplemented!()` / `todo!()`** in production code
- **0 `unwrap()` / `.expect()`** in production code (all in `#[test]` only)
- **0 `f32` / `f64`** in consensus/financial logic (3 display-only + 1 `#[cfg(not(feature = "mainnet"))]`)
- **0 warnings** on `cargo build --release`
- **320 tests passed, 0 failed**
- **Flutter analyze: 0 errors, 0 warnings** (both apps)

---

## [2.0.2] — 2026-02-24

> **Scope:** Flutter apps only (`flutter_wallet`, `flutter_validator`). Rust crates remain at v2.0.1.

### Changed

- **Flutter apps bumped to 2.0.2** — `flutter_wallet` and `flutter_validator` pubspec.yaml, README badges, build scripts, and download links updated.
- **Bootstrap node onion addresses updated** — All 4 genesis validators now use newly generated `.onion` addresses for mainnet deployment. Network configs in `flutter_wallet/assets/network_config.json` and `flutter_validator/assets/network_config.json` updated.
- **Mining disabled on bootstrap nodes** — Genesis validators no longer run `--mine` to keep server resources free; PoW mining rewards are reserved for public miners.

### Fixed (los-core / los-node v2.0.1)

- **Supply audit double-counting** — Validator reward pool (500,000 LOS) was counted in both `distribution.remaining_supply` and `reward_pool_remaining`, causing false inflation alerts. Removed `reward_pool_remaining` from audit formula.
- **Tor Health Monitor port** — Self-ping URL used default port 80 instead of the actual API port. Fixed to use `api_port` for correct Tor circuit validation.

---

## [2.0.1] — 2026-02-24

### Changed

- **Version bumped to 2.0.1** across all crates, Flutter apps, docs, configs, badges, and build scripts.

### Fixed

- **Window titles standardized** — Validator: "LOS Validator & Miner" (Linux/Windows). Wallet: "LOS Wallet" (Windows `main.cpp`).
- **Dockerfile multi-network build** — Added `ARG NETWORK=testnet` with conditional `--features mainnet` for mainnet builds.
- **Dev path leak in release builds** — `NodeProcessService` dev paths gated behind `kDebugMode`.
- **Validator registration docs** — Corrected minimum from 1,000 → 1 LOS for registration, 1,000 LOS for rewards across all docs.
- **PoW Mining documentation** — Added mining section to Flutter Validator Guide with `--mine` and `--mine-threads` CLI flags.
- **Ed25519 verbose comments** — Condensed to single-line note in both wallet services.
- **Homebrew path leak** — Removed hardcoded `/opt/homebrew/bin/tor` from TOR_IMPLEMENTATION.md.
- **Windows Tor README** — Added auto-download note for Tor Expert Bundle.
- **Validator uptime model** — `uptimeBps` (basis points, int) for mainnet-safe uptime display.
- **`validator.toml` documentation** — Added sample configuration section to Validator Guide.

### Removed

- **Web platform directories** — Deleted `flutter_wallet/web/` and `flutter_validator/web/` (web platform not supported).
- **Dev/test scripts** — Moved regression test and helper scripts to `dev_docs/scripts/` (gitignored).
- **Old UAT release artifacts** — Removed v1.0.x `.dmg` installers from `release/`.

---

## [2.0.0] — 2026-02-24

### Changed

- **Major version bump to 2.0.0** — System architecture redesign. Consensus, networking, mining, and reward systems fully stabilized.
- **Version bumped to 2.0.0** across all crates, Flutter apps, docs, configs, badges, and build scripts.

### Fixed

- **Validator uptime tracking** — `is_eligible()` now uses `display_uptime_pct()` (max of current + last epoch) instead of raw `uptime_pct()`, preventing false "0% uptime" at epoch boundaries.
- **Mid-epoch registration uptime** — `uptime_pct()` returns 100% when `expected_heartbeats == 0` but heartbeats recorded (newly registered validator).
- **`/reward-info` API consistency** — Shows `display_uptime_pct()` instead of raw `uptime_pct()`.

### Security

- **Removed unsafe `from_utf8_unchecked`** — Replaced with safe `from_utf8().unwrap_or_default()` in `dex_amm.rs` and `usp01_token.rs`.
- **Removed deprecated f64 function** — `get_capacity_percentage()` removed from `fee_scaling.rs`.
- **Mainnet safety audit passed** — 0 TODO, 0 unimplemented!(), 0 production unwrap(), 0 f64 in consensus, 0 panic in production paths.

---

## [1.0.13] — 2026-02-19

### Fixed\n\n- **Flutter Validator `--mainnet` flag** — `NodeProcessService` now passes `--mainnet` CLI flag to `los-node` binary when `NETWORK=mainnet`, matching the safety gate compile-time check.
- **`build_dmg.sh` mainnet build** — Local macOS DMG builder now compiles `los-node` with `--features mainnet` before bundling.

### Changed

- **Download naming: UAT → LOS** — All release artifacts renamed from `UAT-Wallet-*` / `UAT-Validator-*` to `LOS-Wallet-*` / `LOS-Validator-*` across all documentation and workflows.
- **Version bumped to 1.0.13** across all docs, configs, badges, Cargo.toml, and pubspec.yaml.

---

## [1.0.12] — 2026-02-19

### Security Hardening (11 Fixes)

#### HIGH
- **A-01: Seed phrase no longer cached in memory** — `NodeProcessService` no longer stores `_seedPhrase` as a class field. Re-reads from `FlutterSecureStorage` on demand only when needed.
- **F-01: macOS App Sandbox enabled for wallet** — `Release.entitlements` now enforces sandbox with scoped entitlements (`network.client`, `files.user-selected.read-write`, `keychain-access-groups`). Validator remains unsandboxed (requires child process spawn).

#### MEDIUM
- **A-02: Secret key hex exposure reduced** — Removed `secretKeyHex` getter (9,728-char hex string) from `DilithiumKeypair`. Replaced with `secretKeyBase64` (6,488 chars). Both wallet and validator updated.
- **A-03: Seed phrase reveal requires confirmation** — Added security confirmation dialog before showing mnemonic in Settings. User must explicitly press "REVEAL SEED PHRASE" to proceed.
- **B-01: Tor download integrity verification** — SHA-256 hash verification added after downloading Tor Expert Bundle. Deletes file on hash mismatch. Applied to both wallet and validator `TorService`.
- **F-02+K-01: Native library path restriction** — `DilithiumService` only searches bundled app paths (Frameworks, lib, exe-dir) in release builds. Development paths (native/target, workspace root, PATH) only available in debug mode.

#### LOW
- **G-01: Dependency version sync** — Wallet `pubspec.yaml` dependencies synced to validator's higher versions (`http`, `shared_preferences`, `intl`, `crypto`, `provider`).
- **E-01: Base58Check checksum verification** — Full pure-Dart implementation added to `AddressValidator`. Decodes Base58, verifies SHA-256d checksum (first 4 bytes). Rejects addresses with invalid checksums.
- **J-02: Binary discovery restriction** — `NodeProcessService._findNodeBinary()` restricted to bundled paths in release mode. Cargo build and PATH search only available in debug mode, preventing PATH hijacking.
- **F-03: Screenshot/recording warning** — Red warning banner displayed inside seed phrase reveal container alerting users about screen capture risks.
- **I-01: Auto-clearing clipboard** — New `SecureClipboard` utility auto-clears clipboard after 30s (sensitive data) or 60s (addresses). Applied to all 10 screens with copy functionality.

### Changed
- Version bumped to 1.0.12 across all docs, badges, and configs.

---

## [1.0.11] — 2026-02-18

### Fixed

- **Validator release: `los-node` now built with `--features mainnet`** — Previously bundled testnet binary (CHAIN_ID=2) in mainnet installer. All 3 platforms (macOS, Linux, Windows) now correctly build mainnet binary (CHAIN_ID=1).
- **Windows branding: UAT → LOS** — `Runner.rc` metadata corrected from "UAT Wallet" to "LOS Wallet" and "UAT Validator Node" to "LOS Validator Node" (affects Windows Task Manager & Properties dialog).

---

## [1.0.10] — 2026-02-18

### Changed

- **License changed from Apache-2.0 to AGPL-3.0** — Prevents proprietary forks and closes the network services loophole. All validators running modified code must publish their source. Aligned with blockchain industry standard (Uniswap v3, Aave v3, Lido).
- All SPDX headers updated to `AGPL-3.0-only`.
- All README badges, CONTRIBUTING.md, and SECURITY.md updated.
- **Release workflows converted from Testnet to Mainnet** — Both Flutter Wallet and Validator release pipelines now build with mainnet tags and production release settings.

### Added

- **Smart Contract Developer Guide** (`docs/SMART_CONTRACTS.md`) — Complete guide for writing, compiling, deploying, and interacting with WASM contracts on UVM. Includes SDK reference, USP-01 token standard, DEX AMM, security guidelines, and gas limits.
- **Code of Conduct** (`CODE_OF_CONDUCT.md`) — Contributor Covenant v2.1.
- **Linux desktop entries** — XDG `.desktop` files and icon install rules for both Flutter Wallet and Validator on Linux.
- **App launcher icons** — LOS hexagon logo applied to macOS, Windows, Linux, and Web for both Flutter apps.

---

## [1.0.9] — 2025-06-17

### Mainnet Launch

The first production release of the Unauthority blockchain, running on the live Tor network with 4 bootstrap validators.

### Added

- **Mainnet genesis** with 8 accounts and 21,936,236 LOS total supply.
- **4 bootstrap validators** operating as Tor Hidden Services (.onion).
- **aBFT consensus** with asynchronous Byzantine Fault Tolerance.
- **Block-lattice (DAG)** architecture for parallel transaction processing.
- **Post-quantum cryptography** using Dilithium5 for all signing operations.
- **SHA-3 (NIST FIPS 202)** for all hashing operations.
- **USP-01 token standard** for native fungible tokens and wrapped assets.
- **DEX AMM smart contracts** via WASM Virtual Machine (UVM).
- **46 REST API endpoints** covering accounts, blocks, transactions, validators, contracts, tokens, and DEX.
- **gRPC API** on port `REST + 20,000` for high-performance integrations.
- **Validator reward system**: 500,000 LOS non-inflationary pool, 5,000 LOS/epoch with halving every 48 epochs.
- **Linear voting** (1 LOS = 1 vote) for Sybil-neutral governance.
- **Flat fee model** — BASE_FEE_CIL per transaction.
- **PoW Mining** for fair public token distribution (~96.5% supply).
- **Price feed** support for DEX smart contracts.
- **Flutter Wallet** app (macOS) for sending and receiving LOS.
- **Flutter Validator Dashboard** (macOS) for node monitoring and management.
- **Tor integration** — all nodes auto-generate .onion addresses on startup.
- **Peer discovery** via bootstrap node list with latency-based selection.
- **RocksDB** persistent storage for blocks, accounts, and state.
- **Comprehensive documentation**: Whitepaper, API Reference, Architecture, Validator Guide, Tor Setup, Exchange Integration.

### Security

- Zero `unwrap()` calls in production code paths.
- Zero floating-point arithmetic in consensus or financial logic.
- Pure linear (integer-only) arithmetic for all reward calculations (Sybil-neutral).
- All arithmetic uses checked/saturating operations to prevent overflow.
- Network isolation: Mainnet and Testnet peers cannot contaminate each other.

---

## [1.0.8] — 2025-06-10

### Testnet Phase

Pre-mainnet testing release deployed on the live Tor network.

### Added

- Full testnet deployment with 4 validators on Tor Hidden Services.
- End-to-end transaction testing over the Tor network.
- Validator registration and staking workflow.
- Cross-node balance verification.
- Node crash recovery testing.
- Epoch reward distribution testing.

### Fixed

- Peer contamination bug where testnet peers could leak into mainnet peer tables.
- Network badge incorrectly showing "testnet" in mainnet builds.
- `/tokens` and `/dex/pools` endpoints returning 404 on empty state.
- Genesis reward pool incorrectly included in circulating supply.

---

## [1.0.7] — 2025-06-01

### Added

- Smart contract compilation pipeline (Rust → WASM).
- DEX AMM contract with constant-product market maker.
- USP-01 token deployment and transfer operations.

### Changed

- Upgraded consensus voting to use linear stake weight (1 LOS = 1 vote).
- Improved Tor circuit management with automatic reconnection.

---

## [1.0.6] — 2025-05-20

### Added

- gRPC API alongside REST endpoints.
- Validator metrics endpoint (`/metrics`).
- Slashing logic for double-signing (100% stake) and downtime (1% stake).
- CLI tool (`los-cli`) for wallet and validator management.

### Fixed

- Block ordering edge case in DAG traversal.
- Duplicate transaction detection across parallel chains.

---

## [1.0.5] — 2025-05-10

### Added

- Flutter Validator Dashboard with real-time node monitoring.
- Flutter Wallet with QR code scanning and transaction history.
- `flutter_rust_bridge` integration for Dilithium5 crypto operations in Dart.
- macOS `.dmg` installer builds for both apps.

### Changed

- Migrated all crypto operations from Dart to Rust via FRB.

---

## [1.0.0] — 2025-04-15

### Initial Release

- Core blockchain engine with block-lattice structure.
- Dilithium5 key generation, signing, and verification.
- SHA-3 block hashing.
- Basic REST API for account and transaction operations.
- Tor Hidden Service auto-generation for validator nodes.
- Genesis configuration with fixed 21,936,236 LOS supply.
- RocksDB storage backend.

---

## Genesis Allocation

| Category | Amount (LOS) |
|---|---|
| Dev Treasury 1 | 428,113 |
| Dev Treasury 2 | 245,710 |
| Dev Treasury 3 | 50,000 |
| Dev Treasury 4 | 50,000 |
| Bootstrap Validators (4 × 1,000) | 4,000 |
| **Dev Total** | **777,823** |
| **Public Allocation** | **21,158,413** |
| **Total Supply** | **21,936,236** |

---

[2.1.0]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v2.1.0
[2.0.2]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v2.0.2
[2.0.1]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v2.0.1
[2.0.0]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v2.0.0
[1.0.13]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.13
[1.0.12]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.12
[1.0.11]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.11
[1.0.10]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.10
[1.0.9]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.9
[1.0.8]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.8
[1.0.7]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.7
[1.0.6]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.6
[1.0.5]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.5
[1.0.0]: https://github.com/monkey-king-code/unauthority-core/releases/tag/v1.0.0
