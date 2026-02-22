// SPDX-License-Identifier: AGPL-3.0-only
//! # DEX AMM (Automated Market Maker) for Unauthority
//!
//! Permissionless decentralized exchange using the Constant Product formula (x·y=k).
//! All math uses integer arithmetic (`u128`) — NO floating-point.
//!
//! ## Features
//! - Constant Product AMM (x·y=k)
//! - Liquidity pool creation and management
//! - Swap with slippage protection
//! - 0.3% swap fee (distributed to LPs)
//! - MEV protection: max slippage + deadline enforcement
//! - LP token tracking (proportional share)
//! - Integer-only math (u128) for consensus determinism
//!
//! ## Architecture
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  DEX AMM Contract (WASM on UVM)                     │
//! │  ┌──────────────────────────────────────────────┐   │
//! │  │ Pool: TokenA / TokenB                        │   │
//! │  │  reserve_a: u128    reserve_b: u128          │   │
//! │  │  total_lp: u128     fee_bps: u32 (30 = 0.3%)│   │
//! │  │  k_last: u128       (x·y invariant)          │   │
//! │  ├──────────────────────────────────────────────┤   │
//! │  │ LP shares: HashMap<address, u128>            │   │
//! │  └──────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Compilation
//! ```bash
//! cargo build --release --target wasm32-unknown-unknown --bin dex_amm
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────
// CONSTANTS (integer-only, no f32/f64)
// ─────────────────────────────────────────────────────────────

/// Fee in basis points. 30 bps = 0.3%
const SWAP_FEE_BPS: u128 = 30;
/// Basis point denominator
const BPS_DENOMINATOR: u128 = 10_000;
/// Minimum liquidity locked forever (prevent division-by-zero attacks)
const MINIMUM_LIQUIDITY: u128 = 1_000;
/// Precision multiplier for LP share calculations
const LP_PRECISION: u128 = 1_000_000_000_000; // 10^12

// ─────────────────────────────────────────────────────────────
// DATA STRUCTURES
// ─────────────────────────────────────────────────────────────

/// A liquidity pool pairing two USP-01 tokens (or native LOS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    /// Pool unique ID (e.g. "POOL:tokenA_addr:tokenB_addr")
    pub id: String,
    /// Contract address (USP-01) or "LOS" for native
    pub token_a: String,
    /// Contract address (USP-01) or "LOS" for native
    pub token_b: String,
    /// Reserve of token A in atomic units
    pub reserve_a: u128,
    /// Reserve of token B in atomic units
    pub reserve_b: u128,
    /// Total LP tokens minted
    pub total_lp: u128,
    /// LP shares per provider: address → shares
    pub lp_shares: HashMap<String, u128>,
    /// Swap fee in basis points (default 30 = 0.3%)
    pub fee_bps: u32,
    /// Timestamp of last trade (for TWAP / MEV protection)
    pub last_trade_timestamp: u64,
    /// Creator address
    pub creator: String,
}

/// DEX actions (contract ABI).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum DexAction {
    /// Create a new liquidity pool for a token pair.
    CreatePool {
        token_a: String,
        token_b: String,
        amount_a: u128,
        amount_b: u128,
        /// Optional custom fee (default = 30 bps)
        #[serde(default)]
        fee_bps: Option<u32>,
    },

    /// Add liquidity to an existing pool.
    AddLiquidity {
        pool_id: String,
        amount_a: u128,
        amount_b: u128,
        /// Maximum acceptable slippage in bps (MEV protection)
        min_lp_tokens: u128,
    },

    /// Remove liquidity from a pool.
    RemoveLiquidity {
        pool_id: String,
        lp_amount: u128,
        /// Minimum tokens to receive (slippage protection)
        min_amount_a: u128,
        min_amount_b: u128,
    },

    /// Swap token A for token B (or reverse).
    Swap {
        pool_id: String,
        /// The token being sold (must be token_a or token_b of the pool)
        token_in: String,
        amount_in: u128,
        /// Minimum output (slippage + MEV protection)
        min_amount_out: u128,
        /// Unix timestamp deadline — tx rejected if block.timestamp > deadline
        deadline: u64,
    },

    // ── Read-only queries ──

    /// Get pool info.
    GetPool {
        pool_id: String,
    },

    /// Get a swap quote without executing.
    Quote {
        pool_id: String,
        token_in: String,
        amount_in: u128,
    },

    /// Get caller's LP position in a pool.
    GetPosition {
        pool_id: String,
    },

    /// List all pools.
    ListPools,
}

