// SPDX-License-Identifier: AGPL-3.0-only
//! # DEX AMM Contract (WASM)
//!
//! Deployable `#![no_std]` WASM smart contract implementing a Constant Product
//! AMM (x·y=k) for the Unauthority (LOS) blockchain.
//!
//! ## Features
//! - Permissionless pool creation for any USP-01 token pair (or native LOS)
//! - Constant Product formula (x·y=k) — all integer math (u128)
//! - 0.3% swap fee (30 bps) distributed to liquidity providers
//! - MEV Protection: max slippage + deadline enforcement
//! - LP token tracking (proportional share)
//! - No floating-point arithmetic — consensus-safe
//!
//! ## State Layout
//! - `dex:init`                              → "1" when initialized
//! - `dex:owner`                             → DEX deployer address
//! - `dex:pool_count`                        → Number of pools (decimal string)
//! - `pool:{id}:token_a`                     → Token A address (or "LOS")
//! - `pool:{id}:token_b`                     → Token B address (or "LOS")
//! - `pool:{id}:reserve_a`                   → Reserve A (decimal string)
//! - `pool:{id}:reserve_b`                   → Reserve B (decimal string)
//! - `pool:{id}:total_lp`                    → Total LP tokens (decimal string)
//! - `pool:{id}:fee_bps`                     → Fee in bps (decimal string)
//! - `pool:{id}:creator`                     → Pool creator
//! - `pool:{id}:last_trade`                  → Last trade timestamp
//! - `lp:{pool_id}:{address}`               → LP shares for user (decimal string)
//! - `pool_list:{index}`                     → Pool ID at index (for enumeration)
//!
//! ## Exported Functions
//! | Function           | Args                                                     |
//! |--------------------|----------------------------------------------------------|
//! | `init`             | (none — initializes DEX)                                 |
//! | `create_pool`      | token_a, token_b, amount_a, amount_b [, fee_bps]         |
//! | `add_liquidity`    | pool_id, amount_a, amount_b, min_lp_tokens               |
//! | `remove_liquidity` | pool_id, lp_amount, min_amount_a, min_amount_b           |
//! | `swap`             | pool_id, token_in, amount_in, min_amount_out, deadline   |
//! | `get_pool`         | pool_id                                                  |
//! | `quote`            | pool_id, token_in, amount_in                             |
//! | `get_position`     | pool_id                                                  |
//! | `list_pools`       | (none)                                                   |
//!
//! ## Compilation
//! ```bash
//! cargo build --target wasm32-unknown-unknown --release \
//!     -p los-contract-examples --bin dex_amm_wasm --features sdk
//! ```

#![no_std]
#![no_main]

extern crate alloc;
extern crate los_sdk;

use alloc::format;
use alloc::string::String;
use los_sdk::*;

// ─────────────────────────────────────────────────────────────
// CONSTANTS (integer-only, no f32/f64)
// ─────────────────────────────────────────────────────────────

/// Default swap fee: 30 bps = 0.3%
const DEFAULT_FEE_BPS: u128 = 30;
/// Basis point denominator
const BPS_DENOMINATOR: u128 = 10_000;
/// Minimum liquidity locked forever (prevent price manipulation)
const MINIMUM_LIQUIDITY: u128 = 1_000;
/// Max fee: 1000 bps = 10%
const MAX_FEE_BPS: u128 = 1_000;
/// Precision multiplier for overflow-safe calculations
const PRECISION: u128 = 1_000_000_000_000;

// ─────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────

/// Parse a decimal string to u128. Returns 0 on failure.
fn parse_u128(s: &str) -> u128 {
    let mut result: u128 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            result = match result.checked_mul(10) {
                Some(v) => v,
                None => return 0,
            };
            result = match result.checked_add((*b - b'0') as u128) {
                Some(v) => v,
                None => return 0,
            };
        } else {
            return 0;
        }
    }
    result
}

/// Convert u128 to decimal string without std.
fn u128_to_str(val: u128) -> String {
    if val == 0 {
        return String::from("0");
    }
    let mut buf = [0u8; 40];
    let mut pos = buf.len();
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let bytes = &buf[pos..];
    // SAFETY: we only write ASCII digits
    unsafe { String::from_utf8_unchecked(alloc::vec::Vec::from(bytes)) }
}

