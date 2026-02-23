# Smart Contract Developer Guide — Unauthority (LOS)

A complete guide to writing, compiling, deploying, and interacting with WASM smart contracts on the Unauthority Virtual Machine (UVM).

---

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [Project Setup](#project-setup)
- [Your First Contract](#your-first-contract)
- [SDK Reference](#sdk-reference)
- [Contract Architecture](#contract-architecture)
- [USP-01 Token Standard](#usp-01-token-standard)
- [DEX AMM Contract](#dex-amm-contract)
- [Deployment](#deployment)
- [Interaction](#interaction)
- [Testing](#testing)
- [Security Guidelines](#security-guidelines)
- [Gas & Limits](#gas--limits)
- [Examples](#examples)

---

## Overview

Unauthority smart contracts are written in Rust, compiled to WebAssembly (`wasm32-unknown-unknown`), and executed by the **UVM** (Unauthority Virtual Machine). The UVM is powered by Wasmer with Cranelift compilation and deterministic gas metering.

| Feature | Detail |
|---|---|
| **Language** | Rust (`#![no_std]`, `#![no_main]`) |
| **Target** | `wasm32-unknown-unknown` |
| **Runtime** | Wasmer 4.x + Cranelift |
| **SDK** | `los-sdk` crate (16 host functions) |
| **State** | Persistent key-value storage (per contract) |
| **Events** | Structured event emission (on-chain log) |
| **Transfers** | Native CIL transfers from contract |
| **Crypto** | Blake3 hashing available in-contract |
| **Addressing** | `LOSCon` + 32 hex chars (deterministic) |
| **Arithmetic** | Integer-only (`u128`/`u64`). **No `f32`/`f64`.** |

---

## Prerequisites

```bash
# 1. Rust toolchain (stable, 2021 edition)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. WASM target
rustup target add wasm32-unknown-unknown

# 3. (Optional) wasm tools for size optimization
cargo install wasm-opt  # Part of binaryen
```

---

## Project Setup

### Option A: Standalone Contract

Create a new crate for your contract:

```bash
cargo new --lib my_contract
cd my_contract
```

**`Cargo.toml`:**
```toml
[package]
name = "my-contract"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
los-sdk = { git = "https://github.com/monkey-king-code/unauthority-core", path = "crates/los-sdk" }

[profile.release]
opt-level = "z"      # Optimize for size
lto = true           # Link-time optimization
strip = true         # Strip debug symbols
codegen-units = 1    # Single codegen unit for better optimization
```

### Option B: In-Tree Contract

Add a new contract to `examples/contracts/`:

```toml
# In examples/contracts/Cargo.toml, add:
[[bin]]
name = "my_contract"
path = "my_contract.rs"

[features]
sdk = ["los-sdk"]
```

---

## Your First Contract

**`src/lib.rs`:**
```rust
#![no_std]
#![no_main]

extern crate alloc;
extern crate los_sdk;

use alloc::format;
use los_sdk::*;

/// Called once when the contract is deployed.
#[no_mangle]
pub extern "C" fn init() -> i32 {
    let owner = caller();
    state::set_str("owner", &owner);

    event::emit("Init", &format!(
        r#"{{"owner":"{}","contract":"{}"}}"#,
        owner, self_address()
    ));

    set_return_str(r#"{"success":true}"#);
    0  // Return 0 = success
}

/// Store a greeting message.
#[no_mangle]
pub extern "C" fn set_greeting() -> i32 {
    let msg = match arg(0) {
        Some(m) if !m.is_empty() => m,
        _ => {
            set_return_str(r#"{"success":false,"msg":"greeting required"}"#);
            return 1;
        }
    };

    state::set_str("greeting", &msg);
    event::emit("GreetingSet", &format!(
        r#"{{"greeting":"{}","by":"{}"}}"#, msg, caller()
    ));
    set_return_str(&format!(r#"{{"success":true,"greeting":"{}"}}"#, msg));
    0
}

/// Read the stored greeting.
#[no_mangle]
pub extern "C" fn get_greeting() -> i32 {
    let greeting = state::get_str("greeting").unwrap_or_default();
    set_return_str(&format!(r#"{{"greeting":"{}"}}"#, greeting));
    0
}
```

### Compile

```bash
cargo build --target wasm32-unknown-unknown --release

# Output: target/wasm32-unknown-unknown/release/my_contract.wasm
```

### Optimize (Optional)

```bash
wasm-opt -Oz -o optimized.wasm target/wasm32-unknown-unknown/release/my_contract.wasm
```

---

## SDK Reference

The `los-sdk` crate provides safe wrappers around 16 UVM host functions.

### State Management (`los_sdk::state`)

| Function | Signature | Description |
|---|---|---|
| `set` | `set(key: &str, value: &[u8])` | Write raw bytes to state |
| `set_str` | `set_str(key: &str, value: &str)` | Write UTF-8 string to state |
| `set_u128` | `set_u128(key: &str, value: u128)` | Write u128 (16-byte LE) |
| `set_u64` | `set_u64(key: &str, value: u64)` | Write u64 (8-byte LE) |
| `get` | `get(key: &str) -> Option<Vec<u8>>` | Read raw bytes from state |
| `get_str` | `get_str(key: &str) -> Option<String>` | Read UTF-8 string |
| `get_u128` | `get_u128(key: &str) -> u128` | Read u128 (0 if missing) |
| `get_u64` | `get_u64(key: &str) -> u64` | Read u64 (0 if missing) |
| `del` | `del(key: &str)` | Delete a key from state |
| `exists` | `exists(key: &str) -> bool` | Check if key exists |

### Events (`los_sdk::event`)

| Function | Signature | Description |
|---|---|---|
| `emit` | `emit(event_type: &str, data_json: &str)` | Emit structured event |

Events are stored on-chain and returned in API responses. Use short type names and JSON data.

### Cryptography (`los_sdk::crypto`)

| Function | Signature | Description |
|---|---|---|
| `blake3` | `blake3(data: &[u8]) -> [u8; 32]` | Compute Blake3 hash |

### Context Functions

| Function | Signature | Description |
|---|---|---|
| `caller()` | `fn caller() -> String` | Caller's LOS address (verified from block signature) |
| `self_address()` | `fn self_address() -> String` | This contract's address (`LOSCon...`) |
| `balance()` | `fn balance() -> u128` | Contract's CIL balance |
| `timestamp()` | `fn timestamp() -> u64` | Current block timestamp (Unix seconds) |
| `arg_count()` | `fn arg_count() -> u32` | Number of arguments passed |
| `arg(idx)` | `fn arg(idx: u32) -> Option<String>` | Get argument by index |

### Transfer

| Function | Signature | Description |
|---|---|---|
| `transfer` | `fn transfer(recipient: &str, amount: u128) -> Result<(), &str>` | Send CIL from contract to address |

### Output

| Function | Signature | Description |
|---|---|---|
| `set_return` | `fn set_return(data: &[u8])` | Set raw return data |
| `set_return_str` | `fn set_return_str(s: &str)` | Set string return data |
| `log` | `fn log(msg: &str)` | Debug log (visible in node logs, not on-chain) |
| `abort` | `fn abort(msg: &str) -> !` | Abort execution, revert all state changes |

---

## Contract Architecture

### Entry Points

Each contract function is an `extern "C"` function with `#[no_mangle]`:

```rust
#[no_mangle]
pub extern "C" fn my_function() -> i32 {
    // Return 0 for success, non-zero for error
    0
}
```

| Convention | Rule |
|---|---|
| **`init()`** | Called once at deployment. Initialize contract state. |
| **Return value** | `0` = success, non-zero = error |
| **Arguments** | Read via `arg(0)`, `arg(1)`, etc. |
| **Return data** | Set via `set_return_str()` — caller receives this |
| **State changes** | Reverted on non-zero return or `abort()` |

### Contract Addressing

Contract addresses are deterministic:

```
address = "LOSCon" + hex(blake3(owner + ":" + nonce + ":" + block_number))[0..32]
```

Example: `LOSCon7a3f9b2e1c4d6e8f0a1b2c3d4e5f6a7b`

### Memory Model

- **Bump allocator** — provided by `los-sdk`, grows WASM linear memory
- **No garbage collection** — memory freed when WASM instance is destroyed
- **Max state value** — 256 KB per key
- **Max arg size** — 64 KB per argument

---

## USP-01 Token Standard

The native fungible token standard on Unauthority. Equivalent to ERC-20 on Ethereum.

### Entry Points

| Function | Args | Description |
|---|---|---|
| `init` | name, symbol, decimals, total_supply | Deploy token |
| `transfer` | to, amount | Transfer tokens |
| `approve` | spender, amount | Approve spender allowance |
| `transfer_from` | from, to, amount | Transfer using allowance |
| `burn` | amount | Burn caller's tokens |
| `balance_of` | address | Query balance |
| `allowance_of` | owner, spender | Query allowance |
| `total_supply` | – | Query total supply |
| `token_info` | – | Query name, symbol, decimals |
| `wrap_mint` | to, amount | Mint wrapped tokens (bridge operator only) |
| `wrap_burn` | amount | Burn wrapped tokens |

### Deploy a Token via CLI

```bash
los-cli token deploy \
  --wallet my_wallet \
  --wasm target/wasm32-unknown-unknown/release/usp01_token.wasm \
  --name "My Token" \
  --symbol "MTK" \
  --decimals 8 \
  --total-supply 1000000
```

### State Keys

| Key Pattern | Value | Description |
|---|---|---|
| `name` | `"My Token"` | Token name |
| `symbol` | `"MTK"` | Token symbol |
| `decimals` | `"8"` | Decimal places |
| `total_supply` | `"100000000000000"` | Total supply (atomic) |
| `owner` | `"LOSX..."` | Token creator |
| `bal:{address}` | `"500000"` | Balance of address |
| `allow:{owner}:{spender}` | `"100000"` | Allowance |

### Events

| Event | Data |
|---|---|
| `USP01:Init` | `{"name","symbol","decimals","total_supply","owner"}` |
| `USP01:Transfer` | `{"from","to","amount"}` |
| `USP01:Approval` | `{"owner","spender","amount"}` |
| `USP01:Burn` | `{"from","amount","new_supply"}` |
| `USP01:WrapMint` | `{"to","amount","new_supply"}` |
| `USP01:WrapBurn` | `{"from","amount","new_supply"}` |

---

## DEX AMM Contract

Constant-product AMM (Automated Market Maker) following the `x · y = k` invariant.

### Entry Points

| Function | Args | Description |
|---|---|---|
| `init` | – | Initialize DEX |
| `create_pool` | token_a, token_b, amount_a, amount_b, fee_bps | Create liquidity pool |
| `add_liquidity` | pool_id, amount_a, amount_b, min_lp | Add liquidity |
| `remove_liquidity` | pool_id, lp_amount, min_a, min_b | Remove liquidity |
| `swap` | pool_id, token_in, amount_in, min_out, deadline | Execute swap |
| `get_pool` | pool_id | Query pool info |
| `quote` | pool_id, token_in, amount_in | Get swap quote |
| `get_position` | pool_id | Get caller's LP position |
| `list_pools` | – | List all pools |

### AMM Formula

```
amount_out = (amount_after_fee × reserve_out) / (reserve_in + amount_after_fee)
```

Where `amount_after_fee = amount_in × (10000 - fee_bps) / 10000`

### LP Token Minting

Initial LP: `isqrt(amount_a × amount_b) - MINIMUM_LIQUIDITY`

Subsequent LP: `min(amount_a × total_lp / reserve_a, amount_b × total_lp / reserve_b)`

### MEV Protection

- **Deadline** — swap reverts if `timestamp > deadline`
- **Slippage** — swap reverts if `amount_out < min_amount_out`
- **Integer math** — 100% `u128`, zero floating-point

---

## Deployment

### Via CLI

```bash
# Deploy a generic contract
los-cli dex deploy --wallet my_wallet --wasm path/to/contract.wasm

# Deploy a USP-01 token
los-cli token deploy --wallet my_wallet --wasm path/to/usp01_token.wasm \
  --name "Token Name" --symbol "TKN" --decimals 11 --total-supply 1000000
```

### Via REST API

```bash
# Deploy contract
curl -X POST http://localhost:3030/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "from": "LOSX...",
    "wasm_hex": "'$(xxd -p contract.wasm | tr -d '\n')'",
    "init_args": ["arg1", "arg2"],
    "signature": "...",
    "public_key": "..."
  }'

# Response
{
  "status": "ok",
  "contract_address": "LOSCon7a3f9b2e1c4d6e8f0a1b2c3d4e5f6a7b",
  "init_result": "{\"success\":true}"
}
```

---

## Interaction

### Call a Contract Function

```bash
# Via CLI
los-cli dex swap \
  --wallet my_wallet \
  --contract LOSCon... \
  --pool-id 0 \
  --token-in LOSConTokenA... \
  --amount-in 10000 \
  --min-out 4800

# Via REST API
curl -X POST http://localhost:3030/call-contract \
  -H "Content-Type: application/json" \
  -d '{
    "from": "LOSX...",
    "contract_address": "LOSCon...",
    "function": "swap",
    "args": ["0", "LOSConTokenA...", "10000", "4800", "1771280000"],
    "signature": "...",
    "public_key": "..."
  }'
```

### Read-Only Calls

Read-only functions (no state mutation) can be called without a signature:

```bash
curl -X POST http://localhost:3030/call-contract \
  -d '{
    "contract_address": "LOSCon...",
    "function": "get_pool",
    "args": ["0"]
  }'
```

---

## Testing

### Native Unit Tests

Test contract logic on native targets (not WASM):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_price_calculation() {
        let reserve_a: u128 = 1_000_000;
        let reserve_b: u128 = 500_000;
        let amount_in: u128 = 10_000;
        let fee_bps: u128 = 30;

        let after_fee = amount_in * (10000 - fee_bps) / 10000;
        let amount_out = (after_fee * reserve_b) / (reserve_a + after_fee);
        assert_eq!(amount_out, 4950);
    }
}
```

```bash
cargo test
```

### Integration Testing

Deploy and test on a local testnet node:

```bash
# Start a local testnet node
./target/release/los-node --port 3030 --data-dir /tmp/los-test

# Deploy your contract
los-cli dex deploy --wallet test_wallet --wasm my_contract.wasm --rpc http://localhost:3030

# Call functions
los-cli dex pool --contract LOSCon... --pool-id 0 --rpc http://localhost:3030
```

---

## Security Guidelines

### Mandatory Rules

1. **No floating-point** — Use `u128`/`u64` integer arithmetic only. Floating-point is non-deterministic across platforms.
2. **Checked arithmetic** — Always use checked operations or verify overflow manually. The UVM does not catch integer overflow.
3. **Access control** — Verify `caller()` before privileged operations.
4. **Input validation** — Never trust `arg()` values. Validate all inputs.
5. **No `unwrap()` in production** — Handle `Option`/`Result` explicitly or use `abort()`.
6. **Reentrancy** — `transfer()` does not call arbitrary code, but always update state BEFORE transfers.

### Best Practices

```rust
// GOOD: Check authorization
#[no_mangle]
pub extern "C" fn admin_function() -> i32 {
    let owner = state::get_str("owner").unwrap_or_default();
    if caller() != owner {
        set_return_str(r#"{"success":false,"msg":"unauthorized"}"#);
        return 1;
    }
    // ... privileged logic
    0
}

// GOOD: Checked arithmetic
let total = match amount_a.checked_mul(amount_b) {
    Some(v) => v,
    None => {
        set_return_str(r#"{"success":false,"msg":"overflow"}"#);
        return 1;
    }
};

// GOOD: State-before-transfer (CEI pattern)
state::set_u128("balance", new_balance);  // State change first
transfer(recipient, amount)?;              // External call second
```

### Common Pitfalls

| Pitfall | Solution |
|---|---|
| Floating-point arithmetic | Use `u128` with scaled integers (e.g., 10^11 for LOS) |
| Missing access control on `init()` | Store `caller()` as owner in `init()`, check in admin functions |
| Unbounded loops over state | Maintain explicit counters, limit iteration |
| State key collisions | Use namespaced keys: `bal:{addr}`, `pool:{id}:reserve_a` |
| Integer division rounding | Round in favor of the protocol (round down outputs) |

---

## Gas & Limits

| Resource | Limit |
|---|---|
| **Gas per execution** | 100,000,000 (Cranelift metered) |
| **Max WASM binary** | 1 MB |
| **Max state value** | 256 KB per key |
| **Max argument** | 64 KB per arg |
| **Max events per call** | 100 |
| **Max transfers per call** | 10 |
| **Memory pages** | Initial 4 pages (256 KB), growable |

---

## Examples

The `examples/contracts/` directory contains production-quality contract examples:

| Contract | Description | SDK |
|---|---|---|
| `simple_storage.rs` | Full SDK demo: state, events, transfers, blake3 | `los-sdk` |
| `hello_world.rs` | Basic key-value storage (legacy, uses `std`) | None |
| `token.rs` | Reference token implementation (legacy) | None |
| `oracle_price_feed.rs` | Price oracle contract (legacy) | None |
| `dex_amm.rs` | DEX example (legacy) | None |

The production contracts in `crates/los-contracts/` use the `los-sdk`:

| Contract | Description |
|---|---|
| `usp01_token.rs` | Production USP-01 token standard (~400 lines) |
| `dex_amm.rs` | Production DEX AMM (~600 lines) |

### Build All Examples

```bash
# Legacy examples (use std)
cargo build --release -p los-contract-examples

# SDK examples (WASM target)
cargo build --target wasm32-unknown-unknown --release -p los-contract-examples --features sdk

# Production contracts
cargo build --target wasm32-unknown-unknown --release -p los-contracts
```

---

## Further Reading

- [API Reference](API_REFERENCE.md) — Contract deployment & call endpoints
- [Architecture](ARCHITECTURE.md) — UVM internals, host function pipeline
- [Exchange Integration](EXCHANGE_INTEGRATION.md) — Token & DEX RPC for integrators
- [Whitepaper](WHITEPAPER.md) — UVM design rationale

---

## License

AGPL-3.0 — See [LICENSE](../LICENSE)
