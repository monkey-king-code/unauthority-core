// SPDX-License-Identifier: AGPL-3.0-only
//! # USP-01 Token Registry
//!
//! Node-level helpers for discovering, querying, and indexing USP-01 tokens
//! deployed as WASM contracts on the Unauthority Virtual Machine (UVM).
//!
//! ## How It Works
//!
//! USP-01 contracts store metadata in standardised state keys:
//! - `usp01:init` = "1" when initialized
//! - `usp01:name`, `usp01:symbol`, `usp01:decimals`, etc.
//!
//! The registry inspects contract state to detect USP-01 compliance without
//! executing WASM bytecode (zero gas cost for discovery).
//!
//! ## Usage
//!
//! ```rust,ignore
//! use los_vm::token_registry;
//!
//! let info = token_registry::query_token_info(&engine, "LOSConABC...");
//! let tokens = token_registry::list_usp01_tokens(&engine);
//! let balance = token_registry::query_token_balance(&engine, "LOSConABC...", "LOSWalice...");
//! ```

use crate::WasmEngine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Summary of a USP-01 token (derived from contract state, no WASM execution).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// Contract address (LOSCon...)
    pub contract: String,
    /// Human-readable name
    pub name: String,
    /// Ticker symbol (max 8 chars)
    pub symbol: String,
    /// Decimal places (0-18)
    pub decimals: u64,
    /// Current total supply in atomic units
    pub total_supply: u128,
    /// Whether this is a wrapped asset
    pub is_wrapped: bool,
    /// Origin chain (e.g. "bitcoin") if wrapped
    pub wrapped_origin: String,
    /// Maximum supply cap (0 = no cap)
    pub max_supply: u128,
    /// Bridge operator address (for wrapped assets)
    pub bridge_operator: String,
    /// Token creator / deployer
    pub owner: String,
}

/// Check if a contract is a USP-01 token by inspecting its state.
///
/// A contract is considered USP-01 compliant if its state contains
/// `usp01:init` = "1" and `usp01:symbol` is non-empty.
pub fn is_usp01_token(state: &BTreeMap<String, String>) -> bool {
    state.get("usp01:init") == Some(&"1".to_string())
        && state.get("usp01:symbol").is_some_and(|v| !v.is_empty())
}

/// Extract USP-01 token info from contract state (no WASM execution).
///
/// Returns `None` if the contract is not a USP-01 token.
pub fn token_info_from_state(
    contract_addr: &str,
    state: &BTreeMap<String, String>,
) -> Option<TokenInfo> {
    if !is_usp01_token(state) {
        return None;
    }

    let name = state.get("usp01:name").cloned().unwrap_or_default();
    let symbol = state.get("usp01:symbol").cloned().unwrap_or_default();

    // Decimals may be stored as u64 LE bytes or as a string
    let decimals = parse_state_u64(state, "usp01:decimals");
    let total_supply = parse_state_u128(state, "usp01:total_supply");
    let max_supply = parse_state_u128(state, "usp01:max_supply");

    let is_wrapped = state.get("usp01:is_wrapped") == Some(&"1".to_string());
    let wrapped_origin = state
        .get("usp01:wrapped_origin")
        .cloned()
        .unwrap_or_default();
    let bridge_operator = state
        .get("usp01:bridge_operator")
        .cloned()
        .unwrap_or_default();
    let owner = state.get("usp01:owner").cloned().unwrap_or_default();

    Some(TokenInfo {
        contract: contract_addr.to_string(),
        name,
        symbol,
        decimals,
        total_supply,
        is_wrapped,
        wrapped_origin,
        max_supply,
        bridge_operator,
        owner,
    })
}

/// Query token info from the WasmEngine by contract address.
///
/// Returns `None` if the contract doesn't exist or isn't USP-01 compliant.
pub fn query_token_info(engine: &WasmEngine, contract_addr: &str) -> Option<TokenInfo> {
    let state = engine.get_contract_state(contract_addr).ok()?;
    token_info_from_state(contract_addr, &state)
}

/// Query a holder's token balance from contract state (no WASM execution).
///
/// Returns 0 if the holder has no balance or the contract isn't found.
pub fn query_token_balance(
    engine: &WasmEngine,
    contract_addr: &str,
    holder: &str,
) -> Result<u128, String> {
    let state = engine.get_contract_state(contract_addr)?;
    if !is_usp01_token(&state) {
        return Err("Contract is not a USP-01 token".to_string());
    }
    let key = format!("bal:{}", holder);
    Ok(parse_state_u128(&state, &key))
}

/// Query an allowance from contract state (no WASM execution).
pub fn query_token_allowance(
    engine: &WasmEngine,
    contract_addr: &str,
    owner: &str,
    spender: &str,
) -> Result<u128, String> {
    let state = engine.get_contract_state(contract_addr)?;
    if !is_usp01_token(&state) {
        return Err("Contract is not a USP-01 token".to_string());
    }
    let key = format!("allow:{}:{}", owner, spender);
    Ok(parse_state_u128(&state, &key))
}