/// Parse u64 from decimal string.
fn parse_u64(s: &str) -> u64 {
    let mut result: u64 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            result = match result.checked_mul(10) {
                Some(v) => v,
                None => return 0,
            };
            result = match result.checked_add((*b - b'0') as u64) {
                Some(v) => v,
                None => return 0,
            };
        } else {
            return 0;
        }
    }
    result
}

/// Escape a string for JSON output.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Return failure JSON.
fn fail(msg: &str) -> i32 {
    let resp = format!(
        "{{\"success\":false,\"message\":\"{}\"}}",
        json_escape(msg)
    );
    set_return_str(&resp);
    1
}

/// Return success JSON.
fn ok(msg: &str) -> i32 {
    let resp = format!(
        "{{\"success\":true,\"message\":\"{}\"}}",
        json_escape(msg)
    );
    set_return_str(&resp);
    0
}

/// Return success JSON with data.
fn ok_data(msg: &str, data: &str) -> i32 {
    let resp = format!(
        "{{\"success\":true,\"message\":\"{}\",\"data\":{}}}",
        json_escape(msg),
        data
    );
    set_return_str(&resp);
    0
}

// ─────────────────────────────────────────────────────────────
// INTEGER MATH (NO f32/f64)
// ─────────────────────────────────────────────────────────────

/// Integer square root — Newton's method. Returns floor(√n).
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2); // safe: no overflow for u128::MAX
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Compute swap output using constant product formula.
/// `amount_out = (amount_in * reserve_out) / (reserve_in + amount_in)`
fn compute_output(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return 0;
    }
    match (
        amount_in.checked_mul(reserve_out),
        reserve_in.checked_add(amount_in),
    ) {
        (Some(num), Some(den)) if den > 0 => num / den,
        _ => {
            // Overflow fallback: scaled division
            let ratio_scaled =
                (amount_in as u128 * PRECISION) / reserve_in.saturating_add(amount_in);
            (ratio_scaled * reserve_out) / PRECISION
        }
    }
}

/// Deduct fee from input amount. Returns (after_fee, fee).
fn deduct_fee(amount: u128, fee_bps: u128) -> (u128, u128) {
    let fee = amount * fee_bps / BPS_DENOMINATOR;
    (amount - fee, fee)
}

// ─────────────────────────────────────────────────────────────
// STATE HELPERS
// ─────────────────────────────────────────────────────────────

fn get_state_str(key: &str) -> String {
    state::get_str(key).unwrap_or_default()
}

fn get_state_u128(key: &str) -> u128 {
    parse_u128(&get_state_str(key))
}

fn get_state_u64(key: &str) -> u64 {
    parse_u64(&get_state_str(key))
}

fn set_state_u128(key: &str, val: u128) {
    state::set_str(key, &u128_to_str(val));
}

fn set_state_u64(key: &str, val: u64) {
    state::set_str(key, &u128_to_str(val as u128));
}

/// Generate deterministic pool ID from token pair (sorted).
fn make_pool_id(token_a: &str, token_b: &str) -> String {
    if token_a < token_b {
        format!("POOL:{}:{}", token_a, token_b)
    } else {
        format!("POOL:{}:{}", token_b, token_a)
    }
}

/// Check if a pool exists.
fn pool_exists(pool_id: &str) -> bool {
    let key = format!("pool:{}:token_a", pool_id);
    !get_state_str(&key).is_empty()
}

// ─────────────────────────────────────────────────────────────
// EXPORTED FUNCTIONS
// ─────────────────────────────────────────────────────────────

/// Initialize the DEX contract.
#[no_mangle]
pub extern "C" fn init() -> i32 {
    if get_state_str("dex:init") == "1" {
        return fail("DEX already initialized");
    }
    let who = caller();
    state::set_str("dex:init", "1");
    state::set_str("dex:owner", &who);
    set_state_u64("dex:pool_count", 0);

    event::emit("DexInit", &format!("{{\"owner\":\"{}\"}}", json_escape(&who)));
    ok("DEX initialized")
}