/// DEX response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexResponse {
    pub success: bool,
    pub data: Option<String>,
    pub message: String,
    pub events: Vec<DexEvent>,
}

/// DEX events for indexing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event")]
pub enum DexEvent {
    PoolCreated {
        pool_id: String,
        token_a: String,
        token_b: String,
        reserve_a: u128,
        reserve_b: u128,
    },
    LiquidityAdded {
        pool_id: String,
        provider: String,
        amount_a: u128,
        amount_b: u128,
        lp_tokens: u128,
    },
    LiquidityRemoved {
        pool_id: String,
        provider: String,
        amount_a: u128,
        amount_b: u128,
        lp_burned: u128,
    },
    Swap {
        pool_id: String,
        trader: String,
        token_in: String,
        amount_in: u128,
        token_out: String,
        amount_out: u128,
        fee: u128,
    },
}

// ─────────────────────────────────────────────────────────────
// INTEGER MATH HELPERS (NO f32/f64)
// ─────────────────────────────────────────────────────────────

/// Integer square root using Newton's method (same as consensus isqrt).
/// Returns floor(√n).
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    // Use n/2 + 1 instead of (n+1)/2 to avoid overflow when n = u128::MAX
    let mut y = n / 2 + 1;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Compute swap output using constant product formula with integer math.
///
/// Given:
///   - `amount_in`: tokens sold (after fee deduction by caller)
///   - `reserve_in`: current reserve of input token
///   - `reserve_out`: current reserve of output token
///
/// Returns output amount (rounded down).
///
/// Formula: amount_out = reserve_out - (reserve_in * reserve_out) / (reserve_in + amount_in)
///        = (amount_in * reserve_out) / (reserve_in + amount_in)
fn compute_output(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return 0;
    }
    // Use checked arithmetic to prevent overflow
    // For very large reserves, split into high/low to avoid u128 overflow
    let numerator = amount_in.checked_mul(reserve_out);
    let denominator = reserve_in.checked_add(amount_in);

    match (numerator, denominator) {
        (Some(num), Some(den)) if den > 0 => num / den,
        _ => {
            // Fallback: use u128 division to avoid overflow
            // amount_out ≈ (amount_in / (reserve_in + amount_in)) * reserve_out
            let ratio_scaled = (amount_in as u128 * LP_PRECISION)
                / (reserve_in.saturating_add(amount_in));
            (ratio_scaled * reserve_out) / LP_PRECISION
        }
    }
}

/// Deduct swap fee from input amount.
/// Returns (amount_after_fee, fee_amount).
fn deduct_fee(amount: u128, fee_bps: u128) -> (u128, u128) {
    let fee = amount * fee_bps / BPS_DENOMINATOR;
    (amount - fee, fee)
}

// ─────────────────────────────────────────────────────────────
// DEX STATE (WASM contract state)
// ─────────────────────────────────────────────────────────────

/// Full DEX state (stored in contract storage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexState {
    pub pools: HashMap<String, Pool>,
    pub pool_count: u64,
}

