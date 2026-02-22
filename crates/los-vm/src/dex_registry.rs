//! # DEX Registry — Node-level DEX pool discovery and query helpers
//!
//! Reads DEX contract state directly from `Contract.state` without invoking WASM.
//! Used by REST API endpoints to serve pool info, quotes, and positions efficiently.
//!
//! ## State Layout (decimal strings)
//! - `dex:init`                    → "1"
//! - `dex:pool_count`              → Number of pools
//! - `pool:{id}:token_a`           → Token A address
//! - `pool:{id}:token_b`           → Token B address
//! - `pool:{id}:reserve_a`         → Reserve A
//! - `pool:{id}:reserve_b`         → Reserve B
//! - `pool:{id}:total_lp`          → Total LP tokens
//! - `pool:{id}:fee_bps`           → Fee in bps
//! - `pool:{id}:creator`           → Pool creator address
//! - `pool:{id}:last_trade`        → Last trade timestamp
//! - `pool_list:{index}`           → Pool ID at index
//! - `lp:{pool_id}:{address}`     → LP shares for user

use crate::WasmEngine;
use serde::Serialize;
use std::collections::BTreeMap;

/// Pool info extracted from contract state.
#[derive(Debug, Clone, Serialize)]
pub struct PoolInfo {
    /// Contract address hosting this DEX
    pub contract: String,
    /// Pool ID (e.g. "POOL:tokenA:tokenB")
    pub pool_id: String,
    /// Token A address or "LOS"
    pub token_a: String,
    /// Token B address or "LOS"
    pub token_b: String,
    /// Reserve of token A (atomic units)
    pub reserve_a: u128,
    /// Reserve of token B (atomic units)
    pub reserve_b: u128,
    /// Total LP tokens issued
    pub total_lp: u128,
    /// Fee in basis points (30 = 0.3%)
    pub fee_bps: u64,
    /// Pool creator address
    pub creator: String,
    /// Timestamp of last trade
    pub last_trade: u64,
}

/// Check if a contract's state represents an initialized DEX.
pub fn is_dex_contract(state: &BTreeMap<String, String>) -> bool {
    state.get("dex:init") == Some(&"1".to_string())
        && state
            .get("dex:pool_count")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
}

/// Parse a u128 from contract state (stored as decimal string).
fn parse_state_u128(state: &BTreeMap<String, String>, key: &str) -> u128 {
    state
        .get(key)
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(0)
}