/// Create a new liquidity pool for a token pair.
/// Args: token_a, token_b, amount_a, amount_b [, fee_bps]
#[no_mangle]
pub extern "C" fn create_pool() -> i32 {
    if get_state_str("dex:init") != "1" {
        return fail("DEX not initialized");
    }

    let token_a = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing token_a"),
    };
    let token_b = match arg(1) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing token_b"),
    };
    let amount_a = match arg(2) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing amount_a"),
    };
    let amount_b = match arg(3) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing amount_b"),
    };
    let fee_bps = match arg(4) {
        Some(v) if !v.is_empty() => parse_u128(&v),
        _ => DEFAULT_FEE_BPS,
    };

    // Validation
    if token_a == token_b {
        return fail("Cannot create pool with identical tokens");
    }
    if amount_a == 0 || amount_b == 0 {
        return fail("Initial liquidity must be > 0 for both tokens");
    }
    if fee_bps > MAX_FEE_BPS {
        return fail("Fee too high (max 1000 bps = 10%)");
    }

    let pool_id = make_pool_id(&token_a, &token_b);
    if pool_exists(&pool_id) {
        return fail(&format!("Pool {} already exists", pool_id));
    }

    // Initial LP = sqrt(amount_a * amount_b) - MINIMUM_LIQUIDITY
    let product = match amount_a.checked_mul(amount_b) {
        Some(p) => p,
        None => return fail("Overflow: amounts too large"),
    };
    let initial_lp = isqrt(product);
    if initial_lp <= MINIMUM_LIQUIDITY {
        return fail("Initial liquidity too small");
    }
    let lp_tokens = initial_lp - MINIMUM_LIQUIDITY;

    let who = caller();

    // Store pool state
    let prefix = format!("pool:{}", pool_id);
    state::set_str(&format!("{}:token_a", prefix), &token_a);
    state::set_str(&format!("{}:token_b", prefix), &token_b);
    set_state_u128(&format!("{}:reserve_a", prefix), amount_a);
    set_state_u128(&format!("{}:reserve_b", prefix), amount_b);
    set_state_u128(&format!("{}:total_lp", prefix), initial_lp);
    set_state_u128(&format!("{}:fee_bps", prefix), fee_bps);
    state::set_str(&format!("{}:creator", prefix), &who);
    set_state_u64(&format!("{}:last_trade", prefix), 0);

    // Store LP shares for creator
    let lp_key = format!("lp:{}:{}", pool_id, who);
    set_state_u128(&lp_key, lp_tokens);

    // Add to pool index
    let count = get_state_u64("dex:pool_count");
    state::set_str(&format!("pool_list:{}", count), &pool_id);
    set_state_u64("dex:pool_count", count + 1);

    event::emit(
        "PoolCreated",
        &format!(
            "{{\"pool_id\":\"{}\",\"token_a\":\"{}\",\"token_b\":\"{}\",\"reserve_a\":\"{}\",\"reserve_b\":\"{}\",\"lp_tokens\":\"{}\"}}",
            json_escape(&pool_id),
            json_escape(&token_a),
            json_escape(&token_b),
            u128_to_str(amount_a),
            u128_to_str(amount_b),
            u128_to_str(lp_tokens),
        ),
    );

    ok_data(
        &format!("Pool {} created", pool_id),
        &format!(
            "{{\"pool_id\":\"{}\",\"lp_tokens\":\"{}\",\"reserve_a\":\"{}\",\"reserve_b\":\"{}\"}}",
            json_escape(&pool_id),
            u128_to_str(lp_tokens),
            u128_to_str(amount_a),
            u128_to_str(amount_b),
        ),
    )
}