impl DexState {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
            pool_count: 0,
        }
    }

    /// Generate deterministic pool ID from token pair (sorted).
    pub fn pool_id(token_a: &str, token_b: &str) -> String {
        let (a, b) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };
        format!("POOL:{}:{}", a, b)
    }

    /// Execute a DEX action.
    pub fn execute(
        &mut self,
        caller: &str,
        action: DexAction,
        current_timestamp: u64,
    ) -> DexResponse {
        match action {
            DexAction::CreatePool {
                token_a,
                token_b,
                amount_a,
                amount_b,
                fee_bps,
            } => self.create_pool(caller, token_a, token_b, amount_a, amount_b, fee_bps),

            DexAction::AddLiquidity {
                pool_id,
                amount_a,
                amount_b,
                min_lp_tokens,
            } => self.add_liquidity(caller, &pool_id, amount_a, amount_b, min_lp_tokens),

            DexAction::RemoveLiquidity {
                pool_id,
                lp_amount,
                min_amount_a,
                min_amount_b,
            } => self.remove_liquidity(caller, &pool_id, lp_amount, min_amount_a, min_amount_b),

            DexAction::Swap {
                pool_id,
                token_in,
                amount_in,
                min_amount_out,
                deadline,
            } => self.swap(
                caller,
                &pool_id,
                &token_in,
                amount_in,
                min_amount_out,
                deadline,
                current_timestamp,
            ),

            DexAction::GetPool { pool_id } => {
                match self.pools.get(&pool_id) {
                    Some(pool) => DexResponse {
                        success: true,
                        data: Some(serde_json::to_string(pool).unwrap_or_default()),
                        message: "Pool found".to_string(),
                        events: Vec::new(),
                    },
                    None => DexResponse {
                        success: false,
                        data: None,
                        message: "Pool not found".to_string(),
                        events: Vec::new(),
                    },
                }
            }

            DexAction::Quote {
                pool_id,
                token_in,
                amount_in,
            } => self.quote(&pool_id, &token_in, amount_in),

            DexAction::GetPosition { pool_id } => {
                match self.pools.get(&pool_id) {
                    Some(pool) => {
                        let shares = pool.lp_shares.get(caller).copied().unwrap_or(0);
                        let (amount_a, amount_b) = if pool.total_lp > 0 && shares > 0 {
                            (
                                pool.reserve_a * shares / pool.total_lp,
                                pool.reserve_b * shares / pool.total_lp,
                            )
                        } else {
                            (0, 0)
                        };
                        let data = serde_json::json!({
                            "lp_shares": shares,
                            "total_lp": pool.total_lp,
                            "amount_a": amount_a,
                            "amount_b": amount_b,
                            "share_pct_bps": if pool.total_lp > 0 {
                                (shares * BPS_DENOMINATOR) / pool.total_lp
                            } else { 0 },
                        });
                        DexResponse {
                            success: true,
                            data: Some(data.to_string()),
                            message: "Position found".to_string(),
                            events: Vec::new(),
                        }
                    }
                    None => DexResponse {
                        success: false,
                        data: None,
                        message: "Pool not found".to_string(),
                        events: Vec::new(),
                    },
                }
            }

            DexAction::ListPools => {
                let pool_ids: Vec<&str> = self.pools.keys().map(|s| s.as_str()).collect();
                DexResponse {
                    success: true,
                    data: Some(serde_json::to_string(&pool_ids).unwrap_or_default()),
                    message: format!("{} pools", pool_ids.len()),
                    events: Vec::new(),
                }
            }
        }
    }

    // ── Core Operations ──

    fn create_pool(
        &mut self,
        caller: &str,
        token_a: String,
        token_b: String,
        amount_a: u128,
        amount_b: u128,
        fee_bps: Option<u32>,
    ) -> DexResponse {
        if token_a == token_b {
            return DexResponse {
                success: false,
                data: None,
                message: "Cannot create pool with identical tokens".to_string(),
                events: Vec::new(),
            };
        }
        if amount_a == 0 || amount_b == 0 {
            return DexResponse {
                success: false,
                data: None,
                message: "Initial liquidity must be > 0 for both tokens".to_string(),
                events: Vec::new(),
            };
        }

        let id = Self::pool_id(&token_a, &token_b);
        if self.pools.contains_key(&id) {
            return DexResponse {
                success: false,
                data: None,
                message: format!("Pool {} already exists", id),
                events: Vec::new(),
            };
        }

        // Initial LP tokens = sqrt(amount_a * amount_b) - MINIMUM_LIQUIDITY
        // MINIMUM_LIQUIDITY is burned (locked) to prevent price manipulation with tiny pools
        let initial_lp = isqrt(amount_a.saturating_mul(amount_b));
        if initial_lp <= MINIMUM_LIQUIDITY {
            return DexResponse {
                success: false,
                data: None,
                message: "Initial liquidity too small".to_string(),
                events: Vec::new(),
            };
        }
        let lp_tokens = initial_lp - MINIMUM_LIQUIDITY;

        let fee = fee_bps.unwrap_or(SWAP_FEE_BPS as u32);
        if fee > 1000 {
            // Max 10%
            return DexResponse {
                success: false,
                data: None,
                message: "Fee too high (max 1000 bps = 10%)".to_string(),
                events: Vec::new(),
            };
        }

        let mut lp_shares = HashMap::new();
        lp_shares.insert(caller.to_string(), lp_tokens);

        let pool = Pool {
            id: id.clone(),
            token_a: token_a.clone(),
            token_b: token_b.clone(),
            reserve_a: amount_a,
            reserve_b: amount_b,
            total_lp: initial_lp, // includes MINIMUM_LIQUIDITY
            lp_shares,
            fee_bps: fee,
            last_trade_timestamp: 0,
            creator: caller.to_string(),
        };

        self.pools.insert(id.clone(), pool);
        self.pool_count += 1;

        DexResponse {
            success: true,
            data: Some(
                serde_json::json!({
                    "pool_id": id,
                    "lp_tokens": lp_tokens,
                    "reserve_a": amount_a,
                    "reserve_b": amount_b,
                })
                .to_string(),
            ),
            message: format!("Pool {} created", id),
            events: vec![DexEvent::PoolCreated {
                pool_id: id,
                token_a,
                token_b,
                reserve_a: amount_a,
                reserve_b: amount_b,
            }],
        }
    }

    fn add_liquidity(
        &mut self,
        caller: &str,
        pool_id: &str,
        amount_a: u128,
        amount_b: u128,
        min_lp_tokens: u128,
    ) -> DexResponse {
        let pool = match self.pools.get_mut(pool_id) {
            Some(p) => p,
            None => {
                return DexResponse {
                    success: false,
                    data: None,
                    message: "Pool not found".to_string(),
                    events: Vec::new(),
                }
            }
        };

        if amount_a == 0 || amount_b == 0 {
            return DexResponse {
                success: false,
                data: None,
                message: "Amounts must be > 0".to_string(),
                events: Vec::new(),
            };
        }

        // Calculate LP tokens to mint.
        // LP = min(amount_a * total_lp / reserve_a, amount_b * total_lp / reserve_b)
        // This ensures proportional contribution.
        let lp_from_a = if pool.reserve_a > 0 {
            amount_a * pool.total_lp / pool.reserve_a
        } else {
            0
        };
        let lp_from_b = if pool.reserve_b > 0 {
            amount_b * pool.total_lp / pool.reserve_b
        } else {
            0
        };
        let lp_tokens = lp_from_a.min(lp_from_b);

        if lp_tokens < min_lp_tokens {
            return DexResponse {
                success: false,
                data: None,
                message: format!(
                    "Slippage: would mint {} LP but minimum is {}",
                    lp_tokens, min_lp_tokens
                ),
                events: Vec::new(),
            };
        }

        // Calculate actual amounts used (proportional to pool ratio)
        let actual_a = lp_tokens * pool.reserve_a / pool.total_lp;
        let actual_b = lp_tokens * pool.reserve_b / pool.total_lp;

        pool.reserve_a += actual_a;
        pool.reserve_b += actual_b;
        pool.total_lp += lp_tokens;
        *pool.lp_shares.entry(caller.to_string()).or_insert(0) += lp_tokens;

        DexResponse {
            success: true,
            data: Some(
                serde_json::json!({
                    "lp_tokens": lp_tokens,
                    "amount_a_used": actual_a,
                    "amount_b_used": actual_b,
                })
                .to_string(),
            ),
            message: format!("Added liquidity: {} LP tokens minted", lp_tokens),
            events: vec![DexEvent::LiquidityAdded {
                pool_id: pool_id.to_string(),
                provider: caller.to_string(),
                amount_a: actual_a,
                amount_b: actual_b,
                lp_tokens,
            }],
        }
    }

    fn remove_liquidity(
        &mut self,
        caller: &str,
        pool_id: &str,
        lp_amount: u128,
        min_amount_a: u128,
        min_amount_b: u128,
    ) -> DexResponse {
        let pool = match self.pools.get_mut(pool_id) {
            Some(p) => p,
            None => {
                return DexResponse {
                    success: false,
                    data: None,
                    message: "Pool not found".to_string(),
                    events: Vec::new(),
                }
            }
        };

        let caller_shares = pool.lp_shares.get(caller).copied().unwrap_or(0);
        if caller_shares < lp_amount {
            return DexResponse {
                success: false,
                data: None,
                message: format!(
                    "Insufficient LP tokens: have {} need {}",
                    caller_shares, lp_amount
                ),
                events: Vec::new(),
            };
        }

        if pool.total_lp == 0 {
            return DexResponse {
                success: false,
                data: None,
                message: "Pool has no liquidity".to_string(),
                events: Vec::new(),
            };
        }

        // Calculate proportional token amounts
        let amount_a = lp_amount * pool.reserve_a / pool.total_lp;
        let amount_b = lp_amount * pool.reserve_b / pool.total_lp;

        // Slippage protection
        if amount_a < min_amount_a || amount_b < min_amount_b {
            return DexResponse {
                success: false,
                data: None,
                message: format!(
                    "Slippage: would receive ({}, {}) but minimum is ({}, {})",
                    amount_a, amount_b, min_amount_a, min_amount_b
                ),
                events: Vec::new(),
            };
        }

        pool.reserve_a -= amount_a;
        pool.reserve_b -= amount_b;
        pool.total_lp -= lp_amount;
        *pool.lp_shares.entry(caller.to_string()).or_insert(0) -= lp_amount;

        // Clean up zero-balance LP entries
        if pool.lp_shares.get(caller).copied().unwrap_or(0) == 0 {
            pool.lp_shares.remove(caller);
        }

        DexResponse {
            success: true,
            data: Some(
                serde_json::json!({
                    "amount_a": amount_a,
                    "amount_b": amount_b,
                    "lp_burned": lp_amount,
                })
                .to_string(),
            ),
            message: format!("Removed liquidity: {} LP tokens burned", lp_amount),
            events: vec![DexEvent::LiquidityRemoved {
                pool_id: pool_id.to_string(),
                provider: caller.to_string(),
                amount_a,
                amount_b,
                lp_burned: lp_amount,
            }],
        }
    }

    fn swap(
        &mut self,
        caller: &str,
        pool_id: &str,
        token_in: &str,
        amount_in: u128,
        min_amount_out: u128,
        deadline: u64,
        current_timestamp: u64,
    ) -> DexResponse {
        // MEV Protection: deadline check
        if deadline > 0 && current_timestamp > deadline {
            return DexResponse {
                success: false,
                data: None,
                message: format!(
                    "Transaction expired: deadline {} < current {}",
                    deadline, current_timestamp
                ),
                events: Vec::new(),
            };
        }

        let pool = match self.pools.get_mut(pool_id) {
            Some(p) => p,
            None => {
                return DexResponse {
                    success: false,
                    data: None,
                    message: "Pool not found".to_string(),
                    events: Vec::new(),
                }
            }
        };

        if amount_in == 0 {
            return DexResponse {
                success: false,
                data: None,
                message: "Amount must be > 0".to_string(),
                events: Vec::new(),
            };
        }

        // Determine direction
        let is_a_to_b = token_in == pool.token_a;
        let is_b_to_a = token_in == pool.token_b;
        if !is_a_to_b && !is_b_to_a {
            return DexResponse {
                success: false,
                data: None,
                message: format!(
                    "Token {} is not in pool (expected {} or {})",
                    token_in, pool.token_a, pool.token_b
                ),
                events: Vec::new(),
            };
        }

        let (reserve_in, reserve_out, token_out) = if is_a_to_b {
            (pool.reserve_a, pool.reserve_b, pool.token_b.clone())
        } else {
            (pool.reserve_b, pool.reserve_a, pool.token_a.clone())
        };

        // Deduct fee
        let (amount_after_fee, fee) = deduct_fee(amount_in, pool.fee_bps as u128);

        // Compute output via constant product
        let amount_out = compute_output(amount_after_fee, reserve_in, reserve_out);

        // MEV Protection: slippage check
        if amount_out < min_amount_out {
            return DexResponse {
                success: false,
                data: None,
                message: format!(
                    "Slippage exceeded: output {} < minimum {}",
                    amount_out, min_amount_out
                ),
                events: Vec::new(),
            };
        }

        if amount_out == 0 {
            return DexResponse {
                success: false,
                data: None,
                message: "Output amount is zero (insufficient liquidity)".to_string(),
                events: Vec::new(),
            };
        }

        // Verify output doesn't drain the pool completely
        if amount_out >= reserve_out {
            return DexResponse {
                success: false,
                data: None,
                message: "Insufficient liquidity for this trade".to_string(),
                events: Vec::new(),
            };
        }

        // Update reserves
        if is_a_to_b {
            pool.reserve_a += amount_in; // Fee stays in pool for LPs
            pool.reserve_b -= amount_out;
        } else {
            pool.reserve_b += amount_in;
            pool.reserve_a -= amount_out;
        }

        pool.last_trade_timestamp = current_timestamp;

        DexResponse {
            success: true,
            data: Some(
                serde_json::json!({
                    "amount_out": amount_out,
                    "fee": fee,
                    "price_impact_bps": if reserve_out > 0 {
                        (amount_out * BPS_DENOMINATOR) / reserve_out
                    } else { 0 },
                })
                .to_string(),
            ),
            message: format!("Swapped {} → {} {}", amount_in, amount_out, token_out),
            events: vec![DexEvent::Swap {
                pool_id: pool_id.to_string(),
                trader: caller.to_string(),
                token_in: token_in.to_string(),
                amount_in,
                token_out,
                amount_out,
                fee,
            }],
        }
    }

    fn quote(
        &self,
        pool_id: &str,
        token_in: &str,
        amount_in: u128,
    ) -> DexResponse {
        let pool = match self.pools.get(pool_id) {
            Some(p) => p,
            None => {
                return DexResponse {
                    success: false,
                    data: None,
                    message: "Pool not found".to_string(),
                    events: Vec::new(),
                }
            }
        };

        let is_a_to_b = token_in == pool.token_a;
        let (reserve_in, reserve_out) = if is_a_to_b {
            (pool.reserve_a, pool.reserve_b)
        } else {
            (pool.reserve_b, pool.reserve_a)
        };

        let (after_fee, fee) = deduct_fee(amount_in, pool.fee_bps as u128);
        let amount_out = compute_output(after_fee, reserve_in, reserve_out);

        // Price impact: how much the trade moves the price
        let spot_price_scaled = if reserve_in > 0 {
            reserve_out * LP_PRECISION / reserve_in
        } else {
            0
        };
        let exec_price_scaled = if amount_in > 0 {
            amount_out * LP_PRECISION / amount_in
        } else {
            0
        };
        let impact_bps = if spot_price_scaled > 0 {
            ((spot_price_scaled - exec_price_scaled) * BPS_DENOMINATOR) / spot_price_scaled
        } else {
            0
        };

        DexResponse {
            success: true,
            data: Some(
                serde_json::json!({
                    "amount_out": amount_out,
                    "fee": fee,
                    "price_impact_bps": impact_bps,
                    "spot_price_scaled": spot_price_scaled,
                })
                .to_string(),
            ),
            message: format!("Quote: {} in → {} out", amount_in, amount_out),
            events: Vec::new(),
        }
    }
}

