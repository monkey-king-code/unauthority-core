// SPDX-License-Identifier: AGPL-3.0-only
//! # USP-01 Token Contract
//!
//! Deployable WASM smart contract implementing the USP-01 Fungible Token Standard
//! for the Unauthority (LOS) blockchain.
//!
//! ## Features
//! - Fixed or capped supply at deployment time
//! - Transfer, Approve, TransferFrom (ERC-20 equivalent)
//! - Burn (permanent supply reduction)
//! - Wrapped Asset support (wBTC, wETH, etc.)
//! - All amounts in atomic units (`u128`) — NO floating-point
//! - Standard event emission for indexing
//!
//! ## State Layout
//! - `usp01:init`                → "1" when initialized
//! - `usp01:name`                → Token name (e.g. "Wrapped Bitcoin")
//! - `usp01:symbol`              → Ticker symbol (e.g. "wBTC")
//! - `usp01:decimals`            → Decimal places (u64 LE bytes)
//! - `usp01:total_supply`        → Total supply (u128 LE bytes)
//! - `usp01:is_wrapped`          → "1" or "0"
//! - `usp01:wrapped_origin`      → Origin chain (e.g. "bitcoin")
//! - `usp01:max_supply`          → Max supply cap (u128 LE bytes, 0=no cap)
//! - `usp01:bridge_operator`     → Bridge operator address
//! - `usp01:owner`               → Token creator address
//! - `bal:{address}`             → Balance (u128 LE bytes)
//! - `allow:{owner}:{spender}`   → Allowance (u128 LE bytes)
//!
//! ## Exported Functions
//! | Function         | Args                                              |
//! |------------------|----------------------------------------------------|
//! | `init`           | name, symbol, decimals, total_supply,              |
//! |                  | [is_wrapped], [wrapped_origin], [max_supply],      |
//! |                  | [bridge_operator]                                  |
//! | `transfer`       | to, amount                                         |
//! | `approve`        | spender, amount                                    |
//! | `transfer_from`  | from, to, amount                                   |
//! | `burn`           | amount                                             |
//! | `balance_of`     | account                                            |
//! | `allowance_of`   | owner, spender                                     |
//! | `total_supply`   | (none)                                             |
//! | `token_info`     | (none)                                             |
//! | `wrap_mint`      | to, amount, proof                                  |
//! | `wrap_burn`      | amount, destination                                |
//!
//! ## Compilation
//! ```bash
//! cargo build --target wasm32-unknown-unknown --release \
//!     -p los-contract-examples --bin usp01_token --features sdk
//! ```

#![no_std]
#![no_main]

extern crate alloc;
extern crate los_sdk;

use alloc::format;
use alloc::string::String;
use los_sdk::*;

// ─────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────

/// Parse a u128 from a decimal string. Returns 0 on failure.
fn parse_u128(s: &str) -> u128 {
    let mut result: u128 = 0;
    for b in s.as_bytes() {
        if *b < b'0' || *b > b'9' {
            return 0;
        }
        let digit = (*b - b'0') as u128;
        result = match result.checked_mul(10) {
            Some(v) => v,
            None => return 0,
        };
        result = match result.checked_add(digit) {
            Some(v) => v,
            None => return 0,
        };
    }
    result
}