/// Parse a u64 from contract state (stored as decimal string).
fn parse_state_u64(state: &BTreeMap<String, String>, key: &str) -> u64 {
    state
        .get(key)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Extract a single pool's info from DEX contract state.
fn pool_info_from_state(
    contract: &str,
    pool_id: &str,
    state: &BTreeMap<String, String>,
) -> Option<PoolInfo> {
    let prefix = format!("pool:{}", pool_id);
    let token_a = state.get(&format!("{}:token_a", prefix))?.clone();
    if token_a.is_empty() {
        return None;
    }
    let token_b = state
        .get(&format!("{}:token_b", prefix))
        .cloned()
        .unwrap_or_default();
    let reserve_a = parse_state_u128(state, &format!("{}:reserve_a", prefix));
    let reserve_b = parse_state_u128(state, &format!("{}:reserve_b", prefix));
    let total_lp = parse_state_u128(state, &format!("{}:total_lp", prefix));
    let fee_bps = parse_state_u64(state, &format!("{}:fee_bps", prefix));
    let creator = state
        .get(&format!("{}:creator", prefix))
        .cloned()
        .unwrap_or_default();
    let last_trade = parse_state_u64(state, &format!("{}:last_trade", prefix));

    Some(PoolInfo {
        contract: contract.to_string(),
        pool_id: pool_id.to_string(),
        token_a,
        token_b,
        reserve_a,
        reserve_b,
        total_lp,
        fee_bps,
        creator,
        last_trade,
    })
}

/// List all pools in a DEX contract.
fn list_pools_from_state(contract: &str, state: &BTreeMap<String, String>) -> Vec<PoolInfo> {
    let count = parse_state_u64(state, "dex:pool_count");
    let mut pools = Vec::new();
    for i in 0..count {
        let key = format!("pool_list:{}", i);
        if let Some(pool_id) = state.get(&key) {
            if !pool_id.is_empty() {
                if let Some(info) = pool_info_from_state(contract, pool_id, state) {
                    pools.push(info);
                }
            }
        }
    }
    pools
}

/// Query pool info by pool_id from a specific DEX contract.
pub fn query_pool_info(
    engine: &WasmEngine,
    contract_addr: &str,
    pool_id: &str,
) -> Option<PoolInfo> {
    let contracts = engine.contracts.lock().ok()?;
    let contract = contracts.get(contract_addr)?;
    if !is_dex_contract(&contract.state) {
        return None;
    }
    pool_info_from_state(contract_addr, pool_id, &contract.state)
}

/// Query LP position for a user in a pool.
pub fn query_lp_position(
    engine: &WasmEngine,
    contract_addr: &str,
    pool_id: &str,
    user: &str,
) -> Result<u128, String> {
    let contracts = engine
        .contracts
        .lock()
        .map_err(|_| "Lock error".to_string())?;
    let contract = contracts
        .get(contract_addr)
        .ok_or_else(|| "Contract not found".to_string())?;
    if !is_dex_contract(&contract.state) {
        return Err("Not a DEX contract".to_string());
    }
    let key = format!("lp:{}:{}", pool_id, user);
    Ok(parse_state_u128(&contract.state, &key))
}

/// List all DEX contracts and their pools across the entire engine.
pub fn list_all_dex_pools(engine: &WasmEngine) -> Vec<PoolInfo> {
    let contracts = match engine.contracts.lock() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut all_pools = Vec::new();
    for (addr, contract) in contracts.iter() {
        if is_dex_contract(&contract.state) {
            let pools = list_pools_from_state(addr, &contract.state);
            all_pools.extend(pools);
        }
    }
    all_pools
}

/// Compute a swap quote without executing (pure calculation).
/// Returns `(amount_out, fee, price_impact_bps)`.
pub fn compute_quote(
    engine: &WasmEngine,
    contract_addr: &str,
    pool_id: &str,
    token_in: &str,
    amount_in: u128,
) -> Result<(u128, u128, u128), String> {
    let contracts = engine
        .contracts
        .lock()
        .map_err(|_| "Lock error".to_string())?;
    let contract = contracts
        .get(contract_addr)
        .ok_or_else(|| "Contract not found".to_string())?;
    if !is_dex_contract(&contract.state) {
        return Err("Not a DEX contract".to_string());
    }

    let prefix = format!("pool:{}", pool_id);
    let pool_token_a = contract
        .state
        .get(&format!("{}:token_a", prefix))
        .cloned()
        .unwrap_or_default();
    if pool_token_a.is_empty() {
        return Err("Pool not found".to_string());
    }
    let _pool_token_b = contract
        .state
        .get(&format!("{}:token_b", prefix))
        .cloned()
        .unwrap_or_default();
    let reserve_a = parse_state_u128(&contract.state, &format!("{}:reserve_a", prefix));
    let reserve_b = parse_state_u128(&contract.state, &format!("{}:reserve_b", prefix));
    let fee_bps = parse_state_u128(&contract.state, &format!("{}:fee_bps", prefix));

    let is_a_to_b = token_in == pool_token_a;
    let (reserve_in, reserve_out) = if is_a_to_b {
        (reserve_a, reserve_b)
    } else {
        (reserve_b, reserve_a)
    };

    let fee = amount_in * fee_bps / 10_000;
    let after_fee = amount_in - fee;

    // Constant product: amount_out = (after_fee * reserve_out) / (reserve_in + after_fee)
    let amount_out = if reserve_in > 0 && reserve_out > 0 && after_fee > 0 {
        match (
            after_fee.checked_mul(reserve_out),
            reserve_in.checked_add(after_fee),
        ) {
            (Some(num), Some(den)) if den > 0 => num / den,
            _ => {
                let precision: u128 = 1_000_000_000_000;
                let ratio = (after_fee * precision) / reserve_in.saturating_add(after_fee);
                (ratio * reserve_out) / precision
            }
        }
    } else {
        0
    };

    // Price impact
    let precision: u128 = 1_000_000_000_000;
    let spot = if reserve_in > 0 {
        reserve_out * precision / reserve_in
    } else {
        0
    };
    let exec = if amount_in > 0 {
        amount_out * precision / amount_in
    } else {
        0
    };
    let impact_bps = if spot > 0 && spot > exec {
        ((spot - exec) * 10_000) / spot
    } else {
        0
    };

    Ok((amount_out, fee, impact_bps))
}

// ─────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dex_state() -> BTreeMap<String, String> {
        let mut s = BTreeMap::new();
        s.insert("dex:init".into(), "1".into());
        s.insert("dex:pool_count".into(), "1".into());
        s.insert("pool_list:0".into(), "POOL:LOS:TOKEN_A".into());
        s.insert("pool:POOL:LOS:TOKEN_A:token_a".into(), "LOS".into());
        s.insert("pool:POOL:LOS:TOKEN_A:token_b".into(), "TOKEN_A".into());
        s.insert(
            "pool:POOL:LOS:TOKEN_A:reserve_a".into(),
            "1000000000".into(),
        );
        s.insert(
            "pool:POOL:LOS:TOKEN_A:reserve_b".into(),
            "2000000000".into(),
        );
        s.insert("pool:POOL:LOS:TOKEN_A:total_lp".into(), "1414213562".into());
        s.insert("pool:POOL:LOS:TOKEN_A:fee_bps".into(), "30".into());
        s.insert("pool:POOL:LOS:TOKEN_A:creator".into(), "LOSWalice".into());
        s.insert("pool:POOL:LOS:TOKEN_A:last_trade".into(), "0".into());
        s.insert("lp:POOL:LOS:TOKEN_A:LOSWalice".into(), "1414212562".into());
        s
    }

    #[test]
    fn test_is_dex_contract() {
        let s = make_dex_state();
        assert!(is_dex_contract(&s));
    }

    #[test]
    fn test_is_not_dex_empty() {
        let s = BTreeMap::new();
        assert!(!is_dex_contract(&s));
    }

    #[test]
    fn test_is_not_dex_no_init() {
        let mut s = BTreeMap::new();
        s.insert("dex:pool_count".into(), "1".into());
        assert!(!is_dex_contract(&s));
    }

    #[test]
    fn test_pool_info_from_state() {
        let s = make_dex_state();
        let info = pool_info_from_state("LOSCon123", "POOL:LOS:TOKEN_A", &s).unwrap();
        assert_eq!(info.token_a, "LOS");
        assert_eq!(info.token_b, "TOKEN_A");
        assert_eq!(info.reserve_a, 1_000_000_000);
        assert_eq!(info.reserve_b, 2_000_000_000);
        assert_eq!(info.total_lp, 1_414_213_562);
        assert_eq!(info.fee_bps, 30);
        assert_eq!(info.creator, "LOSWalice");
    }

    #[test]
    fn test_pool_info_missing() {
        let s = make_dex_state();
        assert!(pool_info_from_state("LOSCon123", "POOL:NONEXISTENT", &s).is_none());
    }

    #[test]
    fn test_list_pools_from_state() {
        let s = make_dex_state();
        let pools = list_pools_from_state("LOSCon123", &s);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].pool_id, "POOL:LOS:TOKEN_A");
    }

    #[test]
    fn test_list_pools_empty() {
        let mut s = BTreeMap::new();
        s.insert("dex:init".into(), "1".into());
        s.insert("dex:pool_count".into(), "0".into());
        let pools = list_pools_from_state("LOSCon123", &s);
        assert!(pools.is_empty());
    }

    #[test]
    fn test_parse_state_u128_decimal() {
        let mut s = BTreeMap::new();
        s.insert("key".into(), "42000000000".into());
        assert_eq!(parse_state_u128(&s, "key"), 42_000_000_000);
    }

    #[test]
    fn test_parse_state_u128_missing() {
        let s = BTreeMap::new();
        assert_eq!(parse_state_u128(&s, "key"), 0);
    }

    #[test]
    fn test_parse_state_u64_decimal() {
        let mut s = BTreeMap::new();
        s.insert("key".into(), "30".into());
        assert_eq!(parse_state_u64(&s, "key"), 30);
    }

    #[test]
    fn test_lp_position_from_state() {
        let s = make_dex_state();
        let lp_key = "lp:POOL:LOS:TOKEN_A:LOSWalice";
        assert_eq!(parse_state_u128(&s, lp_key), 1_414_212_562);
    }

    #[test]
    fn test_list_all_dex_pools_empty() {
        let engine = WasmEngine::new();
        let pools = list_all_dex_pools(&engine);
        assert!(pools.is_empty());
    }
}