/// Add liquidity to an existing pool.
/// Args: pool_id, amount_a, amount_b, min_lp_tokens
#[no_mangle]
pub extern "C" fn add_liquidity() -> i32 {
    if get_state_str("dex:init") != "1" {
        return fail("DEX not initialized");
    }

    let pool_id = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing pool_id"),
    };
    let amount_a = match arg(1) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing amount_a"),
    };
    let amount_b = match arg(2) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing amount_b"),
    };
    let min_lp_tokens = match arg(3) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing min_lp_tokens"),
    };

    if !pool_exists(&pool_id) {
        return fail("Pool not found");
    }
    if amount_a == 0 || amount_b == 0 {
        return fail("Amounts must be > 0");
    }

    let prefix = format!("pool:{}", pool_id);
    let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
    let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
    let total_lp = get_state_u128(&format!("{}:total_lp", prefix));

    if reserve_a == 0 || reserve_b == 0 || total_lp == 0 {
        return fail("Pool has no liquidity");
    }

    // LP = min(amount_a * total_lp / reserve_a, amount_b * total_lp / reserve_b)
    let lp_from_a = amount_a * total_lp / reserve_a;
    let lp_from_b = amount_b * total_lp / reserve_b;
    let lp_tokens = if lp_from_a < lp_from_b {
        lp_from_a
    } else {
        lp_from_b
    };

    if lp_tokens < min_lp_tokens {
        return fail(&format!(
            "Slippage: would mint {} LP but minimum is {}",
            u128_to_str(lp_tokens),
            u128_to_str(min_lp_tokens)
        ));
    }

    // Calculate actual amounts used (proportional)
    let actual_a = lp_tokens * reserve_a / total_lp;
    let actual_b = lp_tokens * reserve_b / total_lp;

    // Update reserves
    set_state_u128(&format!("{}:reserve_a", prefix), reserve_a + actual_a);
    set_state_u128(&format!("{}:reserve_b", prefix), reserve_b + actual_b);
    set_state_u128(&format!("{}:total_lp", prefix), total_lp + lp_tokens);

    // Update LP shares
    let who = caller();
    let lp_key = format!("lp:{}:{}", pool_id, who);
    let existing_lp = get_state_u128(&lp_key);
    set_state_u128(&lp_key, existing_lp + lp_tokens);

    event::emit(
        "LiquidityAdded",
        &format!(
            "{{\"pool_id\":\"{}\",\"provider\":\"{}\",\"amount_a\":\"{}\",\"amount_b\":\"{}\",\"lp_tokens\":\"{}\"}}",
            json_escape(&pool_id),
            json_escape(&who),
            u128_to_str(actual_a),
            u128_to_str(actual_b),
            u128_to_str(lp_tokens),
        ),
    );

    ok_data(
        &format!("Added liquidity: {} LP tokens minted", u128_to_str(lp_tokens)),
        &format!(
            "{{\"lp_tokens\":\"{}\",\"amount_a_used\":\"{}\",\"amount_b_used\":\"{}\"}}",
            u128_to_str(lp_tokens),
            u128_to_str(actual_a),
            u128_to_str(actual_b),
        ),
    )
}

/// Remove liquidity from a pool.
/// Args: pool_id, lp_amount, min_amount_a, min_amount_b
#[no_mangle]
pub extern "C" fn remove_liquidity() -> i32 {
    if get_state_str("dex:init") != "1" {
        return fail("DEX not initialized");
    }

    let pool_id = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing pool_id"),
    };
    let lp_amount = match arg(1) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing lp_amount"),
    };
    let min_amount_a = match arg(2) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing min_amount_a"),
    };
    let min_amount_b = match arg(3) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing min_amount_b"),
    };

    if !pool_exists(&pool_id) {
        return fail("Pool not found");
    }

    let who = caller();
    let lp_key = format!("lp:{}:{}", pool_id, who);
    let caller_lp = get_state_u128(&lp_key);
    if caller_lp < lp_amount {
        return fail(&format!(
            "Insufficient LP tokens: have {} need {}",
            u128_to_str(caller_lp),
            u128_to_str(lp_amount)
        ));
    }

    let prefix = format!("pool:{}", pool_id);
    let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
    let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
    let total_lp = get_state_u128(&format!("{}:total_lp", prefix));

    if total_lp == 0 {
        return fail("Pool has no liquidity");
    }

    // Proportional token amounts
    let amount_a = lp_amount * reserve_a / total_lp;
    let amount_b = lp_amount * reserve_b / total_lp;

    // Slippage protection
    if amount_a < min_amount_a || amount_b < min_amount_b {
        return fail(&format!(
            "Slippage: would receive ({}, {}) but minimum is ({}, {})",
            u128_to_str(amount_a),
            u128_to_str(amount_b),
            u128_to_str(min_amount_a),
            u128_to_str(min_amount_b),
        ));
    }

    // Update reserves
    set_state_u128(&format!("{}:reserve_a", prefix), reserve_a - amount_a);
    set_state_u128(&format!("{}:reserve_b", prefix), reserve_b - amount_b);
    set_state_u128(&format!("{}:total_lp", prefix), total_lp - lp_amount);

    // Update LP shares
    let new_lp = caller_lp - lp_amount;
    if new_lp == 0 {
        state::del(&lp_key);
    } else {
        set_state_u128(&lp_key, new_lp);
    }

    event::emit(
        "LiquidityRemoved",
        &format!(
            "{{\"pool_id\":\"{}\",\"provider\":\"{}\",\"amount_a\":\"{}\",\"amount_b\":\"{}\",\"lp_burned\":\"{}\"}}",
            json_escape(&pool_id),
            json_escape(&who),
            u128_to_str(amount_a),
            u128_to_str(amount_b),
            u128_to_str(lp_amount),
        ),
    );

    ok_data(
        &format!(
            "Removed liquidity: {} LP tokens burned",
            u128_to_str(lp_amount)
        ),
        &format!(
            "{{\"amount_a\":\"{}\",\"amount_b\":\"{}\",\"lp_burned\":\"{}\"}}",
            u128_to_str(amount_a),
            u128_to_str(amount_b),
            u128_to_str(lp_amount),
        ),
    )
}