/// Return a u128 as decimal string.
fn u128_to_str(val: u128) -> String {
    if val == 0 {
        return String::from("0");
    }
    let mut digits = alloc::vec::Vec::new();
    let mut v = val;
    while v > 0 {
        digits.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    digits.reverse();
    // Safety: all bytes are ASCII digits
    unsafe { String::from_utf8_unchecked(digits) }
}

/// Build balance state key.
fn bal_key(addr: &str) -> String {
    format!("bal:{}", addr)
}

/// Build allowance state key.
fn allow_key(owner: &str, spender: &str) -> String {
    format!("allow:{}:{}", owner, spender)
}

/// Get balance for an address (stored as decimal string).
fn get_balance(addr: &str) -> u128 {
    parse_u128(&state::get_str(&bal_key(addr)).unwrap_or_default())
}

/// Set balance for an address (stored as decimal string).
fn set_balance(addr: &str, amount: u128) {
    state::set_str(&bal_key(addr), &u128_to_str(amount));
}

/// Get allowance for (owner, spender) (stored as decimal string).
fn get_allowance(owner: &str, spender: &str) -> u128 {
    parse_u128(&state::get_str(&allow_key(owner, spender)).unwrap_or_default())
}

/// Set allowance for (owner, spender) (stored as decimal string).
fn set_allowance(owner: &str, spender: &str, amount: u128) {
    state::set_str(&allow_key(owner, spender), &u128_to_str(amount));
}

/// Get total supply from state (stored as decimal string).
fn get_total_supply() -> u128 {
    parse_u128(&state::get_str("usp01:total_supply").unwrap_or_default())
}

/// Set total supply in state (stored as decimal string).
fn set_total_supply(val: u128) {
    state::set_str("usp01:total_supply", &u128_to_str(val));
}

/// Check if contract is initialized.
fn is_initialized() -> bool {
    state::get_str("usp01:init").map_or(false, |v| v == "1")
}

/// Fail with JSON error response.
fn fail(msg: &str) -> i32 {
    set_return_str(&format!(r#"{{"success":false,"msg":"{}"}}"#, msg));
    1
}

/// Succeed with JSON success response.
fn ok(msg: &str) -> i32 {
    set_return_str(&format!(r#"{{"success":true,"msg":"{}"}}"#, msg));
    0
}

/// Succeed with JSON data response.
fn ok_data(data: &str) -> i32 {
    set_return_str(&format!(r#"{{"success":true,"data":{}}}"#, data));
    0
}

/// Escape a string for JSON (minimal — handles quotes and backslashes).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────
// INIT — Called once at deployment
// ─────────────────────────────────────────────────────────────

/// Initialize a new USP-01 token.
///
/// Args:
///   0: name (string, 1-64 chars)
///   1: symbol (string, 1-8 chars)
///   2: decimals (u8, 0-18)
///   3: total_supply (u128 decimal string)
///   4: is_wrapped ("1" or "0", optional, default "0")
///   5: wrapped_origin (string, optional)
///   6: max_supply (u128 decimal string, optional, default "0")
///   7: bridge_operator (address, optional)
#[no_mangle]
pub extern "C" fn init() -> i32 {
    if is_initialized() {
        return fail("Already initialized");
    }

    // Parse required args
    let name = match arg(0) {
        Some(n) if !n.is_empty() && n.len() <= 64 => n,
        _ => return fail("name required (1-64 chars)"),
    };
    let symbol = match arg(1) {
        Some(s) if !s.is_empty() && s.len() <= 8 => s,
        _ => return fail("symbol required (1-8 chars)"),
    };
    let decimals_str = arg(2).unwrap_or_default();
    let decimals = match decimals_str.parse::<u64>() {
        Ok(d) if d <= 18 => d,
        _ => return fail("decimals must be 0-18"),
    };
    let total_supply_str = arg(3).unwrap_or_default();
    let total_supply = parse_u128(&total_supply_str);
    if total_supply == 0 {
        return fail("total_supply must be > 0");
    }

    // Parse optional args
    let is_wrapped = arg(4).unwrap_or_default() == "1";
    let wrapped_origin = arg(5).unwrap_or_default();
    let max_supply_str = arg(6).unwrap_or_default();
    let max_supply = if max_supply_str.is_empty() {
        0u128
    } else {
        parse_u128(&max_supply_str)
    };
    let bridge_operator = arg(7).unwrap_or_default();

    // Validate
    if max_supply > 0 && total_supply > max_supply {
        return fail("total_supply exceeds max_supply");
    }
    if is_wrapped && wrapped_origin.is_empty() {
        return fail("wrapped tokens must specify wrapped_origin");
    }
    if is_wrapped && bridge_operator.is_empty() {
        return fail("wrapped tokens must specify bridge_operator");
    }

    let creator = caller();
    if creator.is_empty() {
        return fail("caller address not available");
    }

    // Store metadata
    state::set_str("usp01:init", "1");
    state::set_str("usp01:name", &name);
    state::set_str("usp01:symbol", &symbol);
    state::set_str("usp01:decimals", &format!("{}", decimals));
    set_total_supply(total_supply);
    state::set_str("usp01:is_wrapped", if is_wrapped { "1" } else { "0" });
    state::set_str("usp01:wrapped_origin", &wrapped_origin);
    state::set_str("usp01:max_supply", &u128_to_str(max_supply));
    state::set_str("usp01:bridge_operator", &bridge_operator);
    state::set_str("usp01:owner", &creator);

    // Assign total supply to creator
    set_balance(&creator, total_supply);

    // Emit init event
    event::emit(
        "USP01:Init",
        &format!(
            r#"{{"name":"{}","symbol":"{}","decimals":{},"total_supply":"{}","creator":"{}"}}"#,
            json_escape(&name),
            json_escape(&symbol),
            decimals,
            u128_to_str(total_supply),
            json_escape(&creator)
        ),
    );

    log(&format!(
        "USP-01 token initialized: {} ({}) supply={}",
        name,
        symbol,
        u128_to_str(total_supply)
    ));

    set_return_str(&format!(
        r#"{{"success":true,"name":"{}","symbol":"{}","total_supply":"{}","owner":"{}","contract":"{}"}}"#,
        json_escape(&name),
        json_escape(&symbol),
        u128_to_str(total_supply),
        json_escape(&creator),
        json_escape(&self_address())
    ));
    0
}

// ─────────────────────────────────────────────────────────────
// TRANSFER — Send tokens from caller to recipient
// ─────────────────────────────────────────────────────────────

/// Transfer tokens from caller to recipient.
///
/// Args:
///   0: to (recipient address)
///   1: amount (u128 decimal string)
#[no_mangle]
pub extern "C" fn transfer() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let to = match arg(0) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("recipient address required"),
    };
    let amount_str = arg(1).unwrap_or_default();
    let amount = parse_u128(&amount_str);
    if amount == 0 {
        return fail("amount must be > 0");
    }

    let from = caller();
    if from.is_empty() {
        return fail("caller address not available");
    }
    if from == to {
        return fail("cannot transfer to self");
    }

    let from_bal = get_balance(&from);
    if from_bal < amount {
        return fail("insufficient balance");
    }

    // Debit sender (checked_sub for defense-in-depth)
    let new_from = match from_bal.checked_sub(amount) {
        Some(v) => v,
        None => return fail("arithmetic underflow"),
    };
    set_balance(&from, new_from);

    // Credit recipient (checked_add prevents u128 overflow)
    let to_bal = get_balance(&to);
    let new_to = match to_bal.checked_add(amount) {
        Some(v) => v,
        None => return fail("arithmetic overflow"),
    };
    set_balance(&to, new_to);

    // Emit transfer event
    event::emit(
        "USP01:Transfer",
        &format!(
            r#"{{"from":"{}","to":"{}","amount":"{}"}}"#,
            json_escape(&from),
            json_escape(&to),
            u128_to_str(amount)
        ),
    );

    set_return_str(&format!(
        r#"{{"success":true,"from":"{}","to":"{}","amount":"{}"}}"#,
        json_escape(&from),
        json_escape(&to),
        u128_to_str(amount)
    ));
    0
}

// ─────────────────────────────────────────────────────────────
// APPROVE — Set spending allowance
// ─────────────────────────────────────────────────────────────

/// Approve spender to spend up to `amount` on behalf of caller.
///
/// Args:
///   0: spender (address)
///   1: amount (u128 decimal string, 0 to revoke)
#[no_mangle]
pub extern "C" fn approve() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let spender = match arg(0) {
        Some(s) if !s.is_empty() => s,
        _ => return fail("spender address required"),
    };
    let amount_str = arg(1).unwrap_or_default();
    let amount = parse_u128(&amount_str);

    let owner = caller();
    if owner.is_empty() {
        return fail("caller address not available");
    }
    if owner == spender {
        return fail("cannot approve self");
    }

    set_allowance(&owner, &spender, amount);

    // Emit approval event
    event::emit(
        "USP01:Approval",
        &format!(
            r#"{{"owner":"{}","spender":"{}","amount":"{}"}}"#,
            json_escape(&owner),
            json_escape(&spender),
            u128_to_str(amount)
        ),
    );

    set_return_str(&format!(
        r#"{{"success":true,"owner":"{}","spender":"{}","amount":"{}"}}"#,
        json_escape(&owner),
        json_escape(&spender),
        u128_to_str(amount)
    ));
    0
}

// ─────────────────────────────────────────────────────────────
// TRANSFER_FROM — Spend tokens on behalf of owner (requires allowance)
// ─────────────────────────────────────────────────────────────

/// Transfer tokens from `from` to `to` using caller's allowance.
///
/// Args:
///   0: from (owner address)
///   1: to (recipient address)
///   2: amount (u128 decimal string)
#[no_mangle]
pub extern "C" fn transfer_from() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let from = match arg(0) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("from address required"),
    };
    let to = match arg(1) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("to address required"),
    };
    let amount_str = arg(2).unwrap_or_default();
    let amount = parse_u128(&amount_str);
    if amount == 0 {
        return fail("amount must be > 0");
    }
    if from == to {
        return fail("from and to must differ");
    }

    let spender = caller();
    if spender.is_empty() {
        return fail("caller address not available");
    }

    // Check allowance
    let allowance = get_allowance(&from, &spender);
    if allowance < amount {
        return fail("allowance exceeded");
    }

    // Check balance
    let from_bal = get_balance(&from);
    if from_bal < amount {
        return fail("insufficient balance");
    }

    // Debit owner
    let new_from = match from_bal.checked_sub(amount) {
        Some(v) => v,
        None => return fail("arithmetic underflow"),
    };
    set_balance(&from, new_from);

    // Credit recipient
    let to_bal = get_balance(&to);
    let new_to = match to_bal.checked_add(amount) {
        Some(v) => v,
        None => return fail("arithmetic overflow"),
    };
    set_balance(&to, new_to);

    // Reduce allowance
    let new_allowance = match allowance.checked_sub(amount) {
        Some(v) => v,
        None => 0,
    };
    set_allowance(&from, &spender, new_allowance);

    // Emit transfer event
    event::emit(
        "USP01:Transfer",
        &format!(
            r#"{{"from":"{}","to":"{}","amount":"{}","spender":"{}"}}"#,
            json_escape(&from),
            json_escape(&to),
            u128_to_str(amount),
            json_escape(&spender)
        ),
    );

    set_return_str(&format!(
        r#"{{"success":true,"from":"{}","to":"{}","amount":"{}","spender":"{}"}}"#,
        json_escape(&from),
        json_escape(&to),
        u128_to_str(amount),
        json_escape(&spender)
    ));
    0
}

// ─────────────────────────────────────────────────────────────
// BURN — Permanently destroy tokens
// ─────────────────────────────────────────────────────────────

/// Burn tokens from caller's balance, reducing total supply permanently.
///
/// Args:
///   0: amount (u128 decimal string)
#[no_mangle]
pub extern "C" fn burn() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let amount_str = arg(0).unwrap_or_default();
    let amount = parse_u128(&amount_str);
    if amount == 0 {
        return fail("amount must be > 0");
    }

    let from = caller();
    if from.is_empty() {
        return fail("caller address not available");
    }

    let bal = get_balance(&from);
    if bal < amount {
        return fail("insufficient balance to burn");
    }

    // Debit caller
    let new_bal = match bal.checked_sub(amount) {
        Some(v) => v,
        None => return fail("arithmetic underflow"),
    };
    set_balance(&from, new_bal);

    // Reduce total supply permanently
    let supply = get_total_supply();
    let new_supply = supply.saturating_sub(amount);
    set_total_supply(new_supply);

    // Emit burn event
    event::emit(
        "USP01:Burn",
        &format!(
            r#"{{"from":"{}","amount":"{}","new_supply":"{}"}}"#,
            json_escape(&from),
            u128_to_str(amount),
            u128_to_str(new_supply)
        ),
    );

    set_return_str(&format!(
        r#"{{"success":true,"burned":"{}","new_supply":"{}"}}"#,
        u128_to_str(amount),
        u128_to_str(new_supply)
    ));
    0
}