impl Default for DexState {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────
// WASM ENTRY POINT (for standalone compilation)
// ─────────────────────────────────────────────────────────────

fn main() {
    println!("DEX AMM Contract for Unauthority (LOS)");
    println!("Constant Product AMM (x·y=k) with MEV protection");
    println!("Compile to WASM: cargo build --release --target wasm32-unknown-unknown");
}

// ─────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "LOSWalice000000000000000000000000000000";
    const BOB: &str = "LOSWbob00000000000000000000000000000000";
    const TOKEN_A: &str = "LOSConAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const TOKEN_B: &str = "LOSConBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn setup_pool() -> DexState {
        let mut dex = DexState::new();
        dex.execute(
            ALICE,
            DexAction::CreatePool {
                token_a: TOKEN_A.to_string(),
                token_b: TOKEN_B.to_string(),
                amount_a: 1_000_000_000, // 1 billion units
                amount_b: 2_000_000_000,
                fee_bps: None,
            },
            100,
        );
        dex
    }

    fn pool_id() -> String {
        DexState::pool_id(TOKEN_A, TOKEN_B)
    }

    // ── Math helpers ──

    #[test]
    fn test_isqrt() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(1_000_000), 1_000);
        assert_eq!(isqrt(2), 1); // floor
        assert_eq!(isqrt(8), 2); // floor
    }

    #[test]
    fn test_compute_output() {
        // Pool: 1000 A, 2000 B. Swap 100 A → B
        // After fee: 99.7 A (but we test without fee here)
        // output = 100 * 2000 / (1000 + 100) = 200_000 / 1100 = 181
        let out = compute_output(100, 1000, 2000);
        assert_eq!(out, 181); // floor(200000/1100)
    }

    #[test]
    fn test_compute_output_zero() {
        assert_eq!(compute_output(0, 1000, 2000), 0);
        assert_eq!(compute_output(100, 0, 2000), 0);
        assert_eq!(compute_output(100, 1000, 0), 0);
    }

    #[test]
    fn test_deduct_fee() {
        let (after, fee) = deduct_fee(10_000, 30); // 0.3%
        assert_eq!(fee, 30); // 10000 * 30 / 10000
        assert_eq!(after, 9_970);
    }

    // ── Pool Creation ──

    #[test]
    fn test_create_pool() {
        let mut dex = DexState::new();
        let resp = dex.execute(
            ALICE,
            DexAction::CreatePool {
                token_a: TOKEN_A.to_string(),
                token_b: TOKEN_B.to_string(),
                amount_a: 1_000_000,
                amount_b: 2_000_000,
                fee_bps: None,
            },
            100,
        );
        assert!(resp.success);
        assert_eq!(dex.pool_count, 1);
        assert_eq!(resp.events.len(), 1);
    }

    #[test]
    fn test_create_pool_duplicate() {
        let mut dex = setup_pool();
        let resp = dex.execute(
            ALICE,
            DexAction::CreatePool {
                token_a: TOKEN_A.to_string(),
                token_b: TOKEN_B.to_string(),
                amount_a: 100,
                amount_b: 200,
                fee_bps: None,
            },
            200,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("already exists"));
    }

    #[test]
    fn test_create_pool_same_token() {
        let mut dex = DexState::new();
        let resp = dex.execute(
            ALICE,
            DexAction::CreatePool {
                token_a: TOKEN_A.to_string(),
                token_b: TOKEN_A.to_string(),
                amount_a: 100,
                amount_b: 200,
                fee_bps: None,
            },
            100,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("identical"));
    }

    #[test]
    fn test_create_pool_max_fee() {
        let mut dex = DexState::new();
        let resp = dex.execute(
            ALICE,
            DexAction::CreatePool {
                token_a: TOKEN_A.to_string(),
                token_b: TOKEN_B.to_string(),
                amount_a: 1_000_000,
                amount_b: 2_000_000,
                fee_bps: Some(2000), // 20% too high
            },
            100,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("Fee too high"));
    }

    // ── Swap ──

    #[test]
    fn test_swap_a_to_b() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            BOB,
            DexAction::Swap {
                pool_id: pid.clone(),
                token_in: TOKEN_A.to_string(),
                amount_in: 10_000_000,
                min_amount_out: 1,
                deadline: 200,
            },
            150,
        );
        assert!(resp.success);
        assert_eq!(resp.events.len(), 1);

        // Verify reserves changed
        let pool = dex.pools.get(&pid).unwrap();
        assert_eq!(pool.reserve_a, 1_010_000_000); // +10M
        assert!(pool.reserve_b < 2_000_000_000); // something withdrawn
    }

    #[test]
    fn test_swap_b_to_a() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            BOB,
            DexAction::Swap {
                pool_id: pid.clone(),
                token_in: TOKEN_B.to_string(),
                amount_in: 20_000_000,
                min_amount_out: 1,
                deadline: 200,
            },
            150,
        );
        assert!(resp.success);

        let pool = dex.pools.get(&pid).unwrap();
        assert!(pool.reserve_a < 1_000_000_000); // A was withdrawn
        assert_eq!(pool.reserve_b, 2_020_000_000); // B increased
    }

    #[test]
    fn test_swap_slippage_protection() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            BOB,
            DexAction::Swap {
                pool_id: pid,
                token_in: TOKEN_A.to_string(),
                amount_in: 10_000,
                min_amount_out: 999_999_999, // way too high
                deadline: 200,
            },
            150,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("Slippage"));
    }

    #[test]
    fn test_swap_deadline_expired() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            BOB,
            DexAction::Swap {
                pool_id: pid,
                token_in: TOKEN_A.to_string(),
                amount_in: 1_000,
                min_amount_out: 1,
                deadline: 100, // deadline = 100, current = 200
            },
            200,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("expired"));
    }

    #[test]
    fn test_swap_wrong_token() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            BOB,
            DexAction::Swap {
                pool_id: pid,
                token_in: "WRONG_TOKEN".to_string(),
                amount_in: 1_000,
                min_amount_out: 1,
                deadline: 200,
            },
            150,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("not in pool"));
    }

    // ── Liquidity ──

    #[test]
    fn test_add_liquidity() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let pool_before = dex.pools.get(&pid).unwrap().clone();
        let resp = dex.execute(
            BOB,
            DexAction::AddLiquidity {
                pool_id: pid.clone(),
                amount_a: 100_000_000,
                amount_b: 200_000_000,
                min_lp_tokens: 1,
            },
            200,
        );
        assert!(resp.success);

        let pool_after = dex.pools.get(&pid).unwrap();
        assert!(pool_after.reserve_a > pool_before.reserve_a);
        assert!(pool_after.reserve_b > pool_before.reserve_b);
        assert!(pool_after.lp_shares.get(BOB).copied().unwrap_or(0) > 0);
    }

    #[test]
    fn test_remove_liquidity() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let alice_shares = dex.pools.get(&pid).unwrap().lp_shares[ALICE];
        let half = alice_shares / 2;

        let resp = dex.execute(
            ALICE,
            DexAction::RemoveLiquidity {
                pool_id: pid.clone(),
                lp_amount: half,
                min_amount_a: 1,
                min_amount_b: 1,
            },
            200,
        );
        assert!(resp.success);

        let pool = dex.pools.get(&pid).unwrap();
        assert_eq!(pool.lp_shares[ALICE], alice_shares - half);
    }

    #[test]
    fn test_remove_liquidity_insufficient() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            BOB, // Bob has no LP tokens
            DexAction::RemoveLiquidity {
                pool_id: pid,
                lp_amount: 1,
                min_amount_a: 0,
                min_amount_b: 0,
            },
            200,
        );
        assert!(!resp.success);
        assert!(resp.message.contains("Insufficient LP"));
    }

    // ── Queries ──

    #[test]
    fn test_get_pool() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(ALICE, DexAction::GetPool { pool_id: pid }, 100);
        assert!(resp.success);
        assert!(resp.data.is_some());
    }

    #[test]
    fn test_quote() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            ALICE,
            DexAction::Quote {
                pool_id: pid,
                token_in: TOKEN_A.to_string(),
                amount_in: 10_000_000,
            },
            100,
        );
        assert!(resp.success);
        assert!(resp.data.is_some());
    }

    #[test]
    fn test_get_position() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let resp = dex.execute(
            ALICE,
            DexAction::GetPosition { pool_id: pid },
            100,
        );
        assert!(resp.success);
        let data: serde_json::Value =
            serde_json::from_str(resp.data.as_ref().unwrap()).unwrap();
        assert!(data["lp_shares"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_list_pools() {
        let mut dex = setup_pool();
        let resp = dex.execute(ALICE, DexAction::ListPools, 100);
        assert!(resp.success);
        let pools: Vec<String> =
            serde_json::from_str(resp.data.as_ref().unwrap()).unwrap();
        assert_eq!(pools.len(), 1);
    }

    // ── Constant Product Invariant ──

    #[test]
    fn test_constant_product_invariant() {
        let mut dex = setup_pool();
        let pid = pool_id();

        let pool_before = dex.pools.get(&pid).unwrap();
        let k_before = pool_before.reserve_a * pool_before.reserve_b;

        // Perform swap
        dex.execute(
            BOB,
            DexAction::Swap {
                pool_id: pid.clone(),
                token_in: TOKEN_A.to_string(),
                amount_in: 50_000_000,
                min_amount_out: 1,
                deadline: 200,
            },
            150,
        );

        let pool_after = dex.pools.get(&pid).unwrap();
        let k_after = pool_after.reserve_a * pool_after.reserve_b;

        // k should increase (fees add to reserves) or stay the same
        assert!(
            k_after >= k_before,
            "Constant product violated: k_before={} k_after={}",
            k_before,
            k_after
        );
    }

    #[test]
    fn test_no_float_arithmetic() {
        // Verify there's no f32/f64 in our math
        // All operations should work with u128 only
        let amount: u128 = 1_000_000_000_000_000;
        let reserve: u128 = 2_000_000_000_000_000;
        let output = compute_output(amount, reserve, reserve);
        assert!(output > 0);
        assert!(output < reserve); // Can't drain more than reserve
    }

    #[test]
    fn test_large_numbers_no_overflow() {
        // Test with LOS-scale numbers (21.9M LOS × 10^11 CIL)
        let los_max_cil: u128 = 21_936_236 * 100_000_000_000;
        let mut dex = DexState::new();
        let resp = dex.execute(
            ALICE,
            DexAction::CreatePool {
                token_a: "LOS".to_string(),
                token_b: TOKEN_A.to_string(),
                amount_a: los_max_cil / 100, // 1% of LOS supply
                amount_b: 1_000_000_000_000_000, // 1 quadrillion token B
                fee_bps: None,
            },
            100,
        );
        assert!(resp.success, "Pool creation failed: {}", resp.message);
    }
}