/// Swap tokens via constant product AMM.
/// Args: pool_id, token_in, amount_in, min_amount_out, deadline
#[no_mangle]
pub extern "C" fn swap() -> i32 {
    if get_state_str("dex:init") != "1" {
        return fail("DEX not initialized");
    }

    let pool_id = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing pool_id"),
    };
    let token_in = match arg(1) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing token_in"),
    };
    let amount_in = match arg(2) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing amount_in"),
    };
    let min_amount_out = match arg(3) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing min_amount_out"),
    };
    let deadline = match arg(4) {
        Some(v) => parse_u64(&v),
        None => return fail("Missing deadline"),
    };

    if !pool_exists(&pool_id) {
        return fail("Pool not found");
    }
    if amount_in == 0 {
        return fail("Amount must be > 0");
    }

    // MEV Protection: deadline check
    let now = timestamp();
    if deadline > 0 && now > deadline {
        return fail(&format!(
            "Transaction expired: deadline {} < current {}",
            deadline, now
        ));
    }

    let prefix = format!("pool:{}", pool_id);
    let pool_token_a = get_state_str(&format!("{}:token_a", prefix));
    let pool_token_b = get_state_str(&format!("{}:token_b", prefix));
    let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
    let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
    let fee_bps = get_state_u128(&format!("{}:fee_bps", prefix));

    // Determine swap direction
    let is_a_to_b = token_in == pool_token_a;
    let is_b_to_a = token_in == pool_token_b;
    if !is_a_to_b && !is_b_to_a {
        return fail(&format!(
            "Token {} is not in pool (expected {} or {})",
            token_in, pool_token_a, pool_token_b
        ));
    }

    let (reserve_in, reserve_out, token_out) = if is_a_to_b {
        (reserve_a, reserve_b, pool_token_b.clone())
    } else {
        (reserve_b, reserve_a, pool_token_a.clone())
    };

    // Deduct fee
    let (amount_after_fee, fee) = deduct_fee(amount_in, fee_bps);

    // Constant product output
    let amount_out = compute_output(amount_after_fee, reserve_in, reserve_out);

    // MEV Protection: slippage check
    if amount_out < min_amount_out {
        return fail(&format!(
            "Slippage exceeded: output {} < minimum {}",
            u128_to_str(amount_out),
            u128_to_str(min_amount_out)
        ));
    }
    if amount_out == 0 {
        return fail("Output amount is zero (insufficient liquidity)");
    }
    if amount_out >= reserve_out {
        return fail("Insufficient liquidity for this trade");
    }

    // Update reserves — fee stays in pool for LPs
    if is_a_to_b {
        set_state_u128(&format!("{}:reserve_a", prefix), reserve_a + amount_in);
        set_state_u128(&format!("{}:reserve_b", prefix), reserve_b - amount_out);
    } else {
        set_state_u128(&format!("{}:reserve_b", prefix), reserve_b + amount_in);
        set_state_u128(&format!("{}:reserve_a", prefix), reserve_a - amount_out);
    }

    set_state_u64(&format!("{}:last_trade", prefix), now);

    let who = caller();
    event::emit(
        "Swap",
        &format!(
            "{{\"pool_id\":\"{}\",\"trader\":\"{}\",\"token_in\":\"{}\",\"amount_in\":\"{}\",\"token_out\":\"{}\",\"amount_out\":\"{}\",\"fee\":\"{}\"}}",
            json_escape(&pool_id),
            json_escape(&who),
            json_escape(&token_in),
            u128_to_str(amount_in),
            json_escape(&token_out),
            u128_to_str(amount_out),
            u128_to_str(fee),
        ),
    );

    // Price impact (bps)
    let impact_bps = if reserve_out > 0 {
        (amount_out * BPS_DENOMINATOR) / reserve_out
    } else {
        0
    };

    ok_data(
        &format!(
            "Swapped {} {} -> {} {}",
            u128_to_str(amount_in),
            token_in,
            u128_to_str(amount_out),
            token_out
        ),
        &format!(
            "{{\"amount_out\":\"{}\",\"fee\":\"{}\",\"price_impact_bps\":\"{}\"}}",
            u128_to_str(amount_out),
            u128_to_str(fee),
            u128_to_str(impact_bps),
        ),
    )
}