// ─────────────────────────────────────────────────────────────
// BALANCE_OF — Query balance (read-only)
// ─────────────────────────────────────────────────────────────

/// Return balance of an account.
///
/// Args:
///   0: account (address)
#[no_mangle]
pub extern "C" fn balance_of() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let account = match arg(0) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("account address required"),
    };

    let bal = get_balance(&account);
    ok_data(&format!(
        r#"{{"account":"{}","balance":"{}"}}"#,
        json_escape(&account),
        u128_to_str(bal)
    ))
}

// ─────────────────────────────────────────────────────────────
// ALLOWANCE_OF — Query allowance (read-only)
// ─────────────────────────────────────────────────────────────

/// Return allowance granted by owner to spender.
///
/// Args:
///   0: owner (address)
///   1: spender (address)
#[no_mangle]
pub extern "C" fn allowance_of() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let owner = match arg(0) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("owner address required"),
    };
    let spender = match arg(1) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("spender address required"),
    };

    let allowance = get_allowance(&owner, &spender);
    ok_data(&format!(
        r#"{{"owner":"{}","spender":"{}","allowance":"{}"}}"#,
        json_escape(&owner),
        json_escape(&spender),
        u128_to_str(allowance)
    ))
}

// ─────────────────────────────────────────────────────────────
// TOTAL_SUPPLY — Query total supply (read-only)
// ─────────────────────────────────────────────────────────────