/// List all USP-01 tokens deployed on the engine.
///
/// Scans all contracts and returns info for those that are USP-01 compliant.
/// This is O(n) over all contracts — suitable for reasonable contract counts.
pub fn list_usp01_tokens(engine: &WasmEngine) -> Vec<TokenInfo> {
    let addrs = match engine.list_contracts() {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let mut tokens = Vec::new();
    for addr in &addrs {
        if let Some(info) = query_token_info(engine, addr) {
            tokens.push(info);
        }
    }
    tokens
}

// ─────────────────────────────────────────────────────────────
// INTERNAL HELPERS
// ─────────────────────────────────────────────────────────────

/// Parse a u128 from contract state (decimal string).
///
/// USP-01 contracts store numeric values as decimal strings (not raw LE bytes)
/// to survive the `String::from_utf8_lossy` roundtrip in `Contract.state`.
fn parse_state_u128(state: &BTreeMap<String, String>, key: &str) -> u128 {
    state
        .get(key)
        .and_then(|v| v.parse::<u128>().ok())
        .unwrap_or(0)
}

/// Parse a u64 from contract state (decimal string).
fn parse_state_u64(state: &BTreeMap<String, String>, key: &str) -> u64 {
    state
        .get(key)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usp01_state() -> BTreeMap<String, String> {
        let mut state = BTreeMap::new();
        state.insert("usp01:init".to_string(), "1".to_string());
        state.insert("usp01:name".to_string(), "Test Token".to_string());
        state.insert("usp01:symbol".to_string(), "TST".to_string());
        state.insert("usp01:decimals".to_string(), "8".to_string());
        state.insert(
            "usp01:total_supply".to_string(),
            "100000000000000".to_string(), // 1M with 8 decimals
        );
        state.insert("usp01:is_wrapped".to_string(), "0".to_string());
        state.insert("usp01:wrapped_origin".to_string(), String::new());
        state.insert("usp01:max_supply".to_string(), "0".to_string());
        state.insert("usp01:bridge_operator".to_string(), String::new());
        state.insert(
            "usp01:owner".to_string(),
            "LOSWalice000000000000000000000000000000".to_string(),
        );

        // A balance entry (decimal string)
        state.insert(
            "bal:LOSWalice000000000000000000000000000000".to_string(),
            "100000000000000".to_string(),
        );

        state
    }

    #[test]
    fn test_is_usp01_token() {
        let state = make_usp01_state();
        assert!(is_usp01_token(&state));
    }

    #[test]
    fn test_is_not_usp01_token_empty() {
        let state = BTreeMap::new();
        assert!(!is_usp01_token(&state));
    }

    #[test]
    fn test_is_not_usp01_token_no_init() {
        let mut state = BTreeMap::new();
        state.insert("usp01:symbol".to_string(), "TST".to_string());
        assert!(!is_usp01_token(&state));
    }

    #[test]
    fn test_token_info_from_state() {
        let state = make_usp01_state();
        let info = token_info_from_state("LOSConABC", &state).unwrap();
        assert_eq!(info.name, "Test Token");
        assert_eq!(info.symbol, "TST");
        assert_eq!(info.decimals, 8);
        assert_eq!(info.total_supply, 100_000_000_000_000);
        assert!(!info.is_wrapped);
        assert_eq!(info.owner, "LOSWalice000000000000000000000000000000");
    }

    #[test]
    fn test_token_info_from_non_usp01() {
        let state = BTreeMap::new();
        assert!(token_info_from_state("LOSConXYZ", &state).is_none());
    }

    #[test]
    fn test_parse_state_u128_decimal_string() {
        let mut state = BTreeMap::new();
        state.insert("key".to_string(), "42000000000".to_string());
        assert_eq!(parse_state_u128(&state, "key"), 42_000_000_000);
    }

    #[test]
    fn test_parse_state_u128_large_value() {
        let mut state = BTreeMap::new();
        // 21,936,236 LOS in CIL (21_936_236 * 10^11)
        state.insert("key".to_string(), "2193623600000000000".to_string());
        assert_eq!(parse_state_u128(&state, "key"), 2_193_623_600_000_000_000);
    }

    #[test]
    fn test_parse_state_u128_missing() {
        let state = BTreeMap::new();
        assert_eq!(parse_state_u128(&state, "key"), 0);
    }

    #[test]
    fn test_parse_state_u64_decimal_string() {
        let mut state = BTreeMap::new();
        state.insert("key".to_string(), "18".to_string());
        assert_eq!(parse_state_u64(&state, "key"), 18);
    }

    #[test]
    fn test_list_usp01_tokens_empty() {
        let engine = WasmEngine::new();
        let tokens = list_usp01_tokens(&engine);
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_query_token_balance_no_contract() {
        let engine = WasmEngine::new();
        let result = query_token_balance(&engine, "LOSConXYZ", "LOSWalice");
        assert!(result.is_err());
    }
}