/// Get pool info (read-only).
/// Args: pool_id
#[no_mangle]
pub extern "C" fn get_pool() -> i32 {
    let pool_id = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing pool_id"),
    };

    if !pool_exists(&pool_id) {
        return fail("Pool not found");
    }

    let prefix = format!("pool:{}", pool_id);
    let token_a = get_state_str(&format!("{}:token_a", prefix));
    let token_b = get_state_str(&format!("{}:token_b", prefix));
    let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
    let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
    let total_lp = get_state_u128(&format!("{}:total_lp", prefix));
    let fee_bps = get_state_u128(&format!("{}:fee_bps", prefix));
    let creator = get_state_str(&format!("{}:creator", prefix));
    let last_trade = get_state_u64(&format!("{}:last_trade", prefix));

    // Spot price: price_b = reserve_a * PRECISION / reserve_b (A per B)
    let spot_price_scaled = if reserve_b > 0 {
        reserve_a * PRECISION / reserve_b
    } else {
        0
    };

    ok_data(
        "Pool found",
        &format!(
            "{{\"pool_id\":\"{}\",\"token_a\":\"{}\",\"token_b\":\"{}\",\"reserve_a\":\"{}\",\"reserve_b\":\"{}\",\"total_lp\":\"{}\",\"fee_bps\":\"{}\",\"creator\":\"{}\",\"last_trade\":\"{}\",\"spot_price_scaled\":\"{}\"}}",
            json_escape(&pool_id),
            json_escape(&token_a),
            json_escape(&token_b),
            u128_to_str(reserve_a),
            u128_to_str(reserve_b),
            u128_to_str(total_lp),
            u128_to_str(fee_bps),
            json_escape(&creator),
            last_trade,
            u128_to_str(spot_price_scaled),
        ),
    )
}