/// Return current total supply.
#[no_mangle]
pub extern "C" fn total_supply() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let supply = get_total_supply();
    ok_data(&format!(r#"{{"total_supply":"{}"}}"#, u128_to_str(supply)))
}

// ─────────────────────────────────────────────────────────────
// TOKEN_INFO — Query full metadata (read-only)
// ─────────────────────────────────────────────────────────────

/// Return complete token metadata.
#[no_mangle]
pub extern "C" fn token_info() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let name = state::get_str("usp01:name").unwrap_or_default();
    let symbol = state::get_str("usp01:symbol").unwrap_or_default();
    let decimals = parse_u128(&state::get_str("usp01:decimals").unwrap_or_default());
    let total_supply = get_total_supply();
    let is_wrapped = state::get_str("usp01:is_wrapped").unwrap_or_default() == "1";
    let wrapped_origin = state::get_str("usp01:wrapped_origin").unwrap_or_default();
    let max_supply = parse_u128(&state::get_str("usp01:max_supply").unwrap_or_default());
    let bridge_operator = state::get_str("usp01:bridge_operator").unwrap_or_default();
    let owner = state::get_str("usp01:owner").unwrap_or_default();
    let contract = self_address();

    ok_data(&format!(
        r#"{{"name":"{}","symbol":"{}","decimals":{},"total_supply":"{}","is_wrapped":{},"wrapped_origin":"{}","max_supply":"{}","bridge_operator":"{}","owner":"{}","contract":"{}","standard":"USP-01"}}"#,
        json_escape(&name),
        json_escape(&symbol),
        decimals,
        u128_to_str(total_supply),
        is_wrapped,
        json_escape(&wrapped_origin),
        u128_to_str(max_supply),
        json_escape(&bridge_operator),
        json_escape(&owner),
        json_escape(&contract)
    ))
}

// ─────────────────────────────────────────────────────────────
// WRAP_MINT — Mint wrapped tokens (bridge operator only)
// ─────────────────────────────────────────────────────────────

/// Mint wrapped tokens when a deposit is confirmed on the source chain.
/// Only callable by the designated bridge operator.
///
/// Args:
///   0: to (recipient address)
///   1: amount (u128 decimal string)
///   2: proof (deposit proof from source chain)
#[no_mangle]
pub extern "C" fn wrap_mint() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let is_wrapped = state::get_str("usp01:is_wrapped").unwrap_or_default() == "1";
    if !is_wrapped {
        return fail("not a wrapped asset");
    }

    let bridge_op = state::get_str("usp01:bridge_operator").unwrap_or_default();
    let who = caller();
    if who != bridge_op {
        return fail("only bridge operator can mint wrapped tokens");
    }

    let to = match arg(0) {
        Some(a) if !a.is_empty() => a,
        _ => return fail("recipient address required"),
    };
    let amount_str = arg(1).unwrap_or_default();
    let amount = parse_u128(&amount_str);
    if amount == 0 {
        return fail("amount must be > 0");
    }
    let proof = match arg(2) {
        Some(p) if !p.is_empty() => p,
        _ => return fail("deposit proof required"),
    };

    // Check max supply cap
    let max_supply = parse_u128(&state::get_str("usp01:max_supply").unwrap_or_default());
    let supply = get_total_supply();
    if max_supply > 0 {
        let new_supply = match supply.checked_add(amount) {
            Some(v) => v,
            None => return fail("supply overflow"),
        };
        if new_supply > max_supply {
            return fail("would exceed max supply cap");
        }
    }

    // Credit recipient
    let to_bal = get_balance(&to);
    let new_to = match to_bal.checked_add(amount) {
        Some(v) => v,
        None => return fail("balance overflow"),
    };
    set_balance(&to, new_to);

    // Increase total supply
    let new_supply = supply.saturating_add(amount);
    set_total_supply(new_supply);

    // Emit event
    event::emit(
        "USP01:WrapMint",
        &format!(
            r#"{{"to":"{}","amount":"{}","proof":"{}","new_supply":"{}"}}"#,
            json_escape(&to),
            u128_to_str(amount),
            json_escape(&proof),
            u128_to_str(new_supply)
        ),
    );

    set_return_str(&format!(
        r#"{{"success":true,"to":"{}","amount":"{}","proof":"{}"}}"#,
        json_escape(&to),
        u128_to_str(amount),
        json_escape(&proof)
    ));
    0
}