/// Get a swap quote without executing (read-only).
/// Args: pool_id, token_in, amount_in
#[no_mangle]
pub extern "C" fn quote() -> i32 {
    let pool_id = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing pool_id"),
    };
    let token_in = match arg(1) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing token_in"),
    };
    let amount_in = match arg(2) {
        Some(v) => parse_u128(&v),
        None => return fail("Missing amount_in"),
    };

    if !pool_exists(&pool_id) {
        return fail("Pool not found");
    }

    let prefix = format!("pool:{}", pool_id);
    let pool_token_a = get_state_str(&format!("{}:token_a", prefix));
    let pool_token_b = get_state_str(&format!("{}:token_b", prefix));
    let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
    let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
    let fee_bps = get_state_u128(&format!("{}:fee_bps", prefix));

    let is_a_to_b = token_in == pool_token_a;
    let (reserve_in, reserve_out) = if is_a_to_b {
        (reserve_a, reserve_b)
    } else {
        (reserve_b, reserve_a)
    };

    let (after_fee, fee) = deduct_fee(amount_in, fee_bps);
    let amount_out = compute_output(after_fee, reserve_in, reserve_out);

    // Price impact
    let spot_price_scaled = if reserve_in > 0 {
        reserve_out * PRECISION / reserve_in
    } else {
        0
    };
    let exec_price_scaled = if amount_in > 0 {
        amount_out * PRECISION / amount_in
    } else {
        0
    };
    let impact_bps = if spot_price_scaled > 0 && spot_price_scaled > exec_price_scaled {
        ((spot_price_scaled - exec_price_scaled) * BPS_DENOMINATOR) / spot_price_scaled
    } else {
        0
    };

    ok_data(
        &format!(
            "Quote: {} in -> {} out",
            u128_to_str(amount_in),
            u128_to_str(amount_out)
        ),
        &format!(
            "{{\"amount_out\":\"{}\",\"fee\":\"{}\",\"price_impact_bps\":\"{}\",\"spot_price_scaled\":\"{}\"}}",
            u128_to_str(amount_out),
            u128_to_str(fee),
            u128_to_str(impact_bps),
            u128_to_str(spot_price_scaled),
        ),
    )
}

/// Get caller's LP position in a pool (read-only).
/// Args: pool_id
#[no_mangle]
pub extern "C" fn get_position() -> i32 {
    let pool_id = match arg(0) {
        Some(v) if !v.is_empty() => v,
        _ => return fail("Missing pool_id"),
    };

    if !pool_exists(&pool_id) {
        return fail("Pool not found");
    }

    let who = caller();
    let lp_key = format!("lp:{}:{}", pool_id, who);
    let shares = get_state_u128(&lp_key);

    let prefix = format!("pool:{}", pool_id);
    let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
    let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
    let total_lp = get_state_u128(&format!("{}:total_lp", prefix));

    let (amount_a, amount_b) = if total_lp > 0 && shares > 0 {
        (shares * reserve_a / total_lp, shares * reserve_b / total_lp)
    } else {
        (0, 0)
    };

    let share_pct_bps = if total_lp > 0 {
        (shares * BPS_DENOMINATOR) / total_lp
    } else {
        0
    };

    ok_data(
        "Position found",
        &format!(
            "{{\"lp_shares\":\"{}\",\"total_lp\":\"{}\",\"amount_a\":\"{}\",\"amount_b\":\"{}\",\"share_pct_bps\":\"{}\"}}",
            u128_to_str(shares),
            u128_to_str(total_lp),
            u128_to_str(amount_a),
            u128_to_str(amount_b),
            u128_to_str(share_pct_bps),
        ),
    )
}

/// List all pools (read-only).
#[no_mangle]
pub extern "C" fn list_pools() -> i32 {
    let count = get_state_u64("dex:pool_count");
    if count == 0 {
        return ok_data("0 pools", "[]");
    }

    let mut data = String::from("[");
    for i in 0..count {
        let pid = get_state_str(&format!("pool_list:{}", i));
        if pid.is_empty() {
            continue;
        }
        if i > 0 {
            data.push(',');
        }

        let prefix = format!("pool:{}", pid);
        let token_a = get_state_str(&format!("{}:token_a", prefix));
        let token_b = get_state_str(&format!("{}:token_b", prefix));
        let reserve_a = get_state_u128(&format!("{}:reserve_a", prefix));
        let reserve_b = get_state_u128(&format!("{}:reserve_b", prefix));
        let total_lp = get_state_u128(&format!("{}:total_lp", prefix));

        data.push_str(&format!(
            "{{\"pool_id\":\"{}\",\"token_a\":\"{}\",\"token_b\":\"{}\",\"reserve_a\":\"{}\",\"reserve_b\":\"{}\",\"total_lp\":\"{}\"}}",
            json_escape(&pid),
            json_escape(&token_a),
            json_escape(&token_b),
            u128_to_str(reserve_a),
            u128_to_str(reserve_b),
            u128_to_str(total_lp),
        ));
    }
    data.push(']');

    ok_data(&format!("{} pools", count), &data)
}