// ─────────────────────────────────────────────────────────────
// WRAP_BURN — Burn wrapped tokens for redemption
// ─────────────────────────────────────────────────────────────

/// Burn wrapped tokens for redemption on the source chain.
///
/// Args:
///   0: amount (u128 decimal string)
///   1: destination (address on the source chain)
#[no_mangle]
pub extern "C" fn wrap_burn() -> i32 {
    if !is_initialized() {
        return fail("Contract not initialized");
    }

    let is_wrapped = state::get_str("usp01:is_wrapped").unwrap_or_default() == "1";
    if !is_wrapped {
        return fail("not a wrapped asset");
    }

    let amount_str = arg(0).unwrap_or_default();
    let amount = parse_u128(&amount_str);
    if amount == 0 {
        return fail("amount must be > 0");
    }
    let destination = match arg(1) {
        Some(d) if !d.is_empty() => d,
        _ => return fail("destination address required"),
    };

    let from = caller();
    if from.is_empty() {
        return fail("caller address not available");
    }

    let bal = get_balance(&from);
    if bal < amount {
        return fail("insufficient balance for wrap burn");
    }

    // Debit caller
    let new_bal = match bal.checked_sub(amount) {
        Some(v) => v,
        None => return fail("arithmetic underflow"),
    };
    set_balance(&from, new_bal);

    // Reduce total supply
    let supply = get_total_supply();
    let new_supply = supply.saturating_sub(amount);
    set_total_supply(new_supply);

    // Emit event
    event::emit(
        "USP01:WrapBurn",
        &format!(
            r#"{{"from":"{}","amount":"{}","destination":"{}","new_supply":"{}"}}"#,
            json_escape(&from),
            u128_to_str(amount),
            json_escape(&destination),
            u128_to_str(new_supply)
        ),
    );

    set_return_str(&format!(
        r#"{{"success":true,"from":"{}","amount":"{}","destination":"{}"}}"#,
        json_escape(&from),
        u128_to_str(amount),
        json_escape(&destination)
    ));
    0
}
